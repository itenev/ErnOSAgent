// Ern-OS — Ground Truth Verification Gate
//! Prevents the hallucination feedback loop by verifying student answers
//! against external sources before they enter the Golden Buffer.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Verification Result ───────────────────────────────────────────

/// Outcome of verifying a student answer against external truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationResult {
    /// Answer confirmed by external source.
    Confirmed {
        source: String,
        evidence: String,
        confidence: f32,
    },
    /// Answer contradicted by external source.
    Contradicted {
        source: String,
        correct_answer: String,
        evidence: String,
    },
    /// Cannot verify — no external source found.
    Unverifiable { reason: String },
}

impl VerificationResult {
    pub fn is_confirmed(&self) -> bool { matches!(self, Self::Confirmed { .. }) }
    pub fn is_contradicted(&self) -> bool { matches!(self, Self::Contradicted { .. }) }

    /// Confidence score (0.0 for contradicted/unverifiable, actual value for confirmed).
    pub fn confidence(&self) -> f32 {
        match self {
            Self::Confirmed { confidence, .. } => *confidence,
            Self::Contradicted { .. } => 0.0,
            Self::Unverifiable { .. } => 0.0,
        }
    }
}

// ─── Quarantine Buffer ─────────────────────────────────────────────

/// An answer that could not be verified — held for review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineEntry {
    pub id: String,
    pub course_id: String,
    pub lesson_id: String,
    pub scene_index: usize,
    pub student_answer: String,
    pub teacher_grade: f32,
    pub verification_attempts: Vec<VerificationResult>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Buffer for answers that could not be externally verified.
pub struct QuarantineBuffer {
    entries: Vec<QuarantineEntry>,
    file_path: Option<PathBuf>,
}

impl QuarantineBuffer {
    pub fn new() -> Self {
        Self { entries: Vec::new(), file_path: None }
    }

    pub fn open(path: &Path) -> Result<Self> {
        tracing::info!(module = "quarantine", fn_name = "open", "quarantine::open called");
        let mut buf = Self { entries: Vec::new(), file_path: Some(path.to_path_buf()) };
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read quarantine: {}", path.display()))?;
            buf.entries = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse quarantine: {}", path.display()))?;
        }
        tracing::info!(entries = buf.entries.len(), "Quarantine loaded");
        Ok(buf)
    }

    pub fn add(&mut self, entry: QuarantineEntry) -> Result<()> {
        tracing::info!(
            module = "quarantine", fn_name = "add",
            course = %entry.course_id, lesson = %entry.lesson_id,
            "Quarantining unverified answer"
        );
        self.entries.push(entry);
        self.persist()
    }

    /// Approve an entry — returns it for promotion to Golden Buffer.
    pub fn approve(&mut self, id: &str) -> Result<QuarantineEntry> {
        tracing::info!(module = "quarantine", fn_name = "approve", "Approving {}", id);
        let pos = self.entries.iter().position(|e| e.id == id)
            .with_context(|| format!("Quarantine entry '{}' not found", id))?;
        let entry = self.entries.remove(pos);
        self.persist()?;
        Ok(entry)
    }

    /// Reject an entry — removes it permanently.
    pub fn reject(&mut self, id: &str) -> Result<()> {
        tracing::info!(module = "quarantine", fn_name = "reject", "Rejecting {}", id);
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == before {
            anyhow::bail!("Quarantine entry '{}' not found", id);
        }
        self.persist()
    }

    pub fn entries(&self) -> &[QuarantineEntry] { &self.entries }
    pub fn count(&self) -> usize { self.entries.len() }

    fn persist(&self) -> Result<()> {
        if let Some(ref path) = self.file_path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, serde_json::to_string_pretty(&self.entries)?)
                .with_context(|| format!("Failed to write quarantine: {}", path.display()))?;
        }
        Ok(())
    }
}

// ─── Verification Logic ────────────────────────────────────────────

/// Verify against curriculum ground truth (expected_output field).
pub fn verify_against_expected(student_answer: &str, expected: &str) -> VerificationResult {
    tracing::debug!(
        module = "verification", fn_name = "verify_against_expected",
        "Comparing answer to expected output"
    );
    let similarity = text_similarity(student_answer, expected);
    if similarity >= 0.7 {
        VerificationResult::Confirmed {
            source: "curriculum".into(),
            evidence: expected.to_string(),
            confidence: similarity,
        }
    } else {
        VerificationResult::Contradicted {
            source: "curriculum".into(),
            correct_answer: expected.to_string(),
            evidence: format!(
                "Student answer diverges from expected (similarity: {:.2})",
                similarity
            ),
        }
    }
}

