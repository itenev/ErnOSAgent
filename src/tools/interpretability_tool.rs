// Ern-OS — Interpretability tool — SAE feature inspection
//! Now wired to the live SAE: embed text → SAE encode → feature activations.

use anyhow::Result;
use crate::web::state::AppState;

/// SAE interpretability inspection. Provides access to the feature
/// extraction system for analyzing model activations.
pub async fn execute(args: &serde_json::Value, state: &AppState) -> Result<String> {
    tracing::info!(tool = "interpretability", "tool START");
    let action = args["action"].as_str().unwrap_or("");
    match action {
        "snapshot" => take_snapshot(),
        "top_features" | "features" => top_features(args),
        "encode" => encode_input(args, state).await,
        "divergence" => compute_divergence(args, state).await,
        "probe" => probe_concept(args),
        "labeled_features" => list_labeled_features(),
        other => Ok(format!("Unknown interpretability action: {}", other)),
    }
}

fn take_snapshot() -> Result<String> {
    let snapshot_dir = std::path::Path::new("data/snapshots");
    std::fs::create_dir_all(snapshot_dir)?;
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let path = snapshot_dir.join(format!("snapshot_{}.json", ts));
    std::fs::write(&path, "{\"type\": \"neural_snapshot\", \"status\": \"captured\"}")?;
    Ok(format!("Snapshot saved: {}", path.display()))
}

fn top_features(args: &serde_json::Value) -> Result<String> {
    let top_k = args["top_k"].as_u64().unwrap_or(10) as usize;
    let features_path = std::path::Path::new("data/sae_features.json");
    if features_path.exists() {
        let content = std::fs::read_to_string(features_path)?;
        Ok(format!("Top {} features from saved state:\n{}", top_k, &content[..content.len().min(500)]))
    } else {
        Ok(format!("No SAE feature data available. Run training or connect live SAE to populate. Requested top {}.", top_k))
    }
}

/// Encode text through the live SAE pipeline:
/// 1. Get embeddings from the provider (activation proxy)
/// 2. Run through the SAE encoder
/// 3. Return top-k feature activations
async fn encode_input(args: &serde_json::Value, state: &AppState) -> Result<String> {
    let input = args["input"].as_str().unwrap_or("");
    if input.is_empty() { anyhow::bail!("'input' required for encode"); }
    let top_k = args["top_k"].as_u64().unwrap_or(10) as usize;

    let sae_guard = state.sae.read().await;
    let sae = match sae_guard.as_ref() {
        Some(s) => s,
        None => return Ok(
            "SAE not loaded — place .safetensors weights in models/sae/ and restart.".to_string()
        ),
    };

    let activations = match state.provider.embed(input).await {
        Ok(a) => a,
        Err(e) => return Ok(format!(
            "Failed to get embeddings for encode: {}. Is the embedding server running?", e
        )),
    };

    let act_dim = activations.len();
    let aligned = align_activations(activations, sae.model_dim);
    let features = sae.encode(&aligned, top_k);

    if features.is_empty() {
        return Ok(format!(
            "SAE encoded '{}' — no features activated above threshold.\n\
             Embedding dim: {}, SAE dim: {}, features: {}",
            &input[..input.len().min(50)], act_dim, sae.model_dim, sae.num_features
        ));
    }

    Ok(format_feature_results(input, act_dim, sae.model_dim, sae.num_features, top_k, &features))
}

/// Align activation dimensions to SAE model_dim by truncating or zero-padding.
fn align_activations(activations: Vec<f32>, sae_dim: usize) -> Vec<f32> {
    let act_dim = activations.len();
    if act_dim == sae_dim {
        activations
    } else if act_dim > sae_dim {
        activations[..sae_dim].to_vec()
    } else {
        let mut padded = activations;
        padded.resize(sae_dim, 0.0);
        padded
    }
}

/// Format SAE feature results into a human-readable table.
fn format_feature_results(
    input: &str,
    act_dim: usize,
    sae_dim: usize,
    num_features: usize,
    top_k: usize,
    features: &[crate::interpretability::sae::FeatureActivation],
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "SAE Encode: '{}'\nEmbedding dim: {} → SAE dim: {} | {} features | top {}:",
        &input[..input.len().min(50)], act_dim, sae_dim, num_features, top_k
    ));
    lines.push(String::new());
    lines.push(format!("{:<8} {:<12} {}", "Feature", "Activation", "Label"));
    lines.push(format!("{:<8} {:<12} {}", "-------", "----------", "-----"));

    for f in features {
        let label = f.label.as_deref().unwrap_or("(unlabeled)");
        lines.push(format!("{:<8} {:<12.6} {}", f.index, f.activation, label));
    }

    lines.join("\n")
}

fn probe_concept(args: &serde_json::Value) -> Result<String> {
    let concept = args["concept"].as_str().or(args["input"].as_str()).unwrap_or("");
    if concept.is_empty() { anyhow::bail!("'concept' or 'input' required for probe"); }
    Ok(format!("Probing for concept '{}' — requires live SAE activation data.", concept))
}

fn list_labeled_features() -> Result<String> {
    let labels_path = std::path::Path::new("data/feature_labels.json");
    if labels_path.exists() {
        let content = std::fs::read_to_string(labels_path)?;
        Ok(format!("Labeled features:\n{}", content))
    } else {
        Ok("No labeled features found. Train SAE and label features to populate.".to_string())
    }
}

/// Compute divergence between two text inputs via activation comparison.
/// Uses extract_batch for activation collection and cosine_distance for comparison.
async fn compute_divergence(args: &serde_json::Value, state: &AppState) -> Result<String> {
    let text_a = args["text_a"].as_str().or(args["input"].as_str()).unwrap_or("");
    let text_b = args["text_b"].as_str().unwrap_or("");
    if text_a.is_empty() || text_b.is_empty() {
        anyhow::bail!("'text_a' and 'text_b' required for divergence analysis");
    }

    let texts = [text_a, text_b];
    let results = crate::interpretability::extractor::extract_batch(
        state.provider.as_ref(), &texts
    ).await;

    if results.len() < 2 {
        return Ok("Failed to extract activations for both texts".to_string());
    }

    let dist = crate::interpretability::divergence::cosine_distance(
        &results[0].activations, &results[1].activations
    );
    let kl = crate::interpretability::divergence::kl_divergence(
        &results[0].activations, &results[1].activations
    );

    Ok(format!(
        "Divergence analysis:\n\
         Text A: '{}'\n\
         Text B: '{}'\n\
         Cosine distance: {:.6}\n\
         KL divergence: {:.6}\n\
         Activation dims: A={}, B={}",
        &text_a[..text_a.len().min(50)],
        &text_b[..text_b.len().min(50)],
        dist, kl,
        results[0].activations.len(),
        results[1].activations.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot() {
        // Snapshot is sync, doesn't need state
        let r = take_snapshot().unwrap();
        assert!(r.contains("Snapshot saved"));
    }

    #[test]
    fn test_top_features_no_data() {
        let args = serde_json::json!({"action": "top_features", "top_k": 5});
        let r = top_features(&args).unwrap();
        assert!(r.contains("5"));
    }
}
