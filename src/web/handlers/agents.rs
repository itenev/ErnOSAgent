//! Agent & team CRUD handlers.

use crate::web::state::AppState;
use axum::{extract::State, response::IntoResponse, Json};

pub async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.agents.read().await;
    Json(serde_json::json!({ "agents": agents.list() }))
}

pub async fn create_agent(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = body["name"].as_str().unwrap_or("New Agent");
    let description = body["description"].as_str().unwrap_or("");
    let mut agent = crate::agents::AgentDefinition::new(name, description);

    if let Some(tools) = body["tools"].as_array() {
        agent.tools = tools.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(obs) = body["observer_enabled"].as_bool() {
        agent.observer_enabled = obs;
    }

    let mut agents = state.agents.write().await;
    match agents.create(agent) {
        Ok(created) => Json(serde_json::json!({ "ok": true, "agent": created })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

pub async fn get_agent(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let agents = state.agents.read().await;
    match agents.get(&id) {
        Some(agent) => Json(serde_json::json!({ "agent": agent })),
        None => Json(serde_json::json!({ "error": "Agent not found" })),
    }
}

pub async fn update_agent(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut agents = state.agents.write().await;
    let existing = match agents.get(&id) {
        Some(a) => a.clone(),
        None => return Json(serde_json::json!({ "error": "Agent not found" })),
    };

    let mut updated = existing;
    if let Some(name) = body["name"].as_str() { updated.name = name.to_string(); }
    if let Some(desc) = body["description"].as_str() { updated.description = desc.to_string(); }
    if let Some(tools) = body["tools"].as_array() {
        updated.tools = tools.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(obs) = body["observer_enabled"].as_bool() { updated.observer_enabled = obs; }
    updated.updated_at = chrono::Utc::now();

    match agents.update(updated) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

pub async fn delete_agent(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let mut agents = state.agents.write().await;
    match agents.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

pub async fn list_teams(State(state): State<AppState>) -> impl IntoResponse {
    let teams = state.teams.read().await;
    Json(serde_json::json!({ "teams": teams.list() }))
}

pub async fn create_team(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = body["name"].as_str().unwrap_or("New Team");
    let description = body["description"].as_str().unwrap_or("");
    let mode = match body["mode"].as_str() {
        Some("sequential") => crate::agents::teams::ExecutionMode::Sequential,
        _ => crate::agents::teams::ExecutionMode::Parallel,
    };
    let agent_ids: Vec<String> = body["agents"].as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let team = crate::agents::teams::TeamDefinition::new(name, description, mode, agent_ids);
    let mut teams = state.teams.write().await;
    match teams.create(team) {
        Ok(created) => Json(serde_json::json!({ "ok": true, "team": created })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

pub async fn delete_team(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let mut teams = state.teams.write().await;
    match teams.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// Return recent agent activity feed (tool usage, status changes).
pub async fn activity_feed(State(state): State<AppState>) -> impl IntoResponse {
    let path = state.config.general.data_dir.join("agent_activity.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            ([(axum::http::header::CONTENT_TYPE, "application/json")], content).into_response()
        }
        Err(_) => Json(serde_json::json!({
            "entries": [],
            "active_agents": 0,
        })).into_response(),
    }
}

/// Send a prompt direction to a running agent mid-task.
pub async fn send_direction(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let direction = body["direction"].as_str().unwrap_or("").to_string();
    if direction.is_empty() {
        return Json(serde_json::json!({"error": "direction is required"}));
    }

    // Write direction to a file the running agent polls
    let dir_path = state.config.general.data_dir.join("agent_directions");
    let _ = tokio::fs::create_dir_all(&dir_path).await;
    let file = dir_path.join(format!("{}.json", id));
    let payload = serde_json::json!({
        "agent_id": id,
        "direction": direction,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let _ = tokio::fs::write(&file, serde_json::to_string(&payload).unwrap()).await;
    tracing::info!(agent = %id, direction_len = direction.len(), "Prompt direction sent to agent");
    Json(serde_json::json!({"ok": true, "agent_id": id}))
}
