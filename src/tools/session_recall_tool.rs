//! Session recall tool — lets the agent browse and read prior chat sessions.

use crate::web::state::AppState;
use anyhow::Result;

/// Execute a session_recall action.
pub async fn execute(args: &serde_json::Value, state: &AppState) -> Result<String> {
    let action = args["action"].as_str().unwrap_or("list");
    match action {
        "list" => list_sessions(state, args).await,
        "get" => get_session(state, args).await,
        "summary" => summarize_session(state, args).await,
        "search" => search_sessions(state, args).await,
        "topics" => extract_topics(state, args).await,
        other => Ok(format!("Unknown session_recall action: {}", other)),
    }
}

/// Paginated session listing.
async fn list_sessions(state: &AppState, args: &serde_json::Value) -> Result<String> {
    let page = args["page"].as_u64().unwrap_or(1).max(1) as usize;
    let per_page = args["per_page"].as_u64().unwrap_or(10).min(50) as usize;
    let sessions = state.sessions.read().await;
    let all = sessions.list();
    let total = all.len();
    let start = (page - 1) * per_page;

    if start >= total {
        return Ok(format!("Page {} is empty. Total sessions: {}", page, total));
    }

    let end = (start + per_page).min(total);
    let page_sessions: Vec<serde_json::Value> = all[start..end].iter().map(|s| {
        serde_json::json!({
            "id": s.id,
            "title": s.title,
            "preview": s.preview(),
            "message_count": s.messages.len(),
            "created_at": s.created_at.to_rfc3339(),
            "updated_at": s.updated_at.to_rfc3339(),
            "pinned": s.pinned,
        })
    }).collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "page": page, "per_page": per_page,
        "total": total, "total_pages": (total + per_page - 1) / per_page,
        "sessions": page_sessions,
    }))?)
}

/// Get full message history for a session.
async fn get_session(state: &AppState, args: &serde_json::Value) -> Result<String> {
    let id = args["session_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return Ok("Error: session_id is required".to_string());
    }

    let sessions = state.sessions.read().await;
    match sessions.get(id) {
        Some(s) => {
            let msgs: Vec<String> = s.messages.iter().map(|m| {
                format!("[{}]: {}", m.role, m.text_content().chars().take(2000).collect::<String>())
            }).collect();
            Ok(format!("Session: {} ({})\nMessages ({}):\n\n{}",
                s.title, s.id, s.messages.len(), msgs.join("\n\n")))
        }
        None => Ok(format!("Session not found: {}", id)),
    }
}

/// Generate a topic summary from session messages.
async fn summarize_session(state: &AppState, args: &serde_json::Value) -> Result<String> {
    let id = args["session_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return Ok("Error: session_id is required".to_string());
    }

    let sessions = state.sessions.read().await;
    match sessions.get(id) {
        Some(s) => {
            let user_msgs: Vec<String> = s.messages.iter()
                .filter(|m| m.role == "user")
                .map(|m| m.text_content())
                .collect();
            let first_3: Vec<String> = user_msgs.iter().take(3)
                .map(|t| t.chars().take(100).collect::<String>()).collect();
            let last_3: Vec<String> = user_msgs.iter().rev().take(3).rev()
                .map(|t| t.chars().take(100).collect::<String>()).collect();

            Ok(format!("Session: {} ({})\nTotal messages: {}\nUser messages: {}\n\nFirst topics:\n{}\n\nLatest topics:\n{}",
                s.title, s.id, s.messages.len(), user_msgs.len(),
                first_3.iter().enumerate().map(|(i,t)| format!("  {}. {}", i+1, t)).collect::<Vec<_>>().join("\n"),
                last_3.iter().enumerate().map(|(i,t)| format!("  {}. {}", i+1, t)).collect::<Vec<_>>().join("\n"),
            ))
        }
        None => Ok(format!("Session not found: {}", id)),
    }
}

/// Search sessions using existing SessionManager::search.
async fn search_sessions(state: &AppState, args: &serde_json::Value) -> Result<String> {
    let query = args["query"].as_str().unwrap_or("");
    if query.is_empty() {
        return Ok("Error: query is required".to_string());
    }

    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let sessions = state.sessions.read().await;
    let results = sessions.search(query);
    let capped: Vec<serde_json::Value> = results.iter().take(limit).map(|r| {
        serde_json::json!({
            "session_id": r.session_id, "title": r.title,
            "snippet": r.snippet, "updated_at": r.updated_at.to_rfc3339(),
        })
    }).collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "query": query, "results": capped, "total_matches": results.len(),
    }))?)
}

/// Extract topic list from user messages.
async fn extract_topics(state: &AppState, args: &serde_json::Value) -> Result<String> {
    let id = args["session_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return Ok("Error: session_id is required".to_string());
    }

    let sessions = state.sessions.read().await;
    match sessions.get(id) {
        Some(s) => {
            let topics: Vec<String> = s.messages.iter()
                .filter(|m| m.role == "user")
                .map(|m| {
                    let text = m.text_content();
                    text.chars().take(80).collect::<String>().trim().to_string()
                })
                .filter(|t| !t.is_empty())
                .collect();

            Ok(format!("Session: {} ({})\nTopics ({}):\n{}",
                s.title, s.id, topics.len(),
                topics.iter().enumerate()
                    .map(|(i, t)| format!("  {}. {}", i + 1, t))
                    .collect::<Vec<_>>().join("\n")))
        }
        None => Ok(format!("Session not found: {}", id)),
    }
}
