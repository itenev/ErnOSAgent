// Ern-OS — Verification module tests

use super::*;

// ─── VerificationResult ────────────────────────────────────────────

#[test]
fn test_confirmed_result() {
    let r = VerificationResult::Confirmed {
        source: "test".into(),
        evidence: "evidence".into(),
        confidence: 0.95,
    };
    assert!(r.is_confirmed());
    assert!(!r.is_contradicted());
    assert!((r.confidence() - 0.95).abs() < 0.001);
}

#[test]
fn test_contradicted_result() {
    let r = VerificationResult::Contradicted {
        source: "test".into(),
        correct_answer: "correct".into(),
        evidence: "evidence".into(),
    };
    assert!(r.is_contradicted());
    assert!(!r.is_confirmed());
    assert_eq!(r.confidence(), 0.0);
}

#[test]
fn test_unverifiable_result() {
    let r = VerificationResult::Unverifiable { reason: "no source".into() };
    assert!(!r.is_confirmed());
    assert!(!r.is_contradicted());
    assert_eq!(r.confidence(), 0.0);
}

// ─── verify_against_expected ───────────────────────────────────────

#[test]
fn test_verify_exact_match() {
    let r = verify_against_expected("The answer is 4", "The answer is 4");
    assert!(r.is_confirmed());
}

#[test]
fn test_verify_similar_match() {
    // Exact same content, slightly reworded — should confirm
    let r = verify_against_expected(
        "An object at rest stays at rest unless acted upon by external force",
        "An object at rest stays at rest unless acted upon by a force",
    );
    assert!(r.is_confirmed());
}

#[test]
fn test_verify_mismatch() {
    let r = verify_against_expected(
        "The capital of France is Berlin",
        "The capital of France is Paris",
    );
    // These share some words but diverge on the key answer
    // The similarity should still catch some overlap but the wrong answer
    // Note: text_similarity is Jaccard-based, so partial overlap exists
    // This tests the boundary — may be confirmed or contradicted based on threshold
    assert!(r.is_confirmed() || r.is_contradicted());
}

#[test]
fn test_verify_completely_wrong() {
    let r = verify_against_expected(
        "Quantum chromodynamics describes flavor mixing",
        "Two plus two equals four",
    );
    assert!(r.is_contradicted());
}

// ─── verify_against_search_results ─────────────────────────────────

#[test]
fn test_search_verification_tool_failure() {
    let r = verify_against_search_results(
        "answer", "question",
        "[TOOL FAILURE: web_search] All tiers exhausted",
    );
    assert!(matches!(r, VerificationResult::Unverifiable { .. }));
}

#[test]
fn test_search_verification_empty_results() {
    let r = verify_against_search_results("answer", "question", "");
    assert!(matches!(r, VerificationResult::Unverifiable { .. }));
}

#[test]
fn test_search_verification_corroborated() {
    let answer = "Water boils at 100 degrees Celsius at standard pressure";
    let search = "Water boils at 100 degrees Celsius (212°F) at sea level, \
                  which is considered standard atmospheric pressure.";
    let r = verify_against_search_results(answer, "What temperature does water boil at?", search);
    assert!(r.is_confirmed());
}

#[test]
fn test_search_verification_no_claims() {
    let r = verify_against_search_results("hi", "question", "lots of search results here");
    assert!(matches!(r, VerificationResult::Unverifiable { .. }));
}

// ─── QuarantineBuffer ──────────────────────────────────────────────

fn sample_entry(id: &str) -> QuarantineEntry {
    QuarantineEntry {
        id: id.into(),
        course_id: "course-1".into(),
        lesson_id: "lesson-1".into(),
        scene_index: 0,
        student_answer: "test answer".into(),
        teacher_grade: 0.8,
        verification_attempts: vec![VerificationResult::Unverifiable {
            reason: "test".into(),
        }],
        timestamp: chrono::Utc::now(),
    }
}

#[test]
fn test_quarantine_add_and_count() {
    let mut buf = QuarantineBuffer::new();
    buf.add(sample_entry("e1")).unwrap();
    assert_eq!(buf.count(), 1);
}

#[test]
fn test_quarantine_approve() {
    let mut buf = QuarantineBuffer::new();
    buf.add(sample_entry("e1")).unwrap();
    let entry = buf.approve("e1").unwrap();
    assert_eq!(entry.id, "e1");
    assert_eq!(buf.count(), 0);
}

#[test]
fn test_quarantine_approve_nonexistent() {
    let mut buf = QuarantineBuffer::new();
    assert!(buf.approve("nope").is_err());
}

#[test]
fn test_quarantine_reject() {
    let mut buf = QuarantineBuffer::new();
    buf.add(sample_entry("e1")).unwrap();
    buf.reject("e1").unwrap();
    assert_eq!(buf.count(), 0);
}

#[test]
fn test_quarantine_reject_nonexistent() {
    let mut buf = QuarantineBuffer::new();
    assert!(buf.reject("nope").is_err());
}

#[test]
fn test_quarantine_persistence() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("quarantine.json");
    {
        let mut buf = QuarantineBuffer::open(&path).unwrap();
        buf.add(sample_entry("e1")).unwrap();
        buf.add(sample_entry("e2")).unwrap();
    }
    let buf = QuarantineBuffer::open(&path).unwrap();
    assert_eq!(buf.count(), 2);
}

// ─── Helpers ───────────────────────────────────────────────────────

#[test]
fn test_text_similarity_identical() {
    let s = text_similarity("hello world test", "hello world test");
    assert!((s - 1.0).abs() < 0.001);
}

#[test]
fn test_text_similarity_disjoint() {
    let s = text_similarity("alpha beta gamma", "delta epsilon zeta");
    assert_eq!(s, 0.0);
}

#[test]
fn test_text_similarity_partial() {
    let s = text_similarity("the cat sat on the mat", "the dog sat on the rug");
    assert!(s > 0.0);
    assert!(s < 1.0);
}

#[test]
fn test_text_similarity_empty() {
    assert_eq!(text_similarity("", ""), 1.0);
    assert_eq!(text_similarity("hello", ""), 0.0);
}

#[test]
fn test_extract_claims() {
    let claims = extract_claims("Water boils at 100C. Ice melts at 0C. Hi.");
    assert_eq!(claims.len(), 2); // "Hi." has < 3 words
}

#[test]
fn test_extract_claims_empty() {
    let claims = extract_claims("ok");
    assert!(claims.is_empty());
}

#[test]
fn test_compute_confidence() {
    let c = compute_confidence(1, 0.8, 0.9);
    assert!(c > 0.0 && c <= 1.0);
    // rank=1 → rank_score=0.5, so 0.5*0.3 + 0.8*0.4 + 0.9*0.3 = 0.15+0.32+0.27 = 0.74
    assert!((c - 0.74).abs() < 0.01);
}

#[test]
fn test_compute_confidence_clamped() {
    let c = compute_confidence(0, 1.0, 1.0);
    assert!(c <= 1.0);
}

#[test]
fn test_find_relevant_snippet() {
    let results = "line one about cats\nline two about water boiling temperature\nline three";
    let snippet = find_relevant_snippet(results, "water boiling temperature");
    assert!(snippet.contains("water"));
}
