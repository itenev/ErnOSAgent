//! Platform handler — REST API for managing platform adapters.

use crate::web::state::AppState;
use axum::{extract::State, Json};

/// GET /api/platforms — list all platform adapters and their status.
pub async fn list_platforms(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let reg = state.platforms.read().await;
    let statuses: Vec<serde_json::Value> = reg.statuses().iter().map(|s| {
        serde_json::json!({
            "name": s.name,
            "connected": s.connected,
            "error": s.error,
        })
    }).collect();

    Json(serde_json::json!({
        "platforms": statuses,
        "summary": reg.status_summary(),
    }))
}

/// POST /api/platforms/:name/connect — connect a specific platform.
pub async fn connect_platform(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let mut reg = state.platforms.write().await;
    match reg.connect_by_name(&name).await {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": format!("{} connected", name),
        })),
        Err(e) => Json(serde_json::json!({
            "success": false,
            "error": e.to_string(),
        })),
    }
}

/// POST /api/platforms/:name/disconnect — disconnect a specific platform.
pub async fn disconnect_platform(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let mut reg = state.platforms.write().await;
    match reg.disconnect_by_name(&name).await {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": format!("{} disconnected", name),
        })),
        Err(e) => Json(serde_json::json!({
            "success": false,
            "error": e.to_string(),
        })),
    }
}

/// GET /api/platforms/config — get platform config for the settings UI.
pub async fn get_platform_config(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let config = state.mutable_config.read().await;
    Json(serde_json::json!({
        "discord": {
            "enabled": config.discord.enabled,
            "token": config.discord.token.as_deref().map(mask_token),
            "has_token": config.discord.resolve_token().is_some(),
            "admin_ids": config.discord.admin_ids,
            "listen_channels": config.discord.listen_channels,
        },
        "telegram": {
            "enabled": config.telegram.enabled,
            "token": config.telegram.token.as_deref().map(mask_token),
            "has_token": config.telegram.resolve_token().is_some(),
            "admin_ids": config.telegram.admin_ids,
            "allowed_chats": config.telegram.allowed_chats,
        },
    }))
}

/// PUT /api/platforms/config — update platform config from the settings UI.
pub async fn update_platform_config(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let mut config = state.mutable_config.write().await;

    // Update Discord config
    if let Some(discord) = body.get("discord") {
        if let Some(enabled) = discord.get("enabled").and_then(|v| v.as_bool()) {
            config.discord.enabled = enabled;
        }
        if let Some(token) = discord.get("token").and_then(|v| v.as_str()) {
            if !token.is_empty() && !token.contains("****") {
                config.discord.token = Some(token.to_string());
            }
        }
        if let Some(ids) = discord.get("admin_ids").and_then(|v| v.as_array()) {
            config.discord.admin_ids = ids.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(channels) = discord.get("listen_channels").and_then(|v| v.as_array()) {
            config.discord.listen_channels = channels.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
    }

    // Update Telegram config
    if let Some(telegram) = body.get("telegram") {
        if let Some(enabled) = telegram.get("enabled").and_then(|v| v.as_bool()) {
            config.telegram.enabled = enabled;
        }
        if let Some(token) = telegram.get("token").and_then(|v| v.as_str()) {
            if !token.is_empty() && !token.contains("****") {
                config.telegram.token = Some(token.to_string());
            }
        }
        if let Some(ids) = telegram.get("admin_ids").and_then(|v| v.as_array()) {
            config.telegram.admin_ids = ids.iter()
                .filter_map(|v| v.as_i64())
                .collect();
        }
        if let Some(chats) = telegram.get("allowed_chats").and_then(|v| v.as_array()) {
            config.telegram.allowed_chats = chats.iter()
                .filter_map(|v| v.as_i64())
                .collect();
        }
    }

    // Persist to ern-os.toml
    if let Ok(serialized) = toml::to_string_pretty(&*config) {
        if let Err(e) = std::fs::write("ern-os.toml", serialized) {
            tracing::error!(error = %e, "Failed to persist platform config");
            return Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to write config: {}", e),
            }));
        }
    }

    // Drop the config lock before acquiring platform lock
    let discord_should_connect = config.discord.enabled && config.discord.resolve_token().is_some();
    let telegram_should_connect = config.telegram.enabled && config.telegram.resolve_token().is_some();
    drop(config);

    // Auto-connect platforms that are enabled and have a token
    let mut connect_results = Vec::new();
    let mut reg = state.platforms.write().await;

    if discord_should_connect {
        match reg.connect_by_name("Discord").await {
            Ok(_) => {
                tracing::info!("Discord auto-connected after config save");
                connect_results.push("discord: connected");
            }
            Err(e) => {
                tracing::error!(error = %e, "Discord auto-connect failed");
                connect_results.push("discord: failed");
            }
        }
    }

    if telegram_should_connect {
        match reg.connect_by_name("Telegram").await {
            Ok(_) => {
                tracing::info!("Telegram auto-connected after config save");
                connect_results.push("telegram: connected");
            }
            Err(e) => {
                tracing::error!(error = %e, "Telegram auto-connect failed");
                connect_results.push("telegram: failed");
            }
        }
    }

    Json(serde_json::json!({
        "success": true,
        "message": "Platform config updated",
        "connections": connect_results,
    }))
}

