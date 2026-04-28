//! Introspection tool — self-awareness layer for reasoning logs, activity, and system status.

use crate::web::state::AppState;
use anyhow::Result;
use std::path::Path;

/// Execute an introspect action.
pub async fn execute(args: &serde_json::Value, state: &AppState) -> Result<String> {
    let action = args["action"].as_str().unwrap_or("system_status");
    match action {
        "reasoning_log" => get_reasoning_log(&state.config.general.data_dir, args),
        "agent_activity" => get_agent_activity(&state.config.general.data_dir, args),
        "scheduler_status" => get_scheduler_status(state).await,
        "observer_audit" => get_observer_audit(&state.config.general.data_dir, args),
        "system_status" => get_system_status(state).await,
        "my_tools" => list_available_tools(),
        other => Ok(format!("Unknown introspect action: {}", other)),
    }
}

/// Read the per-session reasoning log (JSONL).
fn get_reasoning_log(data_dir: &Path, args: &serde_json::Value) -> Result<String> {
    let session_id = args["session_id"].as_str().unwrap_or("");

    let dir = data_dir.join("reasoning");
    let path = if session_id.is_empty() {
        most_recent_file(&dir, "jsonl")?
    } else {
        dir.join(format!("{}.jsonl", session_id))
    };

    if !path.exists() {
        return Ok("No reasoning log found for this session.".to_string());
    }

    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let page = args["page"].as_u64().unwrap_or(1).max(1) as usize;
    let per_page = args["per_page"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
    let total_pages = (total + per_page - 1).max(1) / per_page.max(1);
    let page = page.min(total_pages);
    // Show most recent entries first — reverse order
    let reversed: Vec<&str> = lines.iter().rev().cloned().collect();
    let start = (page - 1) * per_page;
    let end = (start + per_page).min(total);
    let slice = &reversed[start..end];

    Ok(format!("Reasoning log:\n{}\n--- Page {}/{} ({} total) ---",
        slice.join("\n"), page, total_pages, total))
}

/// Read agent activity feed.
fn get_agent_activity(data_dir: &Path, args: &serde_json::Value) -> Result<String> {
    let path = data_dir.join("agent_activity.json");

    if !path.exists() {
        return Ok("No agent activity recorded yet.".to_string());
    }

    let content = std::fs::read_to_string(&path)?;
    let data: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
    let entries = data["entries"].as_array();

    match entries {
        Some(arr) => {
            let total = arr.len();
            let page = args["page"].as_u64().unwrap_or(1).max(1) as usize;
            let per_page = args["per_page"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
            let total_pages = (total + per_page - 1).max(1) / per_page.max(1);
            let page = page.min(total_pages);
            let start = (page - 1) * per_page;
            let end = (start + per_page).min(total);
            let items: Vec<String> = arr[start..end].iter()
                .map(|e| serde_json::to_string(e).unwrap_or_default())
                .collect();
            Ok(format!("Agent activity:\n{}\n--- Page {}/{} ({} total) ---",
                items.join("\n"), page, total_pages, total))
        }
        None => Ok("No agent activity entries.".to_string()),
    }
}

/// Read scheduler status via JobStore.
async fn get_scheduler_status(state: &AppState) -> Result<String> {
    let store = state.scheduler.read().await;
    let jobs = store.list();
    let history = store.get_history();
    let recent: Vec<String> = history.iter().rev().take(5).map(|e| {
        format!("  {} — {} at {} ({})",
            e.job_name,
            if e.success { "✅" } else { "❌" },
            e.started_at.format("%H:%M"),
            e.result)
    }).collect();

    Ok(format!("Scheduler: {} jobs configured\n\nJobs:\n{}\n\nRecent executions:\n{}",
        jobs.len(),
        jobs.iter().map(|j| format!("  {} [{}] — {}", j.name, if j.enabled {"on"} else {"off"}, j.schedule))
            .collect::<Vec<_>>().join("\n"),
        if recent.is_empty() { "  (none)".to_string() } else { recent.join("\n") }
    ))
}

/// Read observer audit history.
fn get_observer_audit(data_dir: &Path, args: &serde_json::Value) -> Result<String> {
    let path = data_dir.join("observer_history.json");

    if !path.exists() {
        return Ok("No observer audit history found.".to_string());
    }

    let content = std::fs::read_to_string(&path)?;
    let data: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
    let entries = data.as_array();

    match entries {
        Some(arr) => {
            let total = arr.len();
            let page = args["page"].as_u64().unwrap_or(1).max(1) as usize;
            let per_page = args["per_page"].as_u64().unwrap_or(10).clamp(1, 100) as usize;
            let total_pages = (total + per_page - 1).max(1) / per_page.max(1);
            let page = page.min(total_pages);
            // Reverse to show most recent first
            let reversed: Vec<&serde_json::Value> = arr.iter().rev().collect();
            let start = (page - 1) * per_page;
            let end = (start + per_page).min(total);
            let items: Vec<String> = reversed[start..end].iter().map(|e| {
                format!("  {} | conf:{} | {} | {}",
                    if e["approved"].as_bool().unwrap_or(true) { "✅" } else { "❌" },
                    e["confidence"].as_f64().unwrap_or(0.0),
                    e["category"].as_str().unwrap_or(""),
                    e["reason"].as_str().unwrap_or(""))
            }).collect();
            Ok(format!("Observer audit:\n{}\n--- Page {}/{} ({} total) ---",
                items.join("\n"), page, total_pages, total))
        }
        None => Ok("No observer audit entries.".to_string()),
    }
}

/// Get system status — model, memory, provider health.
async fn get_system_status(state: &AppState) -> Result<String> {
    let model = &state.config.llamacpp.model_path;
    let mem = state.memory.read().await;
    let mem_summary = mem.status_summary();
    let provider = state.provider.as_ref();
    let healthy = provider.health().await;

    Ok(format!("System Status:\n  Model: {}\n  Provider: {}\n  Memory:\n    {}",
        model,
        if healthy { "✅ healthy" } else { "❌ unhealthy" },
        mem_summary.replace('\n', "\n    ")))
}

/// List all available tools.
fn list_available_tools() -> Result<String> {
    let l1 = crate::tools::schema::layer1_tools();
    let l2 = crate::tools::schema::layer2_tools();

    let l1_names = extract_tool_names(&l1);
    let l2_names = extract_tool_names(&l2);
    let l2_only: Vec<&String> = l2_names.iter().filter(|n| !l1_names.contains(n)).collect();

    Ok(format!("Available tools:\n\nLayer 1 (direct):\n{}\n\nLayer 2 (ReAct only):\n{}",
        l1_names.iter().map(|n| format!("  • {}", n)).collect::<Vec<_>>().join("\n"),
        l2_only.iter().map(|n| format!("  • {}", n)).collect::<Vec<_>>().join("\n")))
}

fn extract_tool_names(tools: &serde_json::Value) -> Vec<String> {
    tools.as_array().unwrap_or(&vec![]).iter()
        .filter_map(|t| t["function"]["name"].as_str().map(|s| s.to_string()))
        .collect()
}

/// Find most recent file with given extension in a directory.
fn most_recent_file(dir: &Path, ext: &str) -> Result<std::path::PathBuf> {
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map_or(false, |e| e == ext) {
                if let Ok(meta) = p.metadata() {
                    let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    if best.as_ref().map_or(true, |(t, _)| modified > *t) {
                        best = Some((modified, p));
                    }
                }
            }
        }
    }
    Ok(best.map(|(_, p)| p).unwrap_or_else(|| dir.join("none.jsonl")))
}

/// Append a reasoning event to the per-session log.
pub fn log_reasoning_event(data_dir: &Path, session_id: &str, event: &serde_json::Value) {
    let dir = data_dir.join("reasoning");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.jsonl", session_id));
    if let Ok(mut line) = serde_json::to_string(event) {
        line.push('\n');
        let _ = std::fs::OpenOptions::new()
            .create(true).append(true).open(&path)
            .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
    }
}
