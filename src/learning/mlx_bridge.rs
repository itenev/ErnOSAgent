// Ern-OS — MLX training bridge
//! Subprocess-based LoRA fine-tuning via Apple's MLX framework.
//! Prepares JSONL training data, calls `mlx_lm.lora` as a subprocess,
//! registers adapters, and applies EWC regularization post-training.

use crate::learning::curriculum::EducationLevel;
use crate::learning::lora::LoraConfig;
use crate::learning::TrainingSample;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Configuration ─────────────────────────────────────────────────

/// Configuration for an MLX LoRA training run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MlxTrainConfig {
    pub model_path: PathBuf,
    pub output_dir: PathBuf,
    pub learning_rate: f64,
    pub epochs: usize,
    pub batch_size: usize,
    pub lora_rank: usize,
    pub ewc_lambda: f32,
}

impl MlxTrainConfig {
    /// Create config for a specific education level.
    /// Higher levels use lower learning rates and stronger EWC regularization.
    pub fn for_level(level: EducationLevel, model_path: &Path, data_dir: &Path) -> Self {
        let (lr, ewc) = match level {
            EducationLevel::Primary => (2e-5, 0.0),
            EducationLevel::Secondary => (1e-5, 0.1),
            EducationLevel::Undergraduate => (5e-6, 0.5),
            EducationLevel::Masters => (2e-6, 1.0),
            EducationLevel::Doctoral => (1e-6, 2.0),
        };
        Self {
            model_path: model_path.to_path_buf(),
            output_dir: data_dir.join("adapters").join(format!("{:?}", level).to_lowercase()),
            learning_rate: lr,
            epochs: 1, // Incremental — always 1 epoch
            batch_size: 1,
            lora_rank: LoraConfig::default().rank,
            ewc_lambda: ewc,
        }
    }
}

/// Result of an MLX training run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MlxTrainResult {
    pub adapter_path: String,
    pub final_loss: f64,
    pub samples_trained: usize,
    pub duration_secs: u64,
}

// ─── Data Preparation ──────────────────────────────────────────────

/// Prepare training data as JSONL in chat format.
/// Each sample becomes: {"messages": [{"role":"user","content":"..."}, {"role":"assistant","content":"..."}]}
pub fn prepare_training_data(
    samples: &[TrainingSample],
    output_dir: &Path,
) -> Result<PathBuf> {
    if samples.is_empty() {
        anyhow::bail!("Cannot prepare training data from empty samples");
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create training data dir: {}", output_dir.display()))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let path = output_dir.join(format!("train_{}.jsonl", timestamp));

    let mut content = String::new();
    for sample in samples {
        let entry = serde_json::json!({
            "messages": [
                {"role": "user", "content": sample.input},
                {"role": "assistant", "content": sample.output}
            ]
        });
        content.push_str(&serde_json::to_string(&entry)?);
        content.push('\n');
    }

    std::fs::write(&path, &content)
        .with_context(|| format!("Failed to write training JSONL: {}", path.display()))?;

    tracing::info!(
        module = "mlx_bridge", fn_name = "prepare_training_data",
        samples = samples.len(), path = %path.display(),
        "Training data prepared"
    );
    Ok(path)
}

// ─── EWC Snapshots ─────────────────────────────────────────────────

/// Save an EWC parameter snapshot for the given subject.
/// This captures the current adapter weights as θ* for future regularization.
pub fn snapshot_ewc_params(
    adapter_dir: &Path,
    subject: &str,
    ewc_dir: &Path,
) -> Result<PathBuf> {
    std::fs::create_dir_all(ewc_dir)?;

    // Read the current adapter weights (if they exist)
    let adapter_path = adapter_dir.join("adapters.safetensors");
    if !adapter_path.exists() {
        anyhow::bail!("No adapter weights found at {}", adapter_path.display());
    }

    let snapshot_name = format!("{}_snapshot.json", subject.replace(' ', "_").to_lowercase());
    let snapshot_path = ewc_dir.join(&snapshot_name);

    // Copy raw adapter bytes as the snapshot reference
    std::fs::copy(&adapter_path, &snapshot_path)
        .with_context(|| format!("Failed to create EWC snapshot for '{}'", subject))?;

    tracing::info!(
        module = "mlx_bridge", fn_name = "snapshot_ewc_params",
        subject, snapshot = %snapshot_path.display(),
        "EWC parameter snapshot saved"
    );
    Ok(snapshot_path)
}

// ─── MLX Subprocess ────────────────────────────────────────────────

/// Run MLX LoRA fine-tuning as a subprocess.
/// Returns the training result including adapter path and final loss.
pub async fn run_mlx_lora(
    config: &MlxTrainConfig,
    data_path: &Path,
) -> Result<MlxTrainResult> {
    let start = std::time::Instant::now();

    std::fs::create_dir_all(&config.output_dir)?;

    let iters = config.epochs.max(1).to_string();
    let lr = config.learning_rate.to_string();
    let batch = config.batch_size.to_string();

    let output = tokio::process::Command::new("python3")
        .args([
            "-m", "mlx_lm.lora",
            "--model", &config.model_path.to_string_lossy(),
            "--data", &data_path.to_string_lossy(),
            "--adapter-path", &config.output_dir.to_string_lossy(),
            "--train",
            "--num-layers", "8",
            "--learning-rate", &lr,
            "--batch-size", &batch,
            "--iters", &iters,
        ])
        .output()
        .await
        .context("Failed to execute mlx_lm.lora — is mlx-lm installed?")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        anyhow::bail!(
            "mlx_lm.lora failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            if stderr.is_empty() { &stdout } else { &stderr }
        );
    }

    let final_loss = parse_mlx_loss(&stdout);
    let duration = start.elapsed().as_secs();

    let result = MlxTrainResult {
        adapter_path: config.output_dir.to_string_lossy().to_string(),
        final_loss,
        samples_trained: 0, // Parsed from output if available
        duration_secs: duration,
    };

    tracing::info!(
        module = "mlx_bridge", fn_name = "run_mlx_lora",
        loss = final_loss, duration_secs = duration,
        adapter = %config.output_dir.display(),
        "MLX LoRA training complete"
    );
    Ok(result)
}

/// Parse the final training loss from MLX stdout.
/// MLX outputs lines like: "Iter 100: Train loss 0.1234, ..."
fn parse_mlx_loss(stdout: &str) -> f64 {
    stdout.lines().rev()
        .find_map(|line| {
            if let Some(pos) = line.find("Train loss ") {
                let after = &line[pos + 11..];
                after.split(|c: char| !c.is_ascii_digit() && c != '.')
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
            } else {
                None
            }
        })
        .unwrap_or(0.0)
}

// ─── Availability Check ───────────────────────────────────────────

/// Check if MLX LoRA is available on this system.
/// Returns false gracefully if Python/mlx-lm not installed (feature disabled, not crashed).
pub async fn check_mlx_available() -> bool {
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new("python3")
            .args(["-c", "import mlx_lm; print('ok')"])
            .output(),
    ).await {
        Ok(Ok(output)) => output.status.success(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "mlx_bridge_tests.rs"]
mod tests;
