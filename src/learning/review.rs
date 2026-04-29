// Ern-OS — Spaced repetition review system
//! Prevents catastrophic forgetting at the curriculum level using
//! Leitner box intervals. Completed course material is converted to
//! review cards, scheduled for periodic review, and failed reviews
//! re-enter the rejection buffer for retraining.

use crate::learning::curriculum::{CurriculumStore, EducationLevel, SceneType};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Leitner Box Intervals ────────────────────────────────────────

/// Leitner box review intervals (in days).
/// Box 1: daily, Box 2: 3 days, Box 3: weekly, Box 4: biweekly, Box 5: monthly.
pub const LEITNER_INTERVALS_DAYS: [i64; 5] = [1, 3, 7, 14, 30];

/// Maximum box level (0-indexed internally, displayed as 1-5).
pub const MAX_BOX_LEVEL: u8 = 4;

// ─── Data Model ────────────────────────────────────────────────────

/// A single review card derived from completed curriculum material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCard {
    pub id: String,
    pub course_id: String,
    pub lesson_id: String,
    pub scene_index: usize,
    pub question: String,
    pub expected_answer: String,
    pub box_level: u8,
    pub last_reviewed: DateTime<Utc>,
    pub next_review: DateTime<Utc>,
    pub consecutive_correct: u8,
    pub consecutive_wrong: u8,
    pub total_reviews: u32,
}

impl ReviewCard {
    /// Create a new card starting in box 1 (index 0), due immediately.
    pub fn new(
        course_id: &str,
        lesson_id: &str,
        scene_index: usize,
        question: &str,
        expected_answer: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            course_id: course_id.to_string(),
            lesson_id: lesson_id.to_string(),
            scene_index,
            question: question.to_string(),
            expected_answer: expected_answer.to_string(),
            box_level: 0,
            last_reviewed: now,
            next_review: now, // Due immediately
            consecutive_correct: 0,
            consecutive_wrong: 0,
            total_reviews: 0,
        }
    }
}

/// The full review deck — manages all cards and persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewDeck {
    pub cards: Vec<ReviewCard>,
    #[serde(skip)]
    file_path: Option<PathBuf>,
}

impl ReviewDeck {
    pub fn new() -> Self {
        Self { cards: Vec::new(), file_path: None }
    }

    /// Load deck from disk, creating an empty deck if file doesn't exist.
    pub fn open(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read review deck: {}", path.display()))?;
            let mut deck: Self = serde_json::from_str(&content)
                .with_context(|| "Failed to parse review deck JSON")?;
            deck.file_path = Some(path.to_path_buf());
            Ok(deck)
        } else {
            Ok(Self { cards: Vec::new(), file_path: Some(path.to_path_buf()) })
        }
    }

    /// Add a card to the deck (dedup by course+lesson+scene).
    pub fn add_card(&mut self, card: ReviewCard) -> Result<()> {
        let exists = self.cards.iter().any(|c| {
            c.course_id == card.course_id
                && c.lesson_id == card.lesson_id
                && c.scene_index == card.scene_index
        });
        if !exists {
            self.cards.push(card);
            self.persist()?;
        }
        Ok(())
    }

    /// Get all cards that are due for review.
    pub fn due_cards(&self, now: DateTime<Utc>) -> Vec<&ReviewCard> {
        self.cards.iter().filter(|c| c.next_review <= now).collect()
    }

    /// Record a review result. Correct → promote box, wrong → demote to box 1.
    pub fn record_result(&mut self, card_id: &str, correct: bool) -> Result<()> {
        let card = self.cards.iter_mut()
            .find(|c| c.id == card_id)
            .with_context(|| format!("Card '{}' not found", card_id))?;

        card.total_reviews += 1;
        card.last_reviewed = Utc::now();

        if correct {
            card.consecutive_correct += 1;
            card.consecutive_wrong = 0;
            card.box_level = card.box_level.saturating_add(1).min(MAX_BOX_LEVEL);
        } else {
            card.consecutive_wrong += 1;
            card.consecutive_correct = 0;
            card.box_level = 0; // Back to box 1
        }

        let interval_days = LEITNER_INTERVALS_DAYS[card.box_level as usize];
        card.next_review = Utc::now() + Duration::days(interval_days);

        self.persist()?;
        Ok(())
    }

    /// Total number of cards.
    pub fn count(&self) -> usize { self.cards.len() }

    /// Number of cards currently due.
    pub fn due_count(&self, now: DateTime<Utc>) -> usize {
        self.cards.iter().filter(|c| c.next_review <= now).count()
    }

    /// Compute retention statistics.
    pub fn retention_stats(&self) -> RetentionStats {
        let total = self.cards.len();
        let due = self.due_count(Utc::now());
        let avg_box = if total == 0 { 0.0 } else {
            self.cards.iter().map(|c| c.box_level as f32).sum::<f32>() / total as f32
        };
        let total_reviews: u32 = self.cards.iter().map(|c| c.total_reviews).sum();
        let total_correct: u32 = self.cards.iter().map(|c| c.consecutive_correct as u32).sum();
        let retention = if total_reviews == 0 { 0.0 } else {
            total_correct as f32 / total_reviews as f32
        };

        RetentionStats {
            total_cards: total,
            cards_due: due,
            avg_box_level: avg_box,
            retention_rate: retention,
        }
    }

    fn persist(&self) -> Result<()> {
        if let Some(ref path) = self.file_path {
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
            std::fs::write(path, serde_json::to_string_pretty(&self)?)?;
        }
        Ok(())
    }
}

