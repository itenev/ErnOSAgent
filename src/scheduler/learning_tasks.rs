// Ern-OS — Scheduler learning task handlers
//! Extracted from mod.rs to keep file under 500-line governance limit.
//! Contains: AttendClass, ConductResearch, SpacedReview handlers.

use crate::web::state::AppState;

/// Attend class — run one lesson from the curriculum through the student loop.
/// This is the cold-path integration: locks are held only for micro-second flushes.
pub(super) async fn run_attend_class(course_id: &str, state: &AppState) -> (bool, String) {
    use crate::learning::student::{self, StudentSession};

    // 1. Find the course and next lesson (short read lock)
    let (course, lesson) = {
        let store = state.curriculum.read().await;
        let course = if course_id.is_empty() {
            // Pick first course with incomplete lessons
            store.courses().iter()
                .find(|c| store.next_lesson(c).is_some())
                .cloned()
        } else {
            store.get_course(course_id).cloned()
        };
        let course = match course {
            Some(c) => c,
            None => return (true, "No courses available or all complete".into()),
        };
        let lesson = match store.next_lesson(&course) {
            Some(l) => l.clone(),
            None => return (true, format!("Course '{}' complete", course.title)),
        };
        (course, lesson)
    }; // read lock released

    tracing::info!(
        module = "scheduler", fn_name = "run_attend_class",
        course = %course.title, lesson = %lesson.title,
        scenes = lesson.scenes.len(),
        "Attending class"
    );

    // 2. Process all scenes — NO shared locks held
    let mut session = StudentSession::new();
    for (i, scene) in lesson.scenes.iter().enumerate() {
        // Check user-activity preemption
        if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("AttendClass preempted by user activity at scene {}", i);
            // Save position for resume
            let mut store = state.curriculum.write().await;
            let _ = store.save_position(&course.id, &lesson.id, i);
            return (true, format!("Preempted at scene {}/{}", i, lesson.scenes.len()));
        }

        match student::process_scene(
            state.provider.as_ref(), scene, &course.id, &lesson.id,
            i, course.level, &mut session,
        ).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!(
                    module = "scheduler", scene = i, error = %e,
                    "Scene processing failed — skipping"
                );
            }
        }
    }

    // 3. Flush local buffers → shared state (microsecond locks)
    let flush_result = flush_session(&session, state).await;

    // 4. Mark lesson complete (short write lock)
    let score = session.average_score();
    {
        let mut store = state.curriculum.write().await;
        let _ = store.complete_lesson(
            &course.id, &lesson.id,
            if session.scenes_processed > 0 { Some(score) } else { None },
        );
    }

    let msg = format!(
        "lesson='{}' scenes={} golden={} rejected={} quarantine={} avg_score={:.2} {}",
        lesson.title, session.scenes_processed,
        session.pending_golden.len(),
        session.pending_rejections.len(),
        session.pending_quarantine.len(),
        score, flush_result,
    );
    (true, msg)
}

/// Flush StudentSession local buffers to shared state.
/// Each lock is held for the absolute minimum duration.
pub(super) async fn flush_session(
    session: &crate::learning::student::StudentSession, state: &AppState,
) -> String {
    let mut flushed_golden = 0usize;
    let mut flushed_rejected = 0usize;
    let mut flushed_quarantine = 0usize;

    // Flush golden samples
    if !session.pending_golden.is_empty() {
        let mut buf = state.golden_buffer.write().await;
        for sample in &session.pending_golden {
            match buf.add(sample.clone()) {
                Ok(()) => { flushed_golden += 1; }
                Err(e) => { tracing::warn!(error = %e, "Failed to flush golden sample"); }
            }
        }
    } // write lock released

    // Flush rejection pairs
    if !session.pending_rejections.is_empty() {
        let mut buf = state.rejection_buffer.write().await;
        for rej in &session.pending_rejections {
            if let Err(e) = buf.add_pair(&rej.input, &rej.chosen, &rej.rejected, &rej.reason) {
                tracing::warn!(error = %e, "Failed to flush rejection pair");
            } else {
                flushed_rejected += 1;
            }
        }
    } // write lock released

    // Flush quarantine entries
    if !session.pending_quarantine.is_empty() {
        let mut buf = state.quarantine.write().await;
        for entry in &session.pending_quarantine {
            match buf.add(entry.clone()) {
                Ok(()) => { flushed_quarantine += 1; }
                Err(e) => { tracing::warn!(error = %e, "Failed to flush quarantine entry"); }
            }
        }
    } // write lock released

    format!(
        "flushed: golden={} rejected={} quarantine={}",
        flushed_golden, flushed_rejected, flushed_quarantine,
    )
}

