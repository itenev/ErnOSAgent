//! ReAct loop helper utilities — sub-agent execution, tool context building,
//! and shared helpers used by the main ReAct loop handler in `ws_react.rs`.

use crate::provider::Message;
use crate::tools::schema;
use crate::web::state::AppState;
use crate::web::ws_stream::send_ws;
use axum::extract::ws::{Message as WsMessage, WebSocket};

/// Execute a spawn_sub_agent tool call — runs an isolated ReAct loop.
pub async fn execute_sub_agent(
    state: &AppState,
    provider: &dyn crate::provider::Provider,
    tc: &schema::ToolCall,
    sender: &mut futures_util::stream::SplitSink<WebSocket, WsMessage>,
) -> schema::ToolResult {
    let args = tc.args();
    let task = args["task"].as_str().unwrap_or("").to_string();
    let tools: Vec<String> = args["tools"].as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let max_turns = args["max_turns"].as_u64().unwrap_or(5) as usize;

    if task.is_empty() || tools.is_empty() {
        return schema::ToolResult {
            tool_call_id: tc.id.clone(),
            name: tc.name.clone(),
            output: "Error: sub-agent requires non-empty 'task' and 'tools' array".to_string(),
            success: false,
            images: Vec::new(),
        };
    }

    send_ws(sender, "status", &serde_json::json!({
        "message": format!("Sub-agent spawned: {} (tools: {}, max {} turns)", task, tools.join(", "), max_turns)
    })).await;

    let config = crate::inference::sub_agent::SubAgentConfig {
        task: task.clone(),
        allowed_tools: tools,
        max_turns,
    };

    match crate::inference::sub_agent::run_sub_agent(provider, config, state).await {
        Ok(result) => {
            tracing::info!(
                task = %task,
                success = result.success,
                turns = result.turns_used,
                tools_called = %result.tool_calls_made.join(", "),
                "Sub-agent completed"
            );
            schema::ToolResult {
                tool_call_id: tc.id.clone(),
                name: tc.name.clone(),
                output: format!(
                    "[Sub-Agent Result]\nSuccess: {}\nTurns: {}\nTools: {}\n\n{}",
                    result.success, result.turns_used,
                    result.tool_calls_made.join(", "),
                    result.summary
                ),
                success: result.success,
                images: Vec::new(),
            }
        }
        Err(e) => {
            tracing::error!(error = %e, task = %task, "Sub-agent failed");
            schema::ToolResult {
                tool_call_id: tc.id.clone(),
                name: tc.name.clone(),
                output: format!("Sub-agent error: {}", e),
                success: false,
                images: Vec::new(),
            }
        }
    }
}

/// Build a concise tool context string from the message history for the observer.
pub fn build_tool_context(messages: &[Message]) -> String {
    let mut entries: Vec<String> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == "tool" {
            let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
            let result_text = msg.text_content();
            let result_preview = truncate_preview(&result_text, 200);
            let tool_name = find_tool_name(messages, i, tool_call_id);
            entries.push(format!("[{}] {} → {}", entries.len() + 1, tool_name, result_preview));
        }
    }

    if entries.is_empty() {
        String::new()
    } else {
        format!("Tools executed this session ({} calls):\n{}", entries.len(), entries.join("\n"))
    }
}

/// Truncate a string to a preview length at a character boundary.
fn truncate_preview(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        let boundary = text.char_indices()
            .take_while(|(i, _)| *i <= max_chars)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        format!("{}...", &text[..boundary])
    }
}

/// Find the tool name for a given tool_call_id by scanning preceding messages.
fn find_tool_name(messages: &[Message], current_idx: usize, tool_call_id: &str) -> String {
    for j in (0..current_idx).rev() {
        if messages[j].role == "assistant" {
            if let Some(tcs) = &messages[j].tool_calls {
                for tc in tcs {
                    if tc["id"].as_str() == Some(tool_call_id) {
                        return tc["function"]["name"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string();
                    }
                }
            }
        }
    }
    "unknown".to_string()
}

/// Background skill synthesis after ReAct loop completion.
/// Extracts reusable procedural skills from the tool execution history.
pub fn spawn_skill_synthesis(state: &AppState, user_query: &str) {
    let provider = state.provider.clone();
    let memory = state.memory.clone();
    let query = user_query.to_string();

    tokio::spawn(async move {
        // Collect recent tool usage from memory for synthesis
        let tool_history: Vec<(String, String)> = {
            let mem = memory.read().await;
            mem.procedures.recent_tool_usage(10)
        };

        if !crate::observer::skills::is_skill_worthy(tool_history.len()) {
            return;
        }

        match crate::observer::skills::synthesise_skill(provider.as_ref(), &query, &tool_history).await {
            Ok(Some(skill)) => {
                tracing::info!(skill = %skill.name, confidence = skill.confidence, "Skill synthesised");
                let mut mem = memory.write().await;
                let _ = mem.procedures.record_skill(&skill.name, &skill.description);
            }
            Ok(None) => tracing::debug!("No reusable skill extracted"),
            Err(e) => tracing::warn!(error = %e, "Skill synthesis failed"),
        }
    });
}
