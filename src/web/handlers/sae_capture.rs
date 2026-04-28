// Ern-OS — Live SAE activation extraction
//! Hooks into the inference pipeline to extract activations from each response,
//! run them through the SAE encoder, and feed the LiveMonitor + SnapshotStore.
//!
//! Uses a dedicated SAE embedding sidecar (same main model, --embeddings mode)
//! on sae_embed_port (default 8082) to get residual stream activations matching
//! what the SAE was trained on.

use crate::web::state::AppState;
use crate::interpretability::FeatureActivation;

/// Extract activations from a response text and update the live monitor.
/// Runs as a background task — must not block inference.
pub fn spawn_activation_capture(state: &AppState, response_text: &str) {
    let state = state.clone();
    let text = response_text.to_string();
    tokio::spawn(async move {
        if let Err(e) = capture_activations(&state, &text).await {
            tracing::debug!(error = %e, "SAE activation capture skipped");
        }
    });
}

/// Extract activations from the SAE embedding sidecar.
/// This hits the same main model running in --embeddings mode on sae_embed_port,
/// producing the same residual stream activations the SAE was trained on.
async fn extract_model_activations(state: &AppState, text: &str) -> anyhow::Result<Vec<f32>> {
    let port = state.config.llamacpp.sae_embed_port;
    let url = format!("http://localhost:{}/v1/embeddings", port);

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "input": text,
        "encoding_format": "float",
    });

    let resp = client.post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("SAE embed sidecar error {}: {}", status, body);
    }

    let parsed: serde_json::Value = resp.json().await?;

    parsed
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("embedding"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect::<Vec<f32>>()
        })
        .ok_or_else(|| anyhow::anyhow!("Missing embedding data in SAE sidecar response"))
}

async fn capture_activations(state: &AppState, text: &str) -> anyhow::Result<()> {
    // Skip if no SAE loaded
    let sae_guard = state.sae.read().await;
    let sae = match sae_guard.as_ref() {
        Some(s) => s,
        None => return Ok(()),
    };

    if text.len() < 20 {
        return Ok(());
    }

    let embed_text = if text.len() > 500 { &text[..500] } else { text };

    let activations = extract_model_activations(state, embed_text).await?;

    if activations.len() != sae.model_dim {
        tracing::warn!(
            got = activations.len(), expected = sae.model_dim,
            "SAE dim mismatch — sidecar embedding dim doesn't match trained SAE"
        );
        return Ok(());
    }

    let raw_features = sae.encode(&activations, 20);

    let labeled = crate::interpretability::features::labeled_features();
    let feature_activations: Vec<FeatureActivation> = raw_features.iter().map(|f| {
        let label_info = labeled.iter().find(|l| l.index == f.index);
        FeatureActivation {
            feature_index: f.index,
            label: label_info.map(|l| l.label.clone()).unwrap_or_else(|| format!("feature_{}", f.index)),
            activation: f.activation,
            baseline: label_info.map(|l| l.baseline_activation).unwrap_or(0.0),
            delta: f.activation - label_info.map(|l| l.baseline_activation).unwrap_or(0.0),
        }
    }).collect();

    {
        let mut monitor = state.live_monitor.write().await;
        monitor.push(feature_activations.clone());
        tracing::debug!(
            features = feature_activations.len(),
            window = monitor.window_len(),
            top = ?feature_activations.first().map(|f| (&f.label, f.activation)),
            "SAE live monitor updated"
        );
    }

    let averages = state.live_monitor.read().await.averages();
    if averages.len() >= 3 {
        let divergence: f32 = feature_activations.iter()
            .map(|fa| fa.delta.abs())
            .sum::<f32>() / feature_activations.len().max(1) as f32;

        if divergence > 0.5 {
            let top_features: Vec<(usize, f32)> = feature_activations.iter()
                .map(|fa| (fa.feature_index, fa.activation))
                .collect();
            let context_summary = if text.len() > 200 {
                format!("{}...", &text[..200])
            } else {
                text.to_string()
            };

            let mut store = state.snapshot_store.write().await;
            match store.capture(top_features, &context_summary, divergence) {
                Ok(id) => tracing::info!(id, divergence, "Neural snapshot captured"),
                Err(e) => tracing::debug!(error = %e, "Snapshot capture failed"),
            }
        }
    }

    Ok(())
}
