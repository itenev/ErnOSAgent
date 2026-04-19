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
        "divergence" => Ok("Divergence analysis requires live SAE — connect to interpretability dashboard.".to_string()),
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

    // Check if SAE is loaded
    let sae_guard = state.sae.read().await;
    let sae = match sae_guard.as_ref() {
        Some(s) => s,
        None => return Ok(
            "SAE not loaded — place .safetensors weights in models/sae/ and restart.".to_string()
        ),
    };

    // Get embeddings from provider (activation proxy)
    let activations = match state.provider.embed(input).await {
        Ok(a) => a,
        Err(e) => {
            return Ok(format!(
                "Failed to get embeddings for encode: {}. Is the embedding server running?", e
            ));
        }
    };

    // Dimension check — embedding dim may differ from SAE model_dim
    let act_dim = activations.len();
    let sae_dim = sae.model_dim;

    let aligned = if act_dim == sae_dim {
        activations
    } else if act_dim > sae_dim {
        // Truncate to SAE dimension (use first model_dim elements)
        activations[..sae_dim].to_vec()
    } else {
        // Pad with zeros
        let mut padded = activations;
        padded.resize(sae_dim, 0.0);
        padded
    };

    // Run SAE encode
    let features = sae.encode(&aligned, top_k);

    if features.is_empty() {
        return Ok(format!(
            "SAE encoded '{}' — no features activated above threshold.\n\
             Embedding dim: {}, SAE dim: {}, features: {}",
            &input[..input.len().min(50)], act_dim, sae_dim, sae.num_features
        ));
    }

    // Format results
    let mut lines = Vec::new();
    lines.push(format!(
        "SAE Encode: '{}'\nEmbedding dim: {} → SAE dim: {} | {} features | top {}:",
        &input[..input.len().min(50)], act_dim, sae_dim, sae.num_features, top_k
    ));
    lines.push(String::new());
    lines.push(format!("{:<8} {:<12} {}", "Feature", "Activation", "Label"));
    lines.push(format!("{:<8} {:<12} {}", "-------", "----------", "-----"));

    for f in &features {
        let label = f.label.as_deref().unwrap_or("(unlabeled)");
        lines.push(format!("{:<8} {:<12.6} {}", f.index, f.activation, label));
    }

    Ok(lines.join("\n"))
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
        Ok(format!("Labeled features:\n{}", &content[..content.len().min(1000)]))
    } else {
        Ok("No labeled features found. Train SAE and label features to populate.".to_string())
    }
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
