// Ern-OS — Curriculum data model and persistence
//! Structured course → lesson → scene hierarchy for the AI schooling pipeline.
//! Supports OpenMAIC ZIP import and custom JSONL ingestion.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Education Level ───────────────────────────────────────────────

/// Education level — governs learning mode, scene types, and EWC strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EducationLevel {
    Primary,
    Secondary,
    Undergraduate,
    Masters,
    Doctoral,
}

impl EducationLevel {
    /// EWC lambda for forgetting protection — increases with level.
    pub fn ewc_lambda(&self) -> f32 {
        match self {
            Self::Primary => 0.1,
            Self::Secondary => 1.0,
            Self::Undergraduate => 2.0,
            Self::Masters => 5.0,
            Self::Doctoral => 10.0,
        }
    }

    /// Minimum quiz score to pass this level.
    pub fn pass_threshold(&self) -> f32 {
        match self {
            Self::Primary => 0.70,
            Self::Secondary => 0.75,
            Self::Undergraduate => 0.80,
            Self::Masters => 0.85,
            Self::Doctoral => 0.90,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Undergraduate => "undergraduate",
            Self::Masters => "masters",
            Self::Doctoral => "doctoral",
        }
    }
}

// ─── Subject ───────────────────────────────────────────────────────

/// Subject classification — maps to adapter directory naming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    Mathematics,
    Physics,
    Chemistry,
    Biology,
    ComputerScience,
    Philosophy,
    Literature,
    History,
    Economics,
    Engineering,
    Medicine,
    Law,
    Custom(String),
}

impl Subject {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Mathematics => "mathematics",
            Self::Physics => "physics",
            Self::Chemistry => "chemistry",
            Self::Biology => "biology",
            Self::ComputerScience => "computer_science",
            Self::Philosophy => "philosophy",
            Self::Literature => "literature",
            Self::History => "history",
            Self::Economics => "economics",
            Self::Engineering => "engineering",
            Self::Medicine => "medicine",
            Self::Law => "law",
            Self::Custom(s) => s,
        }
    }
}

// ─── Scene Types ───────────────────────────────────────────────────

/// Scene type — what kind of teaching unit this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneType {
    Lecture,
    Quiz,
    Exercise,
    Essay,
    CaseStudy,
    Discussion,
    Simulation,
    Project,
    LiteratureReview,
    MethodologyDesign,
    CriticalAnalysis,
    HypothesisGeneration,
    ExperimentDesign,
    PaperWriting,
    PeerReview,
    ThesisDefense,
}

/// How the student engages with the scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionType {
    ReadAndSummarize,
    AnswerQuestion,
    SolveExercise,
    WriteEssay,
    DebatePosition,
    TeachBack,
    CompareAndContrast,
    SynthesizeNovel,
    ConductResearch,
}

// ─── Core Data Structures ──────────────────────────────────────────

/// A single teaching unit within a lesson.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub scene_type: SceneType,
    pub content: String,
    pub interaction: InteractionType,
    pub expected_output: Option<String>,
    pub difficulty: f32,
    pub time_estimate_secs: u64,
}

/// A single lesson within a course.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub title: String,
    pub order: usize,
    pub scenes: Vec<Scene>,
    pub objectives: Vec<String>,
    pub prerequisites: Vec<String>,
}

/// Where the curriculum came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CurriculumSource {
    OpenMAIC { url: String },
    OSSU { github_url: String },
    ArxivPapers { query: String, paper_ids: Vec<String> },
    CustomJsonl { path: String },
}

/// What must be demonstrated to complete a course.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionCriteria {
    pub min_lessons_completed: usize,
    pub min_quiz_score: f32,
    pub min_essay_score: f32,
    pub requires_original_work: bool,
    pub requires_defense: bool,
}

/// A complete course.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Course {
    pub id: String,
    pub title: String,
    pub description: String,
    pub level: EducationLevel,
    pub subject: Subject,
    pub lessons: Vec<Lesson>,
    pub prerequisites: Vec<String>,
    pub completion_criteria: CompletionCriteria,
    pub source: CurriculumSource,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ─── Progress Tracking ─────────────────────────────────────────────

/// Tracks student progress through a course.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseProgress {
    pub course_id: String,
    pub completed_lessons: Vec<String>,
    pub current_lesson_id: Option<String>,
    pub current_scene_index: usize,
    pub quiz_scores: Vec<f32>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

impl CourseProgress {
    pub fn new(course_id: &str) -> Self {
        let now = chrono::Utc::now();
        Self {
            course_id: course_id.to_string(),
            completed_lessons: Vec::new(),
            current_lesson_id: None,
            current_scene_index: 0,
            quiz_scores: Vec::new(),
            started_at: now,
            last_activity: now,
        }
    }

    /// Average quiz score across all completed quizzes.
    pub fn average_quiz_score(&self) -> f32 {
        if self.quiz_scores.is_empty() {
            return 0.0;
        }
        self.quiz_scores.iter().sum::<f32>() / self.quiz_scores.len() as f32
    }

    /// Fraction of lessons completed (0.0 — 1.0).
    pub fn completion_ratio(&self, total_lessons: usize) -> f32 {
        if total_lessons == 0 {
            return 0.0;
        }
        self.completed_lessons.len() as f32 / total_lessons as f32
    }
}

// ─── Curriculum Store ──────────────────────────────────────────────

/// Persistent store for courses and progress.
pub struct CurriculumStore {
    courses: Vec<Course>,
    progress: Vec<CourseProgress>,
    dir: PathBuf,
}