/// POST /api/chat/platform — ingest a message from a platform adapter.
/// Full inference pipeline: context assembly, tool execution, observer audit, response delivery.
/// Matches WebUI pipeline per governance — no simplified paths.
pub async fn platform_ingest(
    State(state): State<AppState>,
    Json(msg): Json<crate::platform::adapter::PlatformMessage>,
) -> Json<serde_json::Value> {
    tracing::info!(
        platform = %msg.platform,
        user = %msg.user_name,
        is_admin = msg.is_admin,
        content_len = msg.content.len(),
        "Platform message ingested"
    );

    // ── Session scoping: platform + user_id + channel_id ──
    let session_id = format!("{}_{}_{}", msg.platform, msg.user_id, msg.channel_id);

    // Auto-create session if it doesn't exist
    {
        let mut sessions = state.sessions.write().await;
        if sessions.get(&session_id).is_none() {
            let mut session = crate::session::Session::new();
            session.id = session_id.clone();
            session.title = format!("{} — {}", msg.platform, msg.user_name);
            if let Err(e) = sessions.update(&session) {
                tracing::warn!(error = %e, "Failed to persist new platform session");
            }
            // Insert directly since create() generates a new ID
            sessions.list(); // ensure loaded
        }
    }

    // ── Full context assembly (same as WebUI) ──
    let ctx = crate::web::ws_context::build_chat_context(
        &state, &msg.content, &session_id, None, Vec::new(), &msg.platform,
    ).await;
    let mut messages = ctx.messages;

    // ── Tool selection: admin gets full L1, non-admin gets safe tier ──
    let tools = if msg.is_admin {
        crate::tools::schema::layer1_tools()
    } else {
        crate::tools::schema::platform_safe_tools()
    };

    let provider = state.provider.as_ref();
    let thinking = state.config.prompt.thinking_enabled;

    // ── L1 inference ──
    let rx = match provider.chat(&messages, Some(&tools), thinking).await {
        Ok(rx) => rx,
        Err(e) => {
            tracing::error!(error = %e, platform = %msg.platform, "Platform inference failed");
            return Json(serde_json::json!({
                "success": false,
                "error": e.to_string(),
            }));
        }
    };

    // Consume stream (no WS sender — collect silently)
    let result = crate::inference::fast_reply::consume_stream(rx, None).await;

    let mut thinking_content: Option<String> = None;

    let response = match result {
        Ok(crate::inference::fast_reply::FastReplyResult::Reply { text, thinking }) => {
            thinking_content = thinking;

            // ── Observer audit (same as WebUI) ──
            let audited_text = platform_observer_audit(
                &state, provider, &mut messages, &tools, &msg.content, &text, &session_id,
            ).await;

            // Ingest assistant turn into session + timeline
            crate::web::ws_learning::ingest_assistant_turn(&state, &audited_text, &session_id).await;
            crate::web::ws_learning::spawn_insight_extraction(&state, &msg.content, &audited_text);

            audited_text
        }
        Ok(crate::inference::fast_reply::FastReplyResult::Escalate { objective, plan, .. }) => {
            if !msg.is_admin {
                "I can't perform complex multi-step tasks for non-admin users. Please ask an admin.".to_string()
            } else {
                // Run L2 ReAct loop synchronously, collect final reply
                let final_reply = run_platform_react(
                    &state, provider, messages, &objective, plan.as_deref(), &msg.content, &session_id,
                ).await;
                crate::web::ws_learning::ingest_assistant_turn(&state, &final_reply, &session_id).await;
                final_reply
            }
        }
        Ok(crate::inference::fast_reply::FastReplyResult::ToolCall { id, name, arguments }) => {
            // Run L1 tool chain synchronously, collect final reply
            let final_reply = run_platform_tool_chain(
                &state, provider, &mut messages, &tools, &msg.content, &session_id,
                crate::tools::schema::ToolCall { id, name, arguments },
            ).await;
            crate::web::ws_learning::ingest_assistant_turn(&state, &final_reply, &session_id).await;
            final_reply
        }
        Err(e) => {
            tracing::error!(error = %e, platform = %msg.platform, "Stream consumption failed");
            format!("An error occurred: {}", e)
        }
    };

    Json(serde_json::json!({
        "success": true,
        "response": response,
        "thinking": thinking_content,
        "session_id": session_id,
        "platform": msg.platform,
        "channel_id": msg.channel_id,
        "message_id": msg.message_id,
    }))
}

