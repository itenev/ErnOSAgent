// Ern-OS — Research engine for PhD-level autonomous learning
//! Handles arXiv paper ingestion, literature review synthesis,
//! hypothesis generation, and adversarial thesis defense via self-play.
//! All inference uses `provider.chat_sync()` — no streaming, no shared locks.

use crate::provider::{Message, Provider};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ─── Data Model ────────────────────────────────────────────────────

/// A PhD-level research project progressing through defined phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchProject {
    pub id: String,
    pub title: String,
    pub domain: String,
    pub phase: ResearchPhase,
    pub papers: Vec<PaperEntry>,
    pub hypotheses: Vec<Hypothesis>,
    pub defense_log: Vec<DefenseRound>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Ordered phases of a PhD research project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchPhase {
    LiteratureSurvey,
    HypothesisGeneration,
    Experimentation,
    PaperWriting,
    Defense,
    Complete,
}

/// An ingested academic paper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperEntry {
    pub arxiv_id: String,
    pub title: String,
    pub abstract_text: String,
    pub authors: Vec<String>,
    pub key_findings: Vec<String>,
    pub embedding_id: Option<String>,
}

/// A generated hypothesis with evaluation scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub statement: String,
    pub evidence_for: Vec<String>,
    pub evidence_against: Vec<String>,
    pub novelty_score: f32,
    pub testability_score: f32,
    pub status: HypothesisStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HypothesisStatus {
    Proposed,
    Testing,
    Confirmed,
    Rejected,
}

/// A single round of adversarial thesis defense.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefenseRound {
    pub attack_prompt: String,
    pub defense_response: String,
    pub attack_quality: f32,
    pub defense_quality: f32,
    pub weak_points: Vec<String>,
}

// ─── Project Lifecycle ─────────────────────────────────────────────

