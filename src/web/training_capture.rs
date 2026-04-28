// Ern-OS — Training signal capture from Observer verdicts
//! Captures approved/rejected responses as training data.
//! Approved responses → GoldenBuffer (SFT training).
//! Rejected+retried responses → RejectionBuffer (DPO preference pairs).

use crate::learning::{TrainingSample, TrainingMethod};
use crate::web::state::AppState;

/// Capture an approved response as a golden training sample. Fire-and-forget.
pub fn capture_approved(state: &AppState, query: &str, response: &str, score: f32) {
    capture_approved_with_flags(state, query, response, score, &[]);
}

/// Capture an approved response with positive deviation flags.
/// Each positive flag boosts the quality score by 0.02 (capped at 1.0),
/// reinforcing exemplary behaviors in the training pipeline.
pub fn capture_approved_with_flags(
    state: &AppState, query: &str, response: &str, score: f32, positive_flags: &[String],
) {
    let golden = state.golden_buffer.clone();
    let query = query.to_string();
    let response = response.to_string();

    // Boost quality score for responses with positive deviation flags
    let boost = positive_flags.len() as f32 * 0.02;
    let final_score = (score + boost).min(1.0);

    if !positive_flags.is_empty() {
        tracing::info!(
            flags = ?positive_flags,
            base_score = score,
            boosted_score = final_score,
            "Positive deviation detected — boosting training quality"
        );
    }

    tokio::spawn(async move {
        let sample = TrainingSample {
            id: uuid::Uuid::new_v4().to_string(),
            input: query,
            output: response,
            method: TrainingMethod::Sft,
            quality_score: final_score,
            timestamp: chrono::Utc::now(),
        };
        let mut buf = golden.write().await;
        if let Err(e) = buf.add(sample) {
            tracing::warn!(error = %e, "Failed to capture golden training sample");
        } else {
            tracing::debug!(count = buf.count(), "Golden sample captured");
        }
    });
}

/// Capture a rejection as a DPO preference pair. Fire-and-forget.
/// `rejected` is the bad response, `chosen` is the approved retry.
pub fn capture_rejection(
    state: &AppState, query: &str, rejected: &str, chosen: &str, reason: &str,
) {
    let rejection = state.rejection_buffer.clone();
    let query = query.to_string();
    let rejected = rejected.to_string();
    let chosen = chosen.to_string();
    let reason = reason.to_string();

    tokio::spawn(async move {
        let mut buf = rejection.write().await;
        if let Err(e) = buf.add_pair(&query, &chosen, &rejected, &reason) {
            tracing::warn!(error = %e, "Failed to capture rejection pair");
        } else {
            tracing::debug!(count = buf.count(), "Rejection pair captured");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_training_sample_creation() {
        let sample = TrainingSample {
            id: "test".to_string(),
            input: "What is Rust?".to_string(),
            output: "Rust is a language.".to_string(),
            method: TrainingMethod::Sft,
            quality_score: 0.9,
            timestamp: chrono::Utc::now(),
        };
        assert_eq!(sample.method, TrainingMethod::Sft);
        assert!(sample.quality_score > 0.8);
    }
}
