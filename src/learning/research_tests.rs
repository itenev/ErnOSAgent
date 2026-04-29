use super::*;

// ─── ResearchProject Tests ─────────────────────────────────────────

#[test]
fn test_project_new() {
    let p = ResearchProject::new("Test Thesis", "machine_learning");
    assert_eq!(p.title, "Test Thesis");
    assert_eq!(p.domain, "machine_learning");
    assert_eq!(p.phase, ResearchPhase::LiteratureSurvey);
    assert!(p.papers.is_empty());
    assert!(p.hypotheses.is_empty());
    assert!(p.defense_log.is_empty());
}

#[test]
fn test_advance_survey_needs_papers() {
    let mut p = ResearchProject::new("Test", "ml");
    // Not enough papers
    assert!(p.try_advance().is_err());
    let err = p.try_advance().unwrap_err().to_string();
    assert!(err.contains("≥10 papers"));
}

#[test]
fn test_advance_survey_to_hypothesis() {
    let mut p = ResearchProject::new("Test", "ml");
    for i in 0..10 {
        p.papers.push(PaperEntry {
            arxiv_id: format!("2024.{:05}", i),
            title: format!("Paper {}", i),
            abstract_text: "Abstract text".into(),
            authors: vec!["Author".into()],
            key_findings: vec!["Finding".into()],
            embedding_id: None,
        });
    }
    let next = p.try_advance().unwrap();
    assert_eq!(next, ResearchPhase::HypothesisGeneration);
    assert_eq!(p.phase, ResearchPhase::HypothesisGeneration);
}

#[test]
fn test_advance_hypothesis_needs_scored() {
    let mut p = ResearchProject::new("Test", "ml");
    // Skip to hypothesis phase
    p.phase = ResearchPhase::HypothesisGeneration;
    // Add unscored hypotheses
    p.hypotheses.push(Hypothesis {
        statement: "H1".into(),
        evidence_for: vec![], evidence_against: vec![],
        novelty_score: 0.0, testability_score: 0.0,
        status: HypothesisStatus::Proposed,
    });
    assert!(p.try_advance().is_err());
}

#[test]
fn test_advance_hypothesis_to_experimentation() {
    let mut p = ResearchProject::new("Test", "ml");
    p.phase = ResearchPhase::HypothesisGeneration;
    for i in 0..3 {
        p.hypotheses.push(Hypothesis {
            statement: format!("H{}", i),
            evidence_for: vec!["evidence".into()],
            evidence_against: vec![],
            novelty_score: 0.8,
            testability_score: 0.7,
            status: HypothesisStatus::Proposed,
        });
    }
    let next = p.try_advance().unwrap();
    assert_eq!(next, ResearchPhase::Experimentation);
}

#[test]
fn test_advance_defense_needs_rounds() {
    let mut p = ResearchProject::new("Test", "ml");
    p.phase = ResearchPhase::Defense;
    // Not enough rounds
    assert!(p.try_advance().is_err());
    let err = p.try_advance().unwrap_err().to_string();
    assert!(err.contains("≥3 rounds"));
}

#[test]
fn test_advance_defense_needs_quality() {
    let mut p = ResearchProject::new("Test", "ml");
    p.phase = ResearchPhase::Defense;
    for _ in 0..3 {
        p.defense_log.push(DefenseRound {
            attack_prompt: "Q".into(),
            defense_response: "A".into(),
            attack_quality: 0.8,
            defense_quality: 0.5, // Below 0.7 threshold
            weak_points: vec![],
        });
    }
    assert!(p.try_advance().is_err());
}

#[test]
fn test_advance_defense_to_complete() {
    let mut p = ResearchProject::new("Test", "ml");
    p.phase = ResearchPhase::Defense;
    for _ in 0..3 {
        p.defense_log.push(DefenseRound {
            attack_prompt: "Q".into(),
            defense_response: "A".into(),
            attack_quality: 0.8,
            defense_quality: 0.85,
            weak_points: vec![],
        });
    }
    let next = p.try_advance().unwrap();
    assert_eq!(next, ResearchPhase::Complete);
}

#[test]
fn test_advance_complete_errors() {
    let mut p = ResearchProject::new("Test", "ml");
    p.phase = ResearchPhase::Complete;
    assert!(p.try_advance().is_err());
}

#[test]
fn test_avg_defense_quality_empty() {
    let p = ResearchProject::new("Test", "ml");
    assert_eq!(p.avg_defense_quality(), 0.0);
}

#[test]
fn test_avg_defense_quality() {
    let mut p = ResearchProject::new("Test", "ml");
    p.defense_log.push(DefenseRound {
        attack_prompt: "Q".into(), defense_response: "A".into(),
        attack_quality: 0.8, defense_quality: 0.9, weak_points: vec![],
    });
    p.defense_log.push(DefenseRound {
        attack_prompt: "Q".into(), defense_response: "A".into(),
        attack_quality: 0.7, defense_quality: 0.7, weak_points: vec![],
    });
    assert!((p.avg_defense_quality() - 0.8).abs() < 0.01);
}

