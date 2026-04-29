// Ern-OS — Student loop tests

use super::*;
use crate::learning::curriculum::{InteractionType, SceneType};
use crate::learning::verification::VerificationResult;

// ─── StudentSession ────────────────────────────────────────────────

#[test]
fn test_session_new() {
    let s = StudentSession::new();
    assert_eq!(s.scenes_processed, 0);
    assert_eq!(s.average_score(), 0.0);
    assert!(s.pending_golden.is_empty());
    assert!(s.pending_rejections.is_empty());
    assert!(s.pending_quarantine.is_empty());
}

#[test]
fn test_session_average_score() {
    let mut s = StudentSession::new();
    s.scenes_processed = 3;
    s.total_score = 2.7;
    assert!((s.average_score() - 0.9).abs() < 0.001);
}

// ─── build_student_prompt ──────────────────────────────────────────

#[test]
fn test_student_prompt_primary() {
    let p = build_student_prompt(EducationLevel::Primary);
    assert!(p.contains("primary"));
}

#[test]
fn test_student_prompt_doctoral() {
    let p = build_student_prompt(EducationLevel::Doctoral);
    assert!(p.contains("PhD") || p.contains("researcher"));
}

#[test]
fn test_student_prompt_all_levels() {
    // Every level must produce a non-empty prompt
    for level in &[
        EducationLevel::Primary,
        EducationLevel::Secondary,
        EducationLevel::Undergraduate,
        EducationLevel::Masters,
        EducationLevel::Doctoral,
    ] {
        let p = build_student_prompt(*level);
        assert!(!p.is_empty(), "Empty prompt for {:?}", level);
    }
}

// ─── build_scene_prompt ────────────────────────────────────────────

fn make_scene(interaction: InteractionType, content: &str) -> Scene {
    Scene {
        scene_type: SceneType::Quiz,
        content: content.to_string(),
        interaction,
        expected_output: None,
        difficulty: 0.5,
        time_estimate_secs: 60,
    }
}

#[test]
fn test_scene_prompt_answer_question() {
    let s = make_scene(InteractionType::AnswerQuestion, "What is 2+2?");
    let p = build_scene_prompt(&s);
    assert!(p.contains("Answer"));
    assert!(p.contains("2+2"));
}

#[test]
fn test_scene_prompt_write_essay() {
    let s = make_scene(InteractionType::WriteEssay, "Climate change");
    let p = build_scene_prompt(&s);
    assert!(p.contains("essay"));
    assert!(p.contains("Climate change"));
}

#[test]
fn test_scene_prompt_all_interactions() {
    for interaction in &[
        InteractionType::ReadAndSummarize,
        InteractionType::AnswerQuestion,
        InteractionType::SolveExercise,
        InteractionType::WriteEssay,
        InteractionType::DebatePosition,
        InteractionType::TeachBack,
        InteractionType::CompareAndContrast,
        InteractionType::SynthesizeNovel,
        InteractionType::ConductResearch,
    ] {
        let s = make_scene(interaction.clone(), "test content");
        let p = build_scene_prompt(&s);
        assert!(!p.is_empty(), "Empty prompt for {:?}", interaction);
        assert!(p.contains("test content"));
    }
}

// ─── verify_answer ─────────────────────────────────────────────────

#[test]
fn test_verify_with_expected_output_match() {
    let scene = Scene {
        scene_type: SceneType::Quiz,
        content: "What is 2+2?".into(),
        interaction: InteractionType::AnswerQuestion,
        expected_output: Some("The answer is 4".into()),
        difficulty: 0.1,
        time_estimate_secs: 30,
    };
    let r = verify_answer("The answer is 4", &scene, "c1", "l1", 0);
    assert!(r.is_confirmed());
}

