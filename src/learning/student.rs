// Ern-OS — Student Loop Core
//! Autonomous learning engine that processes curriculum scenes,
//! generates answers, verifies them, and accumulates training data.
//! Uses local buffers with batch flushing to avoid lock contention.

use crate::learning::curriculum::{
    EducationLevel, InteractionType, Scene, SceneType,
};
use crate::learning::verification::{
    self, QuarantineEntry, VerificationResult,
};
use crate::learning::{TrainingMethod, TrainingSample};
use crate::provider::{Message, Provider};
use anyhow::{Context, Result};

// ─── Student Session (Local Buffers) ───────────────────────────────

/// Accumulated learning results — held locally, flushed between scenes.
/// This is the core anti-contention mechanism: no shared locks during inference.
pub struct StudentSession {
    pub pending_golden: Vec<TrainingSample>,
    pub pending_rejections: Vec<PendingRejection>,
    pub pending_quarantine: Vec<QuarantineEntry>,
    pub pending_lessons: Vec<(String, String, f32)>,
    pub scenes_processed: usize,
    pub total_score: f32,
}

/// A rejection pair waiting to be flushed.
pub struct PendingRejection {
    pub input: String,
    pub chosen: String,
    pub rejected: String,
    pub reason: String,
}

impl StudentSession {
    pub fn new() -> Self {
        Self {
            pending_golden: Vec::new(),
            pending_rejections: Vec::new(),
            pending_quarantine: Vec::new(),
            pending_lessons: Vec::new(),
            scenes_processed: 0,
            total_score: 0.0,
        }
    }

    /// Average score across all processed scenes.
    pub fn average_score(&self) -> f32 {
        if self.scenes_processed == 0 { return 0.0; }
        self.total_score / self.scenes_processed as f32
    }
}

// ─── Scene Processing ──────────────────────────────────────────────

/// Process a single scene — the core of the student loop.
/// Returns the verification result for this scene.
/// All results are accumulated in the session's local buffers.
pub async fn process_scene(
    provider: &dyn Provider,
    scene: &Scene,
    course_id: &str,
    lesson_id: &str,
    scene_index: usize,
    level: EducationLevel,
    session: &mut StudentSession,
) -> Result<VerificationResult> {
    tracing::info!(
        module = "student", fn_name = "process_scene",
        course_id, lesson_id, scene_index,
        scene_type = ?scene.scene_type,
        "Processing scene"
    );

    let student_answer = generate_answer(provider, scene, level).await
        .with_context(|| format!("Failed to generate answer for scene {}", scene_index))?;

    let verification = verify_answer(&student_answer, scene, course_id, lesson_id, scene_index);
    route_by_verification(
        &verification, &student_answer, scene, course_id, lesson_id,
        scene_index, level, session,
    );

    session.scenes_processed += 1;
    session.total_score += verification.confidence();
    Ok(verification)
}

// ─── Answer Generation ─────────────────────────────────────────────

/// Generate a student answer for a scene using inference.
async fn generate_answer(
    provider: &dyn Provider, scene: &Scene, level: EducationLevel,
) -> Result<String> {
    let system_prompt = build_student_prompt(level);
    let user_prompt = build_scene_prompt(scene);

    let messages = vec![
        Message::text("system", &system_prompt),
        Message::text("user", &user_prompt),
    ];

    let response = provider.chat_sync(&messages, None).await
        .with_context(|| "Student inference failed")?;

    tracing::debug!(
        module = "student", fn_name = "generate_answer",
        response_len = response.len(),
        "Student generated answer"
    );
    Ok(response)
}

/// Build the system prompt for the student based on education level.
fn build_student_prompt(level: EducationLevel) -> String {
    match level {
        EducationLevel::Primary => {
            "You are a primary school student. Answer clearly and simply. \
             Show your reasoning step by step.".into()
        }
        EducationLevel::Secondary => {
            "You are a secondary school student. Provide detailed answers \
             with logical reasoning and evidence.".into()
        }
        EducationLevel::Undergraduate => {
            "You are an undergraduate student. Analyze the topic critically, \
             cite relevant concepts, and synthesize across domains.".into()
        }
        EducationLevel::Masters => {
            "You are a masters student. Evaluate methodology, identify gaps \
             in current understanding, and propose research directions.".into()
        }
        EducationLevel::Doctoral => {
            "You are a PhD researcher. Generate original insights, design \
             experiments, and defend claims with rigorous evidence.".into()
        }
    }
}