/// Observer audit for platform responses — mirrors WebUI audit_and_retry logic
/// but without WebSocket streaming.
async fn platform_observer_audit(
    state: &AppState,
    provider: &dyn crate::provider::Provider,
    messages: &mut Vec<crate::provider::Message>,
    tools: &serde_json::Value,
    user_query: &str,
    initial_text: &str,
    session_id: &str,
) -> String {
    if !state.config.observer.enabled || initial_text.is_empty() {
        return initial_text.to_string();
    }

    let max_retries = 2;
    let mut current_text = initial_text.to_string();
    let tool_context = build_platform_tool_context(messages);

    for attempt in 0..=max_retries {
        match crate::observer::audit_response(
            provider, messages, &current_text, &tool_context, user_query,
        ).await {
            Ok(output) if output.result.verdict.is_allowed() => {
                crate::web::training_capture::capture_approved(state, user_query, &current_text, output.result.confidence);
                crate::web::ws_stream::save_conversation_stack(state, session_id, &output.result);
                return current_text;
            }
            Ok(output) => {
                let rejected_text = current_text.clone();
                let reject_reason = output.result.what_went_wrong.clone();
                let category = output.result.failure_category.clone();

                tracing::info!(
                    attempt,
                    category = %category,
                    reason = %reject_reason,
                    "Platform response rejected by observer — retrying"
                );

                if attempt >= max_retries {
                    tracing::warn!(rejections = attempt + 1, "Platform observer bailout");
                    let bailout = crate::observer::format_bailout_override(attempt + 1);
                    messages.push(crate::provider::Message::text("assistant", &rejected_text));
                    messages.push(crate::provider::Message::text("system", &bailout));

                    if let Ok(rx) = provider.chat(messages, Some(tools), true).await {
                        if let Ok(crate::inference::fast_reply::FastReplyResult::Reply { text, .. }) =
                            crate::inference::fast_reply::consume_stream(rx, None).await
                        {
                            current_text = text;
                        }
                    }
                    break;
                }

                let feedback = crate::observer::format_rejection_feedback(&output.result);
                messages.push(crate::provider::Message::text("assistant", &rejected_text));
                messages.push(crate::provider::Message::text("system", &feedback));

                if let Ok(rx) = provider.chat(messages, Some(tools), true).await {
                    if let Ok(crate::inference::fast_reply::FastReplyResult::Reply { text, .. }) =
                        crate::inference::fast_reply::consume_stream(rx, None).await
                    {
                        crate::web::training_capture::capture_rejection(
                            state, user_query, &rejected_text, &text, &reject_reason,
                        );
                        current_text = text;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Platform observer failed — fail-open");
                return current_text;
            }
        }
    }

    current_text
}

/// Run L1 tool chain synchronously for platform messages.
/// Same logic as ws_l1::run_l1_tool_chain but collects text instead of streaming to WS.
async fn run_platform_tool_chain(
    state: &AppState,
    provider: &dyn crate::provider::Provider,
    messages: &mut Vec<crate::provider::Message>,
    tools: &serde_json::Value,
    user_query: &str,
    session_id: &str,
    first_tc: crate::tools::schema::ToolCall,
) -> String {
    let max_iterations = 50;
    let mut current_tc = first_tc;

    for iteration in 0..max_iterations {
        tracing::info!(tool = %current_tc.name, iteration, "Platform L1 tool execution");

        let result = crate::web::tool_dispatch::execute_tool_with_state(state, &current_tc).await;
        tracing::info!(tool = %current_tc.name, success = result.success, output_len = result.output.len(), "Platform tool complete");

        messages.push(crate::provider::Message::assistant_tool_call(
            &current_tc.id, &current_tc.name, &current_tc.arguments,
        ));
        messages.push(crate::provider::Message::tool_result(&current_tc.id, &result.output));

        // Re-inference with tool results
        let rx_next = match provider.chat(messages, Some(tools), true).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::error!(error = %e, "Platform L1 re-inference failed");
                return format!("Tool execution error: {}", e);
            }
        };

        match crate::inference::fast_reply::consume_stream(rx_next, None).await {
            Ok(crate::inference::fast_reply::FastReplyResult::Reply { text, .. }) => {
                // Audit the final reply
                let audited = platform_observer_audit(
                    state, provider, messages, tools, user_query, &text, session_id,
                ).await;
                return audited;
            }
            Ok(crate::inference::fast_reply::FastReplyResult::ToolCall { id, name, arguments }) => {
                current_tc = crate::tools::schema::ToolCall { id, name, arguments };
            }
            Ok(crate::inference::fast_reply::FastReplyResult::Escalate { objective, plan, .. }) => {
                // L1 chain escalated to L2
                let final_reply = run_platform_react(
                    state, provider, messages.clone(), &objective, plan.as_deref(), user_query, session_id,
                ).await;
                return final_reply;
            }
            Err(e) => {
                tracing::error!(error = %e, "Platform L1 stream error");
                return format!("Error: {}", e);
            }
        }
    }

    "Tool chain reached maximum iterations.".to_string()
}

