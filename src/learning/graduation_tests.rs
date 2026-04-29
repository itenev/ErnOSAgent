use super::*;
use crate::learning::curriculum::{
    CompletionCriteria, Course, CurriculumSource, EducationLevel, Lesson, Subject,
};

fn test_course(id: &str, level: EducationLevel) -> Course {
    Course {
        id: id.to_string(),
        title: format!("Test Course {}", id),
        description: "Test course".to_string(),
        level,
        subject: Subject::Custom("test".to_string()),
        lessons: vec![
            Lesson {
                id: format!("{}_l1", id),
                title: "Lesson 1".into(),
                order: 0,
                scenes: vec![],
                objectives: vec![],
                prerequisites: vec![],
            },
        ],
        prerequisites: vec![],
        completion_criteria: CompletionCriteria {
            min_lessons_completed: 1,
            min_quiz_score: 0.5,
            min_essay_score: 0.0,
            requires_original_work: false,
            requires_defense: false,
        },
        source: CurriculumSource::CustomJsonl { path: "test".into() },
        created_at: chrono::Utc::now(),
    }
}

#[test]
fn test_default_gates() {
    let gates = default_gates();
    assert_eq!(gates.len(), 5);
    assert!(gates.iter().any(|g| g.level == EducationLevel::Primary));
    assert!(gates.iter().any(|g| g.level == EducationLevel::Doctoral));
}

#[test]
fn test_gate_primary_lowest() {
    let gate = gate_for_level(EducationLevel::Primary).unwrap();
    assert_eq!(gate.required_courses, 3);
    assert!((gate.min_average_score - 0.6).abs() < f32::EPSILON);
    assert!(!gate.requires_capstone);
}

#[test]
fn test_gate_doctoral_highest() {
    let gate = gate_for_level(EducationLevel::Doctoral).unwrap();
    assert_eq!(gate.required_courses, 1);
    assert!((gate.min_average_score - 0.8).abs() < f32::EPSILON);
    assert!(gate.requires_capstone);
}

#[test]
fn test_gate_scaling() {
    let gates = default_gates();
    for i in 1..gates.len() {
        assert!(
            gates[i].min_average_score >= gates[i - 1].min_average_score,
            "Gate scores should increase with level"
        );
    }
}

#[test]
fn test_next_level() {
    assert_eq!(next_level(EducationLevel::Primary), Some(EducationLevel::Secondary));
    assert_eq!(next_level(EducationLevel::Secondary), Some(EducationLevel::Undergraduate));
    assert_eq!(next_level(EducationLevel::Undergraduate), Some(EducationLevel::Masters));
    assert_eq!(next_level(EducationLevel::Masters), Some(EducationLevel::Doctoral));
    assert_eq!(next_level(EducationLevel::Doctoral), None);
}

#[test]
fn test_check_graduation_not_ready() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CurriculumStore::open(tmp.path()).unwrap();
    // Empty store — no courses completed
    assert!(check_graduation(&store, EducationLevel::Primary).is_none());
}

#[test]
fn test_check_graduation_ready() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();

    // Add 3 primary courses and complete them with good scores
    for i in 0..3 {
        let course = test_course(&format!("p{}", i), EducationLevel::Primary);
        let lesson_id = course.lessons[0].id.clone();
        let course_id = course.id.clone();
        store.add_course(course).unwrap();
        store.complete_lesson(&course_id, &lesson_id, Some(0.85)).unwrap();
    }

    let result = check_graduation(&store, EducationLevel::Primary);
    assert!(result.is_some());
    let r = result.unwrap();
    assert_eq!(r.from_level, EducationLevel::Primary);
    assert_eq!(r.to_level, EducationLevel::Secondary);
    assert_eq!(r.courses_completed, 3);
    assert!(r.average_score > 0.8);
}

#[test]
fn test_check_graduation_score_too_low() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();

    // Add 3 courses but with low scores
    for i in 0..3 {
        let course = test_course(&format!("low{}", i), EducationLevel::Primary);
        let lesson_id = course.lessons[0].id.clone();
        let course_id = course.id.clone();
        store.add_course(course).unwrap();
        store.complete_lesson(&course_id, &lesson_id, Some(0.3)).unwrap();
    }

    // Score is 0.3, threshold is 0.6 — should not graduate
    assert!(check_graduation(&store, EducationLevel::Primary).is_none());
}

#[test]
fn test_validation_result() {
    let v = ValidationResult {
        correct: 8, total: 10,
        accuracy: 0.8, regression_detected: false,
    };
    assert_eq!(v.correct, 8);
    assert!(!v.regression_detected);
}

#[test]
fn test_graduation_persistence() {
    let tmp = tempfile::TempDir::new().unwrap();
    let result = GraduationResult {
        from_level: EducationLevel::Primary,
        to_level: EducationLevel::Secondary,
        courses_completed: 3,
        average_score: 0.85,
        adapter_fused: false,
        timestamp: chrono::Utc::now(),
    };
    save_graduation(&result, tmp.path()).unwrap();
    let history = load_graduation_history(tmp.path());
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].from_level, EducationLevel::Primary);
}

#[test]
fn test_graduation_history_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert!(load_graduation_history(tmp.path()).is_empty());
}