impl CurriculumStore {
    /// Open or create a curriculum store at the given directory.
    pub fn open(dir: &Path) -> Result<Self> {
        tracing::info!(module = "curriculum", fn_name = "open", "curriculum::open called");
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create curriculum dir: {}", dir.display()))?;

        let courses: Vec<Course> = load_json_or_default(&dir.join("courses.json"))?;
        let progress: Vec<CourseProgress> = load_json_or_default(&dir.join("progress.json"))?;

        tracing::info!(courses = courses.len(), progress = progress.len(), "Curriculum loaded");
        Ok(Self { courses, progress, dir: dir.to_path_buf() })
    }

    /// Add a course to the store.
    pub fn add_course(&mut self, course: Course) -> Result<()> {
        tracing::info!(
            module = "curriculum", fn_name = "add_course",
            course_id = %course.id, title = %course.title,
            "Adding course"
        );
        if self.courses.iter().any(|c| c.id == course.id) {
            anyhow::bail!("Course '{}' already exists", course.id);
        }
        self.courses.push(course);
        self.persist_courses()
    }

    /// Remove a course by ID.
    pub fn remove_course(&mut self, id: &str) -> Result<()> {
        tracing::info!(module = "curriculum", fn_name = "remove_course", "Removing course {}", id);
        let before = self.courses.len();
        self.courses.retain(|c| c.id != id);
        if self.courses.len() == before {
            anyhow::bail!("Course '{}' not found", id);
        }
        self.progress.retain(|p| p.course_id != id);
        self.persist_courses()?;
        self.persist_progress()
    }

    /// Get a course by ID.
    pub fn get_course(&self, id: &str) -> Option<&Course> {
        self.courses.iter().find(|c| c.id == id)
    }

    /// List all courses.
    pub fn courses(&self) -> &[Course] { &self.courses }

    /// Get or create progress for a course.
    pub fn get_or_create_progress(&mut self, course_id: &str) -> Result<&mut CourseProgress> {
        if !self.progress.iter().any(|p| p.course_id == course_id) {
            self.progress.push(CourseProgress::new(course_id));
            self.persist_progress()?;
        }
        Ok(self.progress.iter_mut().find(|p| p.course_id == course_id).unwrap())
    }

    /// Get progress for a course (read-only).
    pub fn get_progress(&self, course_id: &str) -> Option<&CourseProgress> {
        self.progress.iter().find(|p| p.course_id == course_id)
    }

    /// Mark a lesson as completed and update progress.
    pub fn complete_lesson(
        &mut self, course_id: &str, lesson_id: &str, quiz_score: Option<f32>,
    ) -> Result<()> {
        tracing::info!(
            module = "curriculum", fn_name = "complete_lesson",
            course_id, lesson_id, "Lesson completed"
        );
        let progress = self.get_or_create_progress(course_id)?;
        if !progress.completed_lessons.contains(&lesson_id.to_string()) {
            progress.completed_lessons.push(lesson_id.to_string());
        }
        if let Some(score) = quiz_score {
            progress.quiz_scores.push(score);
        }
        progress.last_activity = chrono::Utc::now();
        progress.current_lesson_id = None;
        progress.current_scene_index = 0;
        self.persist_progress()
    }

    /// Save current position within a lesson (for resume after preemption).
    pub fn save_position(
        &mut self, course_id: &str, lesson_id: &str, scene_index: usize,
    ) -> Result<()> {
        let progress = self.get_or_create_progress(course_id)?;
        progress.current_lesson_id = Some(lesson_id.to_string());
        progress.current_scene_index = scene_index;
        progress.last_activity = chrono::Utc::now();
        self.persist_progress()
    }

    /// Find the next incomplete lesson in a course.
    pub fn next_lesson<'a>(&self, course: &'a Course) -> Option<&'a Lesson> {
        let progress = self.get_progress(&course.id);
        let completed: Vec<&str> = progress
            .map(|p| p.completed_lessons.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        course.lessons.iter().find(|l| !completed.contains(&l.id.as_str()))
    }

    /// Check if a course meets its completion criteria.
    pub fn is_course_complete(&self, course: &Course) -> bool {
        let progress = match self.get_progress(&course.id) {
            Some(p) => p,
            None => return false,
        };
        let ratio = progress.completion_ratio(course.lessons.len());
        let min_ratio = course.completion_criteria.min_lessons_completed as f32
            / course.lessons.len().max(1) as f32;
        let score_ok = progress.average_quiz_score() >= course.completion_criteria.min_quiz_score;
        ratio >= min_ratio && score_ok
    }

    /// Get the average quiz score for a course, if progress exists.
    pub fn course_average_score(&self, course: &Course) -> Option<f32> {
        let progress = self.get_progress(&course.id)?;
        if progress.quiz_scores.is_empty() { return None; }
        Some(progress.average_quiz_score())
    }

    pub fn course_count(&self) -> usize { self.courses.len() }

    fn persist_courses(&self) -> Result<()> {
        persist_json(&self.dir.join("courses.json"), &self.courses)
    }

    fn persist_progress(&self) -> Result<()> {
        persist_json(&self.dir.join("progress.json"), &self.progress)
    }
}

// ─── Helpers ───────────────────────────────────────────────────────

fn load_json_or_default<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))
    } else {
        Ok(T::default())
    }
}

fn persist_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(data)?)
        .with_context(|| format!("Failed to write {}", path.display()))
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "curriculum_tests.rs"]
mod tests;