/// Run L2 ReAct loop synchronously for platform messages.
/// Same loop as ws_react but collects the final reply_request text instead of streaming.
async fn run_platform_react(
    state: &AppState,
    provider: &dyn crate::provider::Provider,
    mut messages: Vec<crate::provider::Message>,
    objective: &str,
    plan: Option<&str>,
    user_query: &str,
    session_id: &str,
) -> String {
    let tools = if true { // Platform ReAct always uses L2 tools for admin (caller already checked)
        crate::tools::schema::layer2_tools()
    } else {
        crate::tools::schema::platform_safe_tools()
    };

    // Inject ReAct system message
    let react_instruction = format!(
        "You are now in ReAct (Reason-Act-Observe) mode.\n\
         Objective: {}\n{}\n\
         Use tools to accomplish the objective. When done, call `reply_request` with your final response.",
        objective,
        plan.map(|p| format!("Plan:\n{}", p)).unwrap_or_default(),
    );
    messages.push(crate::provider::Message::text("system", &react_instruction));

    let max_turns = 50;
    for turn in 0..max_turns {
        let rx = match provider.chat(&messages, Some(&tools), true).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::error!(error = %e, turn, "Platform ReAct inference failed");
                return format!("ReAct error: {}", e);
            }
        };

        match crate::inference::fast_reply::consume_stream(rx, None).await {
            Ok(crate::inference::fast_reply::FastReplyResult::Reply { text, .. }) => {
                // Model replied without using reply_request — treat as final
                let audited = platform_observer_audit(
                    state, provider, &mut messages, &tools, user_query, &text, session_id,
                ).await;
                return audited;
            }
            Ok(crate::inference::fast_reply::FastReplyResult::ToolCall { id, name, arguments }) => {
                let tc = crate::tools::schema::ToolCall { id: id.clone(), name: name.clone(), arguments: arguments.clone() };

                // Check for reply_request (loop terminator)
                if let Some(reply_text) = crate::tools::schema::extract_reply_text(&tc) {
                    let audited = platform_observer_audit(
                        state, provider, &mut messages, &tools, user_query, &reply_text, session_id,
                    ).await;
                    crate::web::ws_learning::ingest_assistant_turn(state, &audited, session_id).await;
                    return audited;
                }

                // Execute tool
                tracing::info!(tool = %name, turn, "Platform ReAct tool execution");
                let result = crate::web::tool_dispatch::execute_tool_with_state(state, &tc).await;
                messages.push(crate::provider::Message::assistant_tool_call(&id, &name, &arguments));
                messages.push(crate::provider::Message::tool_result(&id, &result.output));
            }
            Ok(crate::inference::fast_reply::FastReplyResult::Escalate { objective: nested_obj, .. }) => {
                // Nested escalation within ReAct — inject as additional objective
                tracing::info!(nested = %nested_obj, turn, "Platform ReAct nested escalation — continuing loop");
                messages.push(crate::provider::Message::text("system",
                    &format!("Additional objective: {}", nested_obj),
                ));
            }
            Err(e) => {
                tracing::error!(error = %e, turn, "Platform ReAct stream error");
                return format!("ReAct error: {}", e);
            }
        }
    }

    "ReAct loop reached maximum turns.".to_string()
}