impl ResearchProject {
    pub fn new(title: &str, domain: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            domain: domain.to_string(),
            phase: ResearchPhase::LiteratureSurvey,
            papers: Vec::new(),
            hypotheses: Vec::new(),
            defense_log: Vec::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Attempt to advance to the next phase. Returns Err if gate criteria not met.
    pub fn try_advance(&mut self) -> Result<ResearchPhase> {
        let next = match self.phase {
            ResearchPhase::LiteratureSurvey => {
                if self.papers.len() < 10 {
                    anyhow::bail!(
                        "Literature survey requires ≥10 papers (have {})",
                        self.papers.len()
                    );
                }
                ResearchPhase::HypothesisGeneration
            }
            ResearchPhase::HypothesisGeneration => {
                let scored = self.hypotheses.iter()
                    .filter(|h| h.novelty_score > 0.0 || h.testability_score > 0.0)
                    .count();
                if scored < 3 {
                    anyhow::bail!(
                        "Hypothesis generation requires ≥3 scored hypotheses (have {})",
                        scored
                    );
                }
                ResearchPhase::Experimentation
            }
            ResearchPhase::Experimentation => ResearchPhase::PaperWriting,
            ResearchPhase::PaperWriting => {
                if self.defense_log.is_empty() {
                    ResearchPhase::Defense
                } else {
                    anyhow::bail!("Already in defense phase");
                }
            }
            ResearchPhase::Defense => {
                let avg_quality = self.avg_defense_quality();
                if self.defense_log.len() < 3 || avg_quality < 0.7 {
                    anyhow::bail!(
                        "Defense requires ≥3 rounds with avg quality > 0.7 (have {} rounds, avg {:.2})",
                        self.defense_log.len(), avg_quality
                    );
                }
                ResearchPhase::Complete
            }
            ResearchPhase::Complete => anyhow::bail!("Project already complete"),
        };
        tracing::info!(
            module = "research", fn_name = "try_advance",
            from = ?self.phase, to = ?next,
            "Research phase advanced"
        );
        self.phase = next;
        Ok(next)
    }

    /// Average defense quality across all rounds.
    pub fn avg_defense_quality(&self) -> f32 {
        if self.defense_log.is_empty() { return 0.0; }
        let sum: f32 = self.defense_log.iter().map(|d| d.defense_quality).sum();
        sum / self.defense_log.len() as f32
    }
}

// ─── Paper Ingestion ───────────────────────────────────────────────

/// Parse paper metadata from raw fetched text using the inference provider.
/// Extracts title, abstract, authors, and key findings.
pub async fn extract_paper_metadata(
    provider: &dyn Provider,
    arxiv_id: &str,
    raw_text: &str,
) -> Result<PaperEntry> {
    let prompt = format!(
        "Extract metadata from this academic paper text. Return ONLY a JSON object:\n\
         {{\"title\": \"...\", \"abstract\": \"...\", \"authors\": [\"...\"], \"key_findings\": [\"...\"]}}\n\n\
         Paper text (first 4000 chars):\n{}",
        &raw_text[..raw_text.len().min(4000)]
    );
    let messages = vec![
        Message::text("system", "You are an academic paper metadata extractor. Return only valid JSON."),
        Message::text("user", &prompt),
    ];
    let response = provider.chat_sync(&messages, None).await
        .with_context(|| format!("Failed to extract metadata for {}", arxiv_id))?;

    parse_paper_response(arxiv_id, &response)
}

/// Parse the JSON response from the metadata extraction prompt.
fn parse_paper_response(arxiv_id: &str, response: &str) -> Result<PaperEntry> {
    // Try to extract JSON from the response (may be wrapped in markdown)
    let json_str = extract_json_block(response);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .with_context(|| format!("Invalid JSON in paper metadata for {}", arxiv_id))?;

    Ok(PaperEntry {
        arxiv_id: arxiv_id.to_string(),
        title: parsed["title"].as_str().unwrap_or("Unknown").to_string(),
        abstract_text: parsed["abstract"].as_str().unwrap_or("").to_string(),
        authors: parsed["authors"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        key_findings: parsed["key_findings"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        embedding_id: None,
    })
}

// ─── Hypothesis Generation ─────────────────────────────────────────

/// Generate research hypotheses from a literature review context.
pub async fn generate_hypotheses(
    provider: &dyn Provider,
    project: &ResearchProject,
    max_hypotheses: usize,
) -> Result<Vec<Hypothesis>> {
    let paper_context: String = project.papers.iter()
        .take(20)
        .map(|p| format!("- {}: {}", p.title, p.abstract_text.chars().take(200).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Given these papers in the domain \"{}\":\n{}\n\n\
         Identify {max_hypotheses} novel research hypotheses. For each, provide:\n\
         {{\"hypotheses\": [{{\"statement\": \"...\", \"evidence_for\": [\"...\"], \
         \"evidence_against\": [\"...\"], \"novelty\": 0.0-1.0, \"testability\": 0.0-1.0}}]}}",
        project.domain, paper_context
    );
    let messages = vec![
        Message::text("system", "You are a research advisor. Generate novel, testable hypotheses. Return only valid JSON."),
        Message::text("user", &prompt),
    ];
    let response = provider.chat_sync(&messages, None).await
        .context("Failed to generate hypotheses")?;

    parse_hypotheses_response(&response, max_hypotheses)
}

/// Parse the JSON response into Hypothesis structs.
fn parse_hypotheses_response(response: &str, max: usize) -> Result<Vec<Hypothesis>> {
    let json_str = extract_json_block(response);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .context("Invalid JSON in hypothesis response")?;

    let hypotheses = parsed["hypotheses"].as_array()
        .context("Missing 'hypotheses' array in response")?;

    Ok(hypotheses.iter().take(max).map(|h| {
        Hypothesis {
            statement: h["statement"].as_str().unwrap_or("").to_string(),
            evidence_for: json_str_array(&h["evidence_for"]),
            evidence_against: json_str_array(&h["evidence_against"]),
            novelty_score: h["novelty"].as_f64().unwrap_or(0.0) as f32,
            testability_score: h["testability"].as_f64().unwrap_or(0.0) as f32,
            status: HypothesisStatus::Proposed,
        }
    }).collect())
}

// ─── Adversarial Defense ───────────────────────────────────────────

/// Run one round of adversarial thesis defense via self-play.
/// Uses two separate inference calls: attacker (devil's advocate) and defender.
pub async fn run_defense_round(
    provider: &dyn Provider,
    project: &ResearchProject,
    thesis_statement: &str,
) -> Result<DefenseRound> {
    // Generate attack
    let attack_prompt = format!(
        "You are a harsh thesis examiner reviewing this research:\n\
         Domain: {}\nThesis: {}\n\n\
         Ask ONE probing question that challenges the methodology, \
         questions a key assumption, or identifies a logical weakness.\n\
         Be specific and adversarial.",
        project.domain, thesis_statement
    );
    let attack_messages = vec![
        Message::text("system", "You are a critical academic examiner. Challenge the thesis rigorously."),
        Message::text("user", &attack_prompt),
    ];
    let attack = provider.chat_sync(&attack_messages, None).await
        .context("Failed to generate defense attack")?;

    // Generate defense
    let paper_evidence: String = project.papers.iter()
        .take(5)
        .map(|p| format!("- {} ({})", p.title, p.key_findings.join("; ")))
        .collect::<Vec<_>>()
        .join("\n");

    let defense_prompt = format!(
        "You are defending your thesis: {}\n\n\
         An examiner asks: {}\n\n\
         Available evidence from your literature review:\n{}\n\n\
         Defend your position with specific evidence. Be rigorous and precise.",
        thesis_statement, attack, paper_evidence
    );
    let defense_messages = vec![
        Message::text("system", "You are a PhD candidate defending your thesis. Cite evidence precisely."),
        Message::text("user", &defense_prompt),
    ];
    let defense = provider.chat_sync(&defense_messages, None).await
        .context("Failed to generate defense response")?;

    // Score the exchange
    let (attack_quality, defense_quality, weak_points) =
        score_defense_round(provider, &attack, &defense).await?;

    Ok(DefenseRound {
        attack_prompt: attack,
        defense_response: defense,
        attack_quality,
        defense_quality,
        weak_points,
    })
}

/// Score a defense round using a separate grading inference call.
async fn score_defense_round(
    provider: &dyn Provider,
    attack: &str,
    defense: &str,
) -> Result<(f32, f32, Vec<String>)> {
    let prompt = format!(
        "Grade this thesis defense exchange:\n\n\
         ATTACK: {}\n\nDEFENSE: {}\n\n\
         Return JSON: {{\"attack_quality\": 0.0-1.0, \"defense_quality\": 0.0-1.0, \
         \"weak_points\": [\"...\"]}}",
        &attack[..attack.len().min(1000)],
        &defense[..defense.len().min(1000)]
    );
    let messages = vec![
        Message::text("system", "You are an impartial academic grader. Return only valid JSON."),
        Message::text("user", &prompt),
    ];
    let response = provider.chat_sync(&messages, None).await
        .context("Failed to score defense round")?;

    let json_str = extract_json_block(&response);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .unwrap_or_else(|_| serde_json::json!({"attack_quality": 0.5, "defense_quality": 0.5, "weak_points": []}));

    Ok((
        parsed["attack_quality"].as_f64().unwrap_or(0.5) as f32,
        parsed["defense_quality"].as_f64().unwrap_or(0.5) as f32,
        json_str_array(&parsed["weak_points"]),
    ))
}

// ─── Persistence ───────────────────────────────────────────────────

/// Save a research project to disk.
pub fn save_project(project: &ResearchProject, dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", project.id));
    std::fs::write(&path, serde_json::to_string_pretty(project)?)?;
    tracing::info!(
        module = "research", project = %project.title,
        phase = ?project.phase, "Research project saved"
    );
    Ok(())
}

/// Load a research project from disk.
pub fn load_project(dir: &std::path::Path, id: &str) -> Result<ResearchProject> {
    let path = dir.join(format!("{}.json", id));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Research project '{}' not found", id))?;
    serde_json::from_str(&content).context("Failed to parse research project")
}

/// List all research projects in a directory.
pub fn list_projects(dir: &std::path::Path) -> Vec<ResearchProject> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten()
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
        .filter_map(|e| {
            std::fs::read_to_string(e.path()).ok()
                .and_then(|c| serde_json::from_str(&c).ok())
        })
        .collect()
}

// ─── Helpers ───────────────────────────────────────────────────────

/// Extract a JSON block from a response that may contain markdown fencing.
fn extract_json_block(text: &str) -> String {
    // Try ```json ... ``` first
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    // Try ``` ... ```
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            let block = after[..end].trim();
            if block.starts_with('{') || block.starts_with('[') {
                return block.to_string();
            }
        }
    }
    // Try bare JSON
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return text[start..=end].to_string();
        }
    }
    text.trim().to_string()
}

/// Extract a Vec<String> from a JSON array value.
fn json_str_array(val: &serde_json::Value) -> Vec<String> {
    val.as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "research_tests.rs"]
mod tests;