/// Build the user prompt for a specific scene.
fn build_scene_prompt(scene: &Scene) -> String {
    let instruction = match scene.interaction {
        InteractionType::ReadAndSummarize => "Read and summarize in your own words",
        InteractionType::AnswerQuestion => "Answer the following question",
        InteractionType::SolveExercise => "Solve the following exercise, showing your work",
        InteractionType::WriteEssay => "Write a structured essay on the following topic",
        InteractionType::DebatePosition => "Argue for AND against the following position",
        InteractionType::TeachBack => "Explain this concept as if teaching a beginner",
        InteractionType::CompareAndContrast => "Compare and contrast the following",
        InteractionType::SynthesizeNovel => "Synthesize a novel perspective on the following",
        InteractionType::ConductResearch => "Conduct a research analysis of the following",
    };
    format!("{}:\n\n{}", instruction, scene.content)
}

// ─── Verification ──────────────────────────────────────────────────

/// Verify a student answer against available ground truth.
fn verify_answer(
    answer: &str, scene: &Scene,
    course_id: &str, lesson_id: &str, scene_index: usize,
) -> VerificationResult {
    // Priority 1: Curriculum expected output (strongest ground truth)
    if let Some(ref expected) = scene.expected_output {
        return verification::verify_against_expected(answer, expected);
    }

    // Priority 2: For quiz/exercise scenes without expected output, mark unverifiable
    match scene.scene_type {
        SceneType::Quiz | SceneType::Exercise => {
            tracing::warn!(
                module = "student", course_id, lesson_id, scene_index,
                "Quiz/Exercise scene has no expected_output — quarantining"
            );
            VerificationResult::Unverifiable {
                reason: "Quiz scene missing expected_output field".into(),
            }
        }
        // For open-ended scenes (essay, research), defer to web_search verification
        // which happens async in the caller. For now, mark as unverifiable.
        _ => VerificationResult::Unverifiable {
            reason: format!(
                "Open-ended {:?} scene — requires external verification",
                scene.scene_type
            ),
        },
    }
}

// ─── Result Routing ────────────────────────────────────────────────

/// Route a verified answer to the appropriate buffer.
fn route_by_verification(
    verification: &VerificationResult,
    student_answer: &str,
    scene: &Scene,
    course_id: &str,
    lesson_id: &str,
    scene_index: usize,
    _level: EducationLevel,
    session: &mut StudentSession,
) {
    match verification {
        VerificationResult::Confirmed { confidence, .. } => {
            tracing::info!(
                module = "student", course_id, lesson_id, scene_index,
                confidence, "Answer confirmed → Golden Buffer"
            );
            session.pending_golden.push(TrainingSample {
                id: uuid::Uuid::new_v4().to_string(),
                input: build_scene_prompt(scene),
                output: student_answer.to_string(),
                method: TrainingMethod::Sft,
                quality_score: *confidence,
                timestamp: chrono::Utc::now(),
            });
        }
        VerificationResult::Contradicted { correct_answer, evidence, .. } => {
            tracing::info!(
                module = "student", course_id, lesson_id, scene_index,
                "Answer contradicted → Rejection Buffer"
            );
            let chosen = if correct_answer.is_empty() {
                scene.expected_output.clone().unwrap_or_default()
            } else {
                correct_answer.clone()
            };
            session.pending_rejections.push(PendingRejection {
                input: build_scene_prompt(scene),
                chosen,
                rejected: student_answer.to_string(),
                reason: evidence.clone(),
            });
        }
        VerificationResult::Unverifiable { reason } => {
            tracing::info!(
                module = "student", course_id, lesson_id, scene_index,
                reason, "Answer unverifiable → Quarantine"
            );
            session.pending_quarantine.push(QuarantineEntry {
                id: uuid::Uuid::new_v4().to_string(),
                course_id: course_id.to_string(),
                lesson_id: lesson_id.to_string(),
                scene_index,
                student_answer: student_answer.to_string(),
                teacher_grade: 0.0,
                verification_attempts: vec![verification.clone()],
                timestamp: chrono::Utc::now(),
            });
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "student_tests.rs"]
mod tests;