/// Build tool context summary for observer audit (same logic as ws_stream).
fn build_platform_tool_context(messages: &[crate::provider::Message]) -> String {
    let mut entries: Vec<String> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == "tool" {
            let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
            let result_text = msg.text_content();

            let mut tool_name = "unknown".to_string();
            for j in (0..i).rev() {
                if messages[j].role == "assistant" {
                    if let Some(tcs) = &messages[j].tool_calls {
                        for tc in tcs {
                            if tc["id"].as_str() == Some(tool_call_id) {
                                tool_name = tc["function"]["name"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string();
                                break;
                            }
                        }
                        if tool_name != "unknown" { break; }
                    }
                }
            }
            entries.push(format!("[{}] {} → {}", entries.len() + 1, tool_name, result_text));
        }
    }
    if entries.is_empty() {
        String::new()
    } else {
        format!("Platform tools executed ({} calls):\n{}", entries.len(), entries.join("\n"))
    }
}

/// Mask a token for display — show first 8 chars, mask the rest.
fn mask_token(token: &str) -> String {
    if token.len() <= 8 {
        return "****".to_string();
    }
    format!("{}****", &token[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_token_long() {
        let masked = mask_token("abcdefghijklmnop");
        assert_eq!(masked, "abcdefgh****");
    }

    #[test]
    fn test_mask_token_short() {
        let masked = mask_token("abc");
        assert_eq!(masked, "****");
    }

    #[test]
    fn test_session_id_scoping() {
        let session_id = format!("{}_{}_{}", "discord", "user123", "channel456");
        assert_eq!(session_id, "discord_user123_channel456");
    }
}