/// Conduct research — advance the next phase of a PhD research project.
pub(super) async fn run_conduct_research(project_id: &str, state: &AppState) -> (bool, String) {
    use crate::learning::research;

    let research_dir = state.config.general.data_dir.join("research");

    // Find or create the active project
    let mut project = if project_id.is_empty() {
        let projects = research::list_projects(&research_dir);
        match projects.into_iter().find(|p| p.phase != research::ResearchPhase::Complete) {
            Some(p) => p,
            None => return (true, "No active research projects".into()),
        }
    } else {
        match research::load_project(&research_dir, project_id) {
            Ok(p) => p,
            Err(e) => return (false, format!("Failed to load project: {}", e)),
        }
    };

    tracing::info!(
        module = "scheduler", fn_name = "run_conduct_research",
        project = %project.title, phase = ?project.phase,
        "Conducting research"
    );

    let result = match project.phase {
        research::ResearchPhase::LiteratureSurvey => {
            match crate::tools::web_search::search(
                &format!("site:arxiv.org {} recent research", project.domain)
            ).await {
                Ok(search_results) => {
                    match research::extract_paper_metadata(
                        state.provider.as_ref(), "search_result", &search_results
                    ).await {
                        Ok(paper) => {
                            project.papers.push(paper.clone());
                            format!("Ingested paper: '{}' (total: {})", paper.title, project.papers.len())
                        }
                        Err(e) => format!("Paper extraction failed: {}", e),
                    }
                }
                Err(e) => format!("arXiv search failed: {}", e),
            }
        }
        research::ResearchPhase::HypothesisGeneration => {
            match research::generate_hypotheses(
                state.provider.as_ref(), &project, 3
            ).await {
                Ok(hypotheses) => {
                    let count = hypotheses.len();
                    project.hypotheses.extend(hypotheses);
                    format!("Generated {} hypotheses (total: {})", count, project.hypotheses.len())
                }
                Err(e) => format!("Hypothesis generation failed: {}", e),
            }
        }
        research::ResearchPhase::Defense => {
            let thesis = project.hypotheses.iter()
                .find(|h| h.status == research::HypothesisStatus::Confirmed
                    || h.status == research::HypothesisStatus::Testing)
                .map(|h| h.statement.clone())
                .unwrap_or_else(|| project.title.clone());

            match research::run_defense_round(
                state.provider.as_ref(), &project, &thesis
            ).await {
                Ok(round) => {
                    let quality = round.defense_quality;
                    project.defense_log.push(round);
                    format!("Defense round complete (quality: {:.2}, total: {})",
                        quality, project.defense_log.len())
                }
                Err(e) => format!("Defense round failed: {}", e),
            }
        }
        research::ResearchPhase::Experimentation
        | research::ResearchPhase::PaperWriting => {
            format!("Phase {:?} is handled by curriculum scenes", project.phase)
        }
        research::ResearchPhase::Complete => {
            "Research project already complete".into()
        }
    };

    // Try to advance phase if gate criteria met
    let advanced = match project.try_advance() {
        Ok(next) => format!(" → Advanced to {:?}", next),
        Err(_) => String::new(),
    };

    // Persist project state
    if let Err(e) = research::save_project(&project, &research_dir) {
        tracing::warn!(error = %e, "Failed to save research project");
    }

    (true, format!("{}{}", result, advanced))
}

/// Spaced review — process due review cards and route results to training buffers.
pub(super) async fn run_spaced_review(state: &AppState) -> (bool, String) {
    use crate::learning::review;

    // Generate new cards from any newly completed courses
    {
        let curriculum = state.curriculum.read().await;
        let new_cards = review::generate_review_cards(&curriculum);
        if !new_cards.is_empty() {
            let mut deck = state.review_deck.write().await;
            for card in new_cards {
                let _ = deck.add_card(card);
            }
        }
    }

    let due_count = {
        let deck = state.review_deck.read().await;
        deck.due_count(chrono::Utc::now())
    };

    if due_count == 0 {
        return (true, "No review cards due".into());
    }

    let max_cards = due_count.min(10);
    let mut correct = 0usize;
    let mut wrong = 0usize;

    let cards_data: Vec<(String, String, String)> = {
        let deck = state.review_deck.read().await;
        deck.due_cards(chrono::Utc::now())
            .into_iter()
            .take(max_cards)
            .map(|c| (c.id.clone(), c.question.clone(), c.expected_answer.clone()))
            .collect()
    };

    for (card_id, question, expected) in &cards_data {
        if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::info!(module = "scheduler", "Spaced review preempted by user activity");
            break;
        }

        let messages = vec![
            crate::provider::Message::text("user", question),
        ];
        match state.provider.chat_sync(&messages, None).await {
            Ok(answer) => {
                let is_correct = answer.to_lowercase().contains(&expected.to_lowercase())
                    || expected.to_lowercase().contains(&answer.to_lowercase().trim());

                let mut deck = state.review_deck.write().await;
                let _ = deck.record_result(card_id, is_correct);
                drop(deck);

                if is_correct {
                    correct += 1;
                } else {
                    wrong += 1;
                    let mut rej = state.rejection_buffer.write().await;
                    let _ = rej.add_pair(question, expected, &answer, "Failed spaced review");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Spaced review inference failed");
            }
        }
    }

    (true, format!(
        "Reviewed {} cards: {} correct, {} wrong ({} still due)",
        correct + wrong, correct, wrong, due_count - (correct + wrong)
    ))
}