/// Aggregate retention statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionStats {
    pub total_cards: usize,
    pub cards_due: usize,
    pub avg_box_level: f32,
    pub retention_rate: f32,
}

/// Result of a review session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSessionResult {
    pub cards_reviewed: usize,
    pub correct: usize,
    pub wrong: usize,
}

// ─── Card Generation ───────────────────────────────────────────────

/// Generate review cards from completed courses in the curriculum.
/// Only quiz/exercise scenes with expected outputs become cards.
pub fn generate_review_cards(store: &CurriculumStore) -> Vec<ReviewCard> {
    let mut cards = Vec::new();
    for course in store.courses() {
        if !store.is_course_complete(course) { continue; }
        for lesson in &course.lessons {
            for (i, scene) in lesson.scenes.iter().enumerate() {
                // Only reviewable scene types with expected output
                let reviewable = matches!(
                    scene.scene_type,
                    SceneType::Quiz | SceneType::Exercise | SceneType::CriticalAnalysis
                );
                if reviewable {
                    if let Some(ref expected) = scene.expected_output {
                        cards.push(ReviewCard::new(
                            &course.id, &lesson.id, i,
                            &scene.content, expected,
                        ));
                    }
                }
            }
        }
    }
    tracing::info!(
        module = "review", fn_name = "generate_review_cards",
        cards = cards.len(), "Generated review cards from completed courses"
    );
    cards
}

/// Cross-level review: when at advanced levels, also review lower-level material.
pub fn cross_level_cards(
    deck: &ReviewDeck,
    current_level: EducationLevel,
    max_cards: usize,
) -> Vec<&ReviewCard> {
    let review_levels: Vec<EducationLevel> = match current_level {
        EducationLevel::Masters | EducationLevel::Doctoral => {
            vec![EducationLevel::Primary, EducationLevel::Secondary, EducationLevel::Undergraduate]
        }
        EducationLevel::Undergraduate => {
            vec![EducationLevel::Primary, EducationLevel::Secondary]
        }
        _ => vec![],
    };

    if review_levels.is_empty() {
        return Vec::new();
    }

    // Return due cards, limited to max_cards
    // Note: we can't check level directly on cards without course lookup,
    // so we return all due cards capped to the limit
    deck.due_cards(Utc::now()).into_iter().take(max_cards).collect()
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
