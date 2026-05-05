//! HUD data gathering helpers — extracted from ws_context.rs for §1.1 compliance.
//! These functions collect data from memory tiers, logs, and config
//! for injection into the dynamic HUD context.

use crate::provider::Message;

/// Phase 1: Keyword-match last 3 user messages against lessons, return top 3.
pub fn match_relevant_lessons(
    lessons: &crate::memory::lessons::LessonStore,
    content: &str,
    history: &[Message],
) -> Vec<(f32, String)> {
    let keywords = extract_user_keywords(content, history);
    if keywords.is_empty() { return vec![]; }
    let mut matched = Vec::new();
    for lesson in lessons.all() {
        let rule_lower = lesson.rule.to_lowercase();
        if keywords.iter().any(|kw| rule_lower.contains(kw)) {
            matched.push((lesson.confidence, lesson.rule.clone()));
        }
        if matched.len() >= 3 { break; }
    }
    matched.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    matched
}

/// Phase 2: Keyword-match procedures, return matching names (max 3).
pub fn match_relevant_procedures(
    procs: &crate::memory::procedures::ProcedureStore,
    content: &str,
    history: &[Message],
) -> Vec<String> {
    let keywords = extract_user_keywords(content, history);
    if keywords.is_empty() { return vec![]; }
    let mut matched = Vec::new();
    for proc in procs.all() {
        let name_lower = proc.name.to_lowercase();
        let desc_lower = proc.description.to_lowercase();
        if keywords.iter().filter(|kw| kw.len() > 4).any(|kw| name_lower.contains(kw) || desc_lower.contains(kw)) {
            matched.push(proc.name.clone());
        }
        if matched.len() >= 3 { break; }
    }
    matched
}

/// Shared keyword extraction: last 3 user messages, words > 3 chars.
fn extract_user_keywords(content: &str, history: &[Message]) -> std::collections::HashSet<String> {
    let mut words = std::collections::HashSet::new();
    for w in content.split_whitespace() {
        let clean: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
        if clean.len() > 3 { words.insert(clean.to_lowercase()); }
    }
    for msg in history.iter().rev().filter(|m| m.role == "user").take(3) {
        for w in msg.text_content().split_whitespace() {
            let clean: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if clean.len() > 3 { words.insert(clean.to_lowercase()); }
        }
    }
    words
}

/// Phase 4: Format scratchpad entries as content (capped at 2000 chars).
pub fn format_scratchpad_content(scratchpad: &crate::memory::scratchpad::ScratchpadStore) -> Option<String> {
    let entries = scratchpad.all();
    if entries.is_empty() { return None; }
    let mut out = String::new();
    for entry in entries {
        let line = format!("{}: {}\n", entry.key, entry.value);
        if out.len() + line.len() > 2000 { break; }
        out.push_str(&line);
    }
    Some(out)
}

/// Phase 5: Read last 10 WARN/ERROR lines from today's log file.
pub fn read_log_tail(data_dir: &std::path::Path) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let log_path = data_dir.join("logs").join(format!("ern-os.log.{}", date));
    let content = match std::fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let filtered: Vec<&str> = content.lines()
        .filter(|l| l.contains("WARN") || l.contains("ERROR"))
        .collect();
    let tail = if filtered.len() > 10 { &filtered[filtered.len() - 10..] } else { &filtered[..] };
    tail.join("\n")
}

/// Phase 6: Format knowledge graph snapshot — recent nodes + edges.
pub fn format_kg_snapshot(synaptic: &crate::memory::synaptic::SynapticGraph) -> String {
    if synaptic.node_count() == 0 { return String::new(); }
    let mut out = String::new();
    let nodes = synaptic.recent_nodes(5);
    if !nodes.is_empty() {
        out.push_str("Recent nodes: ");
        let names: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        out.push_str(&names.join(", "));
    }
    let edges = synaptic.all_edges();
    if !edges.is_empty() {
        let recent: Vec<&crate::memory::synaptic::SynapticEdge> = {
            let mut sorted: Vec<_> = edges.iter().collect();
            sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            sorted.into_iter().take(5).collect()
        };
        if !out.is_empty() { out.push('\n'); }
        out.push_str("Recent edges: ");
        let edge_strs: Vec<String> = recent.iter()
            .map(|e| format!("({}) -[{}]-> ({})", e.source, e.edge_type, e.target))
            .collect();
        out.push_str(&edge_strs.join(", "));
    }
    out
}