#[test]
fn test_verify_with_expected_output_wrong() {
    let scene = Scene {
        scene_type: SceneType::Quiz,
        content: "What is 2+2?".into(),
        interaction: InteractionType::AnswerQuestion,
        expected_output: Some("The answer is four".into()),
        difficulty: 0.1,
        time_estimate_secs: 30,
    };
    let r = verify_answer(
        "Quantum mechanics defines the Planck constant", &scene, "c1", "l1", 0,
    );
    assert!(r.is_contradicted());
}

#[test]
fn test_verify_quiz_no_expected() {
    let scene = Scene {
        scene_type: SceneType::Quiz,
        content: "What is gravity?".into(),
        interaction: InteractionType::AnswerQuestion,
        expected_output: None,
        difficulty: 0.3,
        time_estimate_secs: 60,
    };
    let r = verify_answer("Gravity is a force", &scene, "c1", "l1", 0);
    assert!(matches!(r, VerificationResult::Unverifiable { .. }));
}

#[test]
fn test_verify_essay_unverifiable() {
    let scene = Scene {
        scene_type: SceneType::Essay,
        content: "Discuss AI ethics".into(),
        interaction: InteractionType::WriteEssay,
        expected_output: None,
        difficulty: 0.7,
        time_estimate_secs: 300,
    };
    let r = verify_answer("AI ethics is complex", &scene, "c1", "l1", 0);
    assert!(matches!(r, VerificationResult::Unverifiable { .. }));
}

// ─── route_by_verification ─────────────────────────────────────────

#[test]
fn test_route_confirmed_to_golden() {
    let mut session = StudentSession::new();
    let scene = make_scene(InteractionType::AnswerQuestion, "test");
    let v = VerificationResult::Confirmed {
        source: "test".into(),
        evidence: "correct".into(),
        confidence: 0.95,
    };
    route_by_verification(
        &v, "my answer", &scene, "c1", "l1", 0,
        EducationLevel::Primary, &mut session,
    );
    assert_eq!(session.pending_golden.len(), 1);
    assert!(session.pending_rejections.is_empty());
    assert!(session.pending_quarantine.is_empty());
}

#[test]
fn test_route_contradicted_to_rejection() {
    let mut session = StudentSession::new();
    let mut scene = make_scene(InteractionType::AnswerQuestion, "test");
    scene.expected_output = Some("correct answer".into());
    let v = VerificationResult::Contradicted {
        source: "test".into(),
        correct_answer: "correct answer".into(),
        evidence: "wrong".into(),
    };
    route_by_verification(
        &v, "wrong answer", &scene, "c1", "l1", 0,
        EducationLevel::Primary, &mut session,
    );
    assert!(session.pending_golden.is_empty());
    assert_eq!(session.pending_rejections.len(), 1);
    assert_eq!(session.pending_rejections[0].chosen, "correct answer");
    assert_eq!(session.pending_rejections[0].rejected, "wrong answer");
}

#[test]
fn test_route_unverifiable_to_quarantine() {
    let mut session = StudentSession::new();
    let scene = make_scene(InteractionType::WriteEssay, "test");
    let v = VerificationResult::Unverifiable { reason: "test".into() };
    route_by_verification(
        &v, "my essay", &scene, "c1", "l1", 0,
        EducationLevel::Undergraduate, &mut session,
    );
    assert!(session.pending_golden.is_empty());
    assert!(session.pending_rejections.is_empty());
    assert_eq!(session.pending_quarantine.len(), 1);
}

#[test]
fn test_route_contradicted_empty_correct_uses_expected() {
    let mut session = StudentSession::new();
    let mut scene = make_scene(InteractionType::AnswerQuestion, "test");
    scene.expected_output = Some("expected answer".into());
    let v = VerificationResult::Contradicted {
        source: "test".into(),
        correct_answer: String::new(),
        evidence: "wrong".into(),
    };
    route_by_verification(
        &v, "wrong answer", &scene, "c1", "l1", 0,
        EducationLevel::Primary, &mut session,
    );
    assert_eq!(session.pending_rejections[0].chosen, "expected answer");
}