// ─── Paper Parsing Tests ───────────────────────────────────────────

#[test]
fn test_parse_paper_response_valid() {
    let json = r#"{"title": "Deep Learning", "abstract": "We present...", "authors": ["Smith"], "key_findings": ["Finding 1"]}"#;
    let paper = parse_paper_response("2024.00001", json).unwrap();
    assert_eq!(paper.title, "Deep Learning");
    assert_eq!(paper.authors, vec!["Smith"]);
    assert_eq!(paper.key_findings, vec!["Finding 1"]);
    assert_eq!(paper.arxiv_id, "2024.00001");
}

#[test]
fn test_parse_paper_response_markdown_wrapped() {
    let response = "Here's the data:\n```json\n{\"title\": \"ML Paper\", \"abstract\": \"...\", \"authors\": [], \"key_findings\": []}\n```\nDone.";
    let paper = parse_paper_response("2024.00002", response).unwrap();
    assert_eq!(paper.title, "ML Paper");
}

#[test]
fn test_parse_paper_response_invalid() {
    assert!(parse_paper_response("x", "not json").is_err());
}

// ─── Hypothesis Parsing Tests ──────────────────────────────────────

#[test]
fn test_parse_hypotheses_response() {
    let json = r#"{"hypotheses": [
        {"statement": "H1", "evidence_for": ["e1"], "evidence_against": [], "novelty": 0.9, "testability": 0.8},
        {"statement": "H2", "evidence_for": [], "evidence_against": ["e2"], "novelty": 0.7, "testability": 0.6}
    ]}"#;
    let result = parse_hypotheses_response(json, 5).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].statement, "H1");
    assert!((result[0].novelty_score - 0.9).abs() < 0.01);
    assert_eq!(result[1].evidence_against, vec!["e2"]);
    assert_eq!(result[0].status, HypothesisStatus::Proposed);
}

#[test]
fn test_parse_hypotheses_response_limited() {
    let json = r#"{"hypotheses": [
        {"statement": "H1", "novelty": 0.9, "testability": 0.8},
        {"statement": "H2", "novelty": 0.7, "testability": 0.6},
        {"statement": "H3", "novelty": 0.5, "testability": 0.4}
    ]}"#;
    let result = parse_hypotheses_response(json, 2).unwrap();
    assert_eq!(result.len(), 2); // Capped at max
}

// ─── JSON Extraction Tests ─────────────────────────────────────────

#[test]
fn test_extract_json_block_bare() {
    let text = r#"{"key": "value"}"#;
    assert_eq!(extract_json_block(text), r#"{"key": "value"}"#);
}

#[test]
fn test_extract_json_block_fenced() {
    let text = "Here:\n```json\n{\"key\": \"value\"}\n```\nDone";
    assert_eq!(extract_json_block(text), r#"{"key": "value"}"#);
}

#[test]
fn test_extract_json_block_embedded() {
    let text = "Analysis: {\"verdict\": true} end.";
    assert_eq!(extract_json_block(text), r#"{"verdict": true}"#);
}

// ─── Persistence Tests ─────────────────────────────────────────────

#[test]
fn test_save_and_load_project() {
    let tmp = tempfile::TempDir::new().unwrap();
    let p = ResearchProject::new("Persistence Test", "cs");
    save_project(&p, tmp.path()).unwrap();
    let loaded = load_project(tmp.path(), &p.id).unwrap();
    assert_eq!(loaded.title, "Persistence Test");
    assert_eq!(loaded.phase, ResearchPhase::LiteratureSurvey);
}

#[test]
fn test_list_projects_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert!(list_projects(tmp.path()).is_empty());
}

#[test]
fn test_list_projects() {
    let tmp = tempfile::TempDir::new().unwrap();
    save_project(&ResearchProject::new("P1", "ml"), tmp.path()).unwrap();
    save_project(&ResearchProject::new("P2", "nlp"), tmp.path()).unwrap();
    let projects = list_projects(tmp.path());
    assert_eq!(projects.len(), 2);
}

#[test]
fn test_load_missing_project() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert!(load_project(tmp.path(), "nonexistent").is_err());
}

// ─── Helper Tests ──────────────────────────────────────────────────

#[test]
fn test_json_str_array() {
    let val = serde_json::json!(["a", "b", "c"]);
    assert_eq!(json_str_array(&val), vec!["a", "b", "c"]);
}

#[test]
fn test_json_str_array_empty() {
    let val = serde_json::json!(null);
    assert!(json_str_array(&val).is_empty());
}