/// Phase 7: Extract last 3 thinking traces from the persisted reasoning log.
/// Reads `data/reasoning/{session_id}.jsonl` and returns entries with actual
/// chain-of-thought content, truncated to 500 chars each (char-boundary safe).
pub fn extract_recent_reasoning(data_dir: &std::path::Path, session_id: &str) -> Vec<String> {
    let path = data_dir.join("reasoning").join(format!("{}.jsonl", session_id));
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut traces: Vec<String> = content.lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let thinking = v["thinking"].as_str()?;
            if thinking.is_empty() { return None; }
            let truncated = if thinking.len() > 500 {
                let boundary = thinking.char_indices()
                    .take_while(|(i, _)| *i <= 500)
                    .last().map(|(i, _)| i).unwrap_or(0);
                format!("{}...", &thinking[..boundary])
            } else {
                thinking.to_string()
            };
            Some(truncated)
        })
        .collect();

    // Keep only the last 3
    if traces.len() > 3 {
        traces = traces.split_off(traces.len() - 3);
    }
    traces
}

/// Phase 8: Format active steering vectors.
pub fn format_active_steering(data_dir: &std::path::Path) -> Option<String> {
    let steering_dir = data_dir.join("steering");
    if !steering_dir.exists() { return None; }
    let store = crate::steering::vectors::VectorStore::new(&steering_dir).ok()?;
    let active = store.active_vectors();
    if active.is_empty() { return None; }
    let parts: Vec<String> = active.iter()
        .map(|v| format!("{} ({:.1})", v.name, v.strength))
        .collect();
    Some(parts.join(", "))
}

/// Phase 10: Format timeline narrative from recent entries.
pub fn format_timeline_narrative(timeline: &crate::memory::timeline::TimelineStore) -> String {
    let recent = timeline.recent(10);
    if recent.is_empty() { return String::new(); }
    let mut out = String::new();
    for entry in recent.iter().take(5) {
        let time = entry.timestamp.format("%H:%M").to_string();
        let summary: &str = entry.transcript.lines().next().unwrap_or("(empty)");
        let truncated = if summary.len() > 80 {
            let boundary = summary.char_indices().take_while(|(i, _)| *i <= 80)
                .last().map(|(i, _)| i).unwrap_or(0);
            &summary[..boundary]
        } else { summary };
        out.push_str(&format!("[{}] {}\n", time, truncated));
    }
    out
}

/// Phase 11: Load user preferences (best-effort, per-user JSON).
pub fn load_user_preferences(data_dir: &std::path::Path, session_id: &str) -> Option<String> {
    let parts: Vec<&str> = session_id.split('_').collect();
    let user_id = if parts.len() >= 2 { parts[1] } else { session_id };
    let prefs_path = data_dir.join("preferences").join(format!("{}.json", user_id));
    let content = std::fs::read_to_string(&prefs_path).ok()?;
    let prefs: serde_json::Value = serde_json::from_str(&content).ok()?;
    let entries = prefs.as_array()?;
    if entries.is_empty() { return None; }
    let mut out = String::new();
    for entry in entries {
        let key = entry["key"].as_str().unwrap_or("unknown");
        let value = entry["value"].as_str().unwrap_or("");
        out.push_str(&format!("- {}: {}\n", key, value));
    }
    Some(out)
}

/// Phase 12: Format scheduler job status.
pub fn format_scheduler_status(store: &crate::scheduler::store::JobStore) -> String {
    let jobs = store.list();
    if jobs.is_empty() { return "No jobs configured".to_string(); }
    let now = chrono::Utc::now();
    let mut lines = Vec::new();
    for job in jobs.iter().take(10) {
        let last = match job.last_run {
            Some(t) => {
                let ago = (now - t).num_minutes();
                if ago < 60 { format!("{}m ago", ago) }
                else if ago < 1440 { format!("{}h ago", ago / 60) }
                else { format!("{}d ago", ago / 1440) }
            }
            None => "never".to_string(),
        };
        let status = if !job.enabled { "⏸" }
            else if job.last_result.as_deref().map_or(false, |r| r.contains("error") || r.contains("fail")) { "❌" }
            else { "✅" };
        lines.push(format!("- {} | last: {} {}", job.name, last, status));
    }
    lines.join("\n")
}
