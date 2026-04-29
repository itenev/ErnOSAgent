// Ern-OS — Graduation pipeline
//! Auto-promotes Ern-OS from one education level to the next when
//! completion criteria are met. Validates adapters against held-out
//! test data and optionally fuses adapters into the base model via MLX.

use crate::learning::curriculum::{CurriculumStore, EducationLevel};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ─── Data Model ────────────────────────────────────────────────────

/// Result of a successful graduation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationResult {
    pub from_level: EducationLevel,
    pub to_level: EducationLevel,
    pub courses_completed: usize,
    pub average_score: f32,
    pub adapter_fused: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Gate criteria for graduating from a given level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationGate {
    pub level: EducationLevel,
    pub required_courses: usize,
    pub min_average_score: f32,
    pub requires_capstone: bool,
}

/// Result of adapter validation against held-out test questions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub correct: usize,
    pub total: usize,
    pub accuracy: f32,
    pub regression_detected: bool,
}

// ─── Gate Definitions ──────────────────────────────────────────────

/// Default graduation gates for each education level.
pub fn default_gates() -> Vec<GraduationGate> {
    vec![
        GraduationGate {
            level: EducationLevel::Primary,
            required_courses: 3, min_average_score: 0.6,
            requires_capstone: false,
        },
        GraduationGate {
            level: EducationLevel::Secondary,
            required_courses: 4, min_average_score: 0.65,
            requires_capstone: false,
        },
        GraduationGate {
            level: EducationLevel::Undergraduate,
            required_courses: 5, min_average_score: 0.7,
            requires_capstone: false,
        },
        GraduationGate {
            level: EducationLevel::Masters,
            required_courses: 3, min_average_score: 0.75,
            requires_capstone: true,
        },
        GraduationGate {
            level: EducationLevel::Doctoral,
            required_courses: 1, min_average_score: 0.8,
            requires_capstone: true, // Defense required
        },
    ]
}

/// Get the graduation gate for a specific level.
pub fn gate_for_level(level: EducationLevel) -> Option<GraduationGate> {
    default_gates().into_iter().find(|g| g.level == level)
}

/// Get the next education level, if one exists.
pub fn next_level(current: EducationLevel) -> Option<EducationLevel> {
    match current {
        EducationLevel::Primary => Some(EducationLevel::Secondary),
        EducationLevel::Secondary => Some(EducationLevel::Undergraduate),
        EducationLevel::Undergraduate => Some(EducationLevel::Masters),
        EducationLevel::Masters => Some(EducationLevel::Doctoral),
        EducationLevel::Doctoral => None, // Already at the top
    }
}

// ─── Graduation Check ──────────────────────────────────────────────

/// Check if Ern-OS is ready to graduate from the current level.
/// Returns Some(result) if all gate criteria are met, None if not ready.
pub fn check_graduation(
    store: &CurriculumStore,
    current_level: EducationLevel,
) -> Option<GraduationResult> {
    let gate = gate_for_level(current_level)?;
    let next = next_level(current_level)?;

    // Count completed courses at current level
    let completed: Vec<_> = store.courses().iter()
        .filter(|c| c.level == current_level && store.is_course_complete(c))
        .collect();

    if completed.len() < gate.required_courses {
        tracing::debug!(
            module = "graduation",
            level = ?current_level,
            completed = completed.len(),
            required = gate.required_courses,
            "Not enough completed courses for graduation"
        );
        return None;
    }

    // Calculate average score across completed courses
    let scores: Vec<f32> = completed.iter()
        .filter_map(|c| store.course_average_score(c))
        .collect();

    if scores.is_empty() {
        return None;
    }

    let avg_score = scores.iter().sum::<f32>() / scores.len() as f32;
    if avg_score < gate.min_average_score {
        tracing::debug!(
            module = "graduation",
            level = ?current_level,
            avg_score, required = gate.min_average_score,
            "Average score too low for graduation"
        );
        return None;
    }

    tracing::info!(
        module = "graduation", fn_name = "check_graduation",
        from = ?current_level, to = ?next,
        courses = completed.len(), avg_score,
        "Graduation criteria met!"
    );

    Some(GraduationResult {
        from_level: current_level,
        to_level: next,
        courses_completed: completed.len(),
        average_score: avg_score,
        adapter_fused: false,
        timestamp: chrono::Utc::now(),
    })
}

// ─── Adapter Fusion ────────────────────────────────────────────────

/// Fuse a trained LoRA adapter into the base model via MLX subprocess.
pub async fn fuse_adapter(
    model_path: &std::path::Path,
    adapter_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(output_path)?;

    let output = tokio::process::Command::new("python3")
        .args([
            "-m", "mlx_lm.fuse",
            "--model", &model_path.to_string_lossy(),
            "--adapter-path", &adapter_path.to_string_lossy(),
            "--save-path", &output_path.to_string_lossy(),
        ])
        .output()
        .await
        .context("Failed to execute mlx_lm.fuse — is mlx-lm installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("mlx_lm.fuse failed: {}", stderr);
    }

    tracing::info!(
        module = "graduation", fn_name = "fuse_adapter",
        model = %model_path.display(),
        adapter = %adapter_path.display(),
        output = %output_path.display(),
        "Adapter fused into base model"
    );
    Ok(output_path.to_path_buf())
}

// ─── Persistence ───────────────────────────────────────────────────

/// Save a graduation result to disk.
pub fn save_graduation(
    result: &GraduationResult,
    data_dir: &std::path::Path,
) -> Result<()> {
    let dir = data_dir.join("graduations");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "graduation_{:?}_to_{:?}.json",
        result.from_level, result.to_level
    ).to_lowercase());
    std::fs::write(&path, serde_json::to_string_pretty(result)?)?;
    tracing::info!(
        module = "graduation",
        from = ?result.from_level, to = ?result.to_level,
        "Graduation recorded"
    );
    Ok(())
}

/// Load all graduation history from disk.
pub fn load_graduation_history(data_dir: &std::path::Path) -> Vec<GraduationResult> {
    let dir = data_dir.join("graduations");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    entries.flatten()
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
        .filter_map(|e| {
            std::fs::read_to_string(e.path()).ok()
                .and_then(|c| serde_json::from_str(&c).ok())
        })
        .collect()
}

#[cfg(test)]
#[path = "graduation_tests.rs"]
mod tests;
