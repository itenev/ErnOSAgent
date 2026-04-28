// Ern-OS — Platform router SSE streaming layer
//! Handles SSE-based streaming from the hub's `/api/chat/platform/stream`
//! endpoint, posting thinking deltas and tool events to the Discord
//! thinking thread in real-time.

use crate::platform::adapter::PlatformMessage;
use crate::platform::registry::PlatformRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Stream-forward a platform message via the SSE endpoint, posting thinking
/// deltas and tool events to the Discord thread in real-time.
pub(crate) async fn forward_to_hub_streaming(
    msg: &PlatformMessage,
    port: u16,
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str,
    thinking_thread_id: &Option<String>,
) -> anyhow::Result<super::router::HubResponse> {
    let url = format!("http://127.0.0.1:{}/api/chat/platform/stream", port);
    let client = reqwest::Client::builder()
        .read_timeout(std::time::Duration::from_secs(300))
        .build()?;

    let payload = build_platform_payload(msg);

    let resp = match client.post(&url).json(&payload).send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::debug!(
                error = %e, url = %url,
                "SSE stream endpoint connection failed, falling back to blocking"
            );
            return super::router::forward_to_hub(msg, port).await;
        }
    };

    if !resp.status().is_success() {
        tracing::debug!(
            status = %resp.status(), url = %url,
            "SSE stream endpoint returned non-success, falling back to blocking"
        );
        return super::router::forward_to_hub(msg, port).await;
    }

    tracing::debug!(url = %url, "SSE stream connected — consuming events");
    parse_sse_stream(resp, registry, platform, thinking_thread_id).await
}

/// Parse an SSE response stream, posting events to the thinking thread live.
async fn parse_sse_stream(
    resp: reqwest::Response,
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str,
    thinking_thread_id: &Option<String>,
) -> anyhow::Result<super::router::HubResponse> {
    let mut response = String::new();
    let mut thinking_buf = String::new();
    let mut tool_events: Vec<serde_json::Value> = Vec::new();
    let mut session_id = None;
    let mut has_plan = false;
    let mut plan_markdown = None;

    let mut stream = resp.bytes_stream();
    let mut line_buf = String::new();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(_) => break,
        };
        line_buf.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(nl) = line_buf.find('\n') {
            let line = line_buf[..nl].trim().to_string();
            line_buf = line_buf[nl + 1..].to_string();

            if line.starts_with("data:") {
                let data = line[5..].trim();
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                    handle_sse_event(
                        &val, &mut response, &mut thinking_buf,
                        &mut tool_events, &mut session_id,
                        &mut has_plan, &mut plan_markdown,
                        registry, platform, thinking_thread_id,
                    ).await;
                }
            }
        }
    }

    Ok(super::router::HubResponse {
        response,
        thinking: if thinking_buf.is_empty() { None } else { Some(thinking_buf) },
        tool_events,
        audit: None,
        session_id,
        has_plan,
        plan_markdown,
    })
}

/// Handle a single SSE event: post to thinking thread and accumulate state.
async fn handle_sse_event(
    val: &serde_json::Value,
    response: &mut String, thinking: &mut String,
    tool_events: &mut Vec<serde_json::Value>,
    session_id: &mut Option<String>,
    has_plan: &mut bool, plan_markdown: &mut Option<String>,
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str, tid: &Option<String>,
) {
    if let Some(chunk) = val.get("chunk").and_then(|v| v.as_str()) {
        thinking.push_str(chunk);
        // Post thinking progress every ~200 chars
        if let Some(tid) = tid {
            if thinking.len() % 200 < chunk.len() {
                post_thinking_preview(registry, platform, tid, thinking).await;
            }
        }
    } else if val.get("name").is_some() && val.get("success").is_some() {
        tool_events.push(val.clone());
        if let Some(tid) = tid {
            post_tool_event_live(registry, platform, tid, val).await;
        }
    } else if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
        *response = text.to_string();
        if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
            *session_id = Some(sid.to_string());
        }
        if val.get("has_plan").and_then(|v| v.as_bool()).unwrap_or(false) {
            *has_plan = true;
        }
        if let Some(pm) = val.get("plan_markdown").and_then(|v| v.as_str()) {
            *plan_markdown = Some(pm.to_string());
        }
    }
}

/// Post a thinking preview to the thread (truncated to Discord's limit).
async fn post_thinking_preview(
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str, tid: &str, thinking: &str,
) {
    let reg = registry.read().await;
    let preview = if thinking.len() > 1900 {
        let b = thinking.char_indices().rev()
            .take_while(|(i, _)| *i > thinking.len() - 1900)
            .last().map(|(i, _)| i).unwrap_or(0);
        format!("…{}", &thinking[b..])
    } else {
        thinking.to_string()
    };
    let _ = reg.send_to_thread(platform, tid, &format!("💭 {}", preview)).await;
}

/// Post a live tool execution event to the thinking thread.
async fn post_tool_event_live(
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str, tid: &str, val: &serde_json::Value,
) {
    let name = val["name"].as_str().unwrap_or("?");
    let success = val["success"].as_bool().unwrap_or(false);
    let icon = if success { "✅" } else { "❌" };
    let elapsed = val["elapsed_ms"].as_u64().unwrap_or(0);
    let reg = registry.read().await;
    let _ = reg.send_to_thread(platform, tid,
        &format!("🔧 **{}** {} ({}ms)", name, icon, elapsed),
    ).await;
}

/// Build the JSON payload for hub forwarding.
pub(crate) fn build_platform_payload(msg: &PlatformMessage) -> serde_json::Value {
    serde_json::json!({
        "platform": msg.platform,
        "channel_id": msg.channel_id,
        "user_id": msg.user_id,
        "user_name": msg.user_name,
        "content": msg.content,
        "attachments": msg.attachments,
        "message_id": msg.message_id,
        "is_admin": msg.is_admin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_platform_payload() {
        let msg = PlatformMessage {
            platform: "discord".into(),
            channel_id: "ch1".into(),
            user_id: "u1".into(),
            user_name: "Test".into(),
            content: "Hello".into(),
            attachments: vec![],
            message_id: "m1".into(),
            is_admin: true,
        };
        let payload = build_platform_payload(&msg);
        assert_eq!(payload["platform"], "discord");
        assert_eq!(payload["is_admin"], true);
    }
}