/// Verify against web search results.
/// Returns Confirmed/Contradicted/Unverifiable based on search content.
pub fn verify_against_search_results(
    student_answer: &str, question: &str, search_results: &str,
) -> VerificationResult {
    tracing::debug!(
        module = "verification", fn_name = "verify_against_search_results",
        "Verifying answer against web search results"
    );
    if search_results.contains("TOOL FAILURE") || search_results.is_empty() {
        return VerificationResult::Unverifiable {
            reason: "Web search unavailable".into(),
        };
    }

    let results_lower = search_results.to_lowercase();

    // Extract key claims from the student answer (sentences)
    let claims = extract_claims(student_answer);
    if claims.is_empty() {
        return VerificationResult::Unverifiable {
            reason: "Could not extract verifiable claims from answer".into(),
        };
    }

    let mut confirmed_count = 0usize;
    let mut total_claims = claims.len();
    let mut best_evidence = String::new();

    for claim in &claims {
        let claim_lower = claim.to_lowercase();
        // Check if the search results contain key terms from this claim
        let claim_words: Vec<&str> = claim_lower.split_whitespace()
            .filter(|w| w.len() > 3)
            .collect();
        if claim_words.is_empty() {
            total_claims -= 1;
            continue;
        }
        let matched = claim_words.iter()
            .filter(|w| results_lower.contains(**w))
            .count();
        let match_ratio = matched as f32 / claim_words.len() as f32;
        if match_ratio >= 0.5 {
            confirmed_count += 1;
            if best_evidence.is_empty() {
                best_evidence = find_relevant_snippet(&results_lower, &claim_lower);
            }
        }
    }

    if total_claims == 0 {
        return VerificationResult::Unverifiable {
            reason: "No substantive claims to verify".into(),
        };
    }

    let verification_ratio = confirmed_count as f32 / total_claims as f32;
    let confidence = compute_confidence(1, verification_ratio, 0.7);

    if verification_ratio >= 0.5 {
        VerificationResult::Confirmed {
            source: "web_search".into(),
            evidence: if best_evidence.is_empty() {
                "Multiple claims corroborated by search results".into()
            } else {
                best_evidence
            },
            confidence,
        }
    } else if verification_ratio <= 0.2 {
        VerificationResult::Contradicted {
            source: "web_search".into(),
            correct_answer: String::new(),
            evidence: format!(
                "Only {:.0}% of claims corroborated by search results for: {}",
                verification_ratio * 100.0, question
            ),
        }
    } else {
        VerificationResult::Unverifiable {
            reason: format!(
                "Inconclusive: {:.0}% of claims partially corroborated",
                verification_ratio * 100.0
            ),
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────

/// Compute verification confidence from source rank, text overlap, and authority.
pub fn compute_confidence(source_rank: usize, text_overlap: f32, source_authority: f32) -> f32 {
    let rank_score = 1.0 / (source_rank as f32 + 1.0);
    (rank_score * 0.3 + text_overlap * 0.4 + source_authority * 0.3).clamp(0.0, 1.0)
}

/// Simple text similarity — proportion of shared significant words.
fn text_similarity(a: &str, b: &str) -> f32 {
    let words_a: std::collections::HashSet<String> = a.to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect();
    let words_b: std::collections::HashSet<String> = b.to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect();
    if words_a.is_empty() && words_b.is_empty() {
        return 1.0;
    }
    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }
    let intersection = words_a.intersection(&words_b).count() as f32;
    let union = words_a.union(&words_b).count() as f32;
    intersection / union
}

/// Extract verifiable claims from text (split into sentences).
fn extract_claims(text: &str) -> Vec<String> {
    text.split(|c: char| c == '.' || c == '!' || c == '?' || c == '\n')
        .map(|s| s.trim().to_string())
        .filter(|s| s.split_whitespace().count() >= 3)
        .collect()
}

/// Find the most relevant snippet from search results for a given claim.
fn find_relevant_snippet(results: &str, claim: &str) -> String {
    let claim_words: Vec<&str> = claim.split_whitespace()
        .filter(|w| w.len() > 3)
        .take(5)
        .collect();
    // Find the line in results with the most matching words
    results.lines()
        .filter(|line| line.len() > 20)
        .max_by_key(|line| {
            claim_words.iter().filter(|w| line.contains(**w)).count()
        })
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "verification_tests.rs"]
mod tests;
