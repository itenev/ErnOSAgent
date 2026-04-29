// Ern-OS — Curriculum module tests

use super::*;

fn sample_course() -> Course {
    Course {
        id: "test-101".into(),
        title: "Test Course".into(),
        description: "A test course".into(),
        level: EducationLevel::Primary,
        subject: Subject::Mathematics,
        lessons: vec![
            Lesson {
                id: "lesson-1".into(),
                title: "Lesson 1".into(),
                order: 0,
                scenes: vec![Scene {
                    scene_type: SceneType::Quiz,
                    content: "What is 2+2?".into(),
                    interaction: InteractionType::AnswerQuestion,
                    expected_output: Some("4".into()),
                    difficulty: 0.1,
                    time_estimate_secs: 30,
                }],
                objectives: vec!["Basic arithmetic".into()],
                prerequisites: vec![],
            },
            Lesson {
                id: "lesson-2".into(),
                title: "Lesson 2".into(),
                order: 1,
                scenes: vec![],
                objectives: vec![],
                prerequisites: vec!["lesson-1".into()],
            },
        ],
        prerequisites: vec![],
        completion_criteria: CompletionCriteria {
            min_lessons_completed: 2,
            min_quiz_score: 0.7,
            min_essay_score: 0.0,
            requires_original_work: false,
            requires_defense: false,
        },
        source: CurriculumSource::CustomJsonl { path: "test.jsonl".into() },
        created_at: chrono::Utc::now(),
    }
}

#[test]
fn test_education_level_ewc() {
    assert!(EducationLevel::Doctoral.ewc_lambda() > EducationLevel::Primary.ewc_lambda());
}

#[test]
fn test_education_level_threshold() {
    assert!(EducationLevel::Doctoral.pass_threshold() > EducationLevel::Primary.pass_threshold());
}

#[test]
fn test_subject_as_str() {
    assert_eq!(Subject::ComputerScience.as_str(), "computer_science");
    assert_eq!(Subject::Custom("ai".into()).as_str(), "ai");
}

#[test]
fn test_course_progress_new() {
    let p = CourseProgress::new("test-101");
    assert_eq!(p.course_id, "test-101");
    assert!(p.completed_lessons.is_empty());
    assert_eq!(p.average_quiz_score(), 0.0);
}

#[test]
fn test_course_progress_average_score() {
    let mut p = CourseProgress::new("test-101");
    p.quiz_scores = vec![0.8, 0.9, 1.0];
    let avg = p.average_quiz_score();
    assert!((avg - 0.9).abs() < 0.001);
}

#[test]
fn test_course_progress_completion_ratio() {
    let mut p = CourseProgress::new("test-101");
    p.completed_lessons = vec!["l1".into(), "l2".into()];
    assert!((p.completion_ratio(4) - 0.5).abs() < 0.001);
    assert_eq!(p.completion_ratio(0), 0.0);
}

#[test]
fn test_store_add_and_get() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    store.add_course(sample_course()).unwrap();
    assert_eq!(store.course_count(), 1);
    assert!(store.get_course("test-101").is_some());
}

#[test]
fn test_store_duplicate_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    store.add_course(sample_course()).unwrap();
    assert!(store.add_course(sample_course()).is_err());
}

#[test]
fn test_store_remove() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    store.add_course(sample_course()).unwrap();
    store.remove_course("test-101").unwrap();
    assert_eq!(store.course_count(), 0);
}

#[test]
fn test_store_remove_nonexistent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    assert!(store.remove_course("nope").is_err());
}

#[test]
fn test_next_lesson() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    let course = sample_course();
    store.add_course(course.clone()).unwrap();

    let next = store.next_lesson(&course);
    assert_eq!(next.unwrap().id, "lesson-1");

    store.complete_lesson("test-101", "lesson-1", Some(0.9)).unwrap();
    let next = store.next_lesson(&course);
    assert_eq!(next.unwrap().id, "lesson-2");
}

#[test]
fn test_save_position_and_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    store.add_course(sample_course()).unwrap();
    store.save_position("test-101", "lesson-1", 3).unwrap();

    let progress = store.get_progress("test-101").unwrap();
    assert_eq!(progress.current_lesson_id.as_deref(), Some("lesson-1"));
    assert_eq!(progress.current_scene_index, 3);
}

#[test]
fn test_is_course_complete() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = CurriculumStore::open(tmp.path()).unwrap();
    let course = sample_course();
    store.add_course(course.clone()).unwrap();

    assert!(!store.is_course_complete(&course));

    store.complete_lesson("test-101", "lesson-1", Some(0.8)).unwrap();
    store.complete_lesson("test-101", "lesson-2", Some(0.9)).unwrap();
    assert!(store.is_course_complete(&course));
}

#[test]
fn test_persistence_roundtrip() {
    let tmp = tempfile::TempDir::new().unwrap();
    {
        let mut store = CurriculumStore::open(tmp.path()).unwrap();
        store.add_course(sample_course()).unwrap();
        store.complete_lesson("test-101", "lesson-1", Some(0.85)).unwrap();
    }
    // Reopen and verify data persisted
    let store = CurriculumStore::open(tmp.path()).unwrap();
    assert_eq!(store.course_count(), 1);
    let progress = store.get_progress("test-101").unwrap();
    assert_eq!(progress.completed_lessons.len(), 1);
    assert!((progress.quiz_scores[0] - 0.85).abs() < 0.001);
}
