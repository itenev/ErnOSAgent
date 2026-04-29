//! Thread-posting helpers for the platform router.
//! Formats and delivers tool events, audit summaries, plan content,
//! and error messages to Discord thinking threads.

use crate::platform::registry::PlatformRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Format and post tool execution events to the thinking thread.
pub(crate) async fn post_tool_events(
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str,
    thread_id: &str,
    tool_events: &[serde_json::Value],
) {
    if tool_events.is_empty() {
        return;
    }

    let mut lines: Vec<String> = Vec::new();
    for event in tool_events {
        let name = event["name"].as_str().unwrap_or("unknown");
        let success = event["success"].as_bool().unwrap_or(false);
        let elapsed = event["elapsed_ms"].as_u64().unwrap_or(0);
        let preview = event["output_preview"].as_str().unwrap_or("");

        let icon = if success { "✅" } else { "❌" };
        let elapsed_fmt = format_elapsed(elapsed);

        // Truncate preview for Discord readability
        let short_preview = if preview.len() > 120 {
            let b = preview.char_indices().take_while(|(i,_)| *i <= 120).last().map(|(i,_)| i).unwrap_or(0);
            format!("{}…", &preview[..b])
        } else {
            preview.to_string()
        };

        lines.push(format!("🔧 **{}** — {} {} ({})", name, icon, if success { "success" } else { "failed" }, elapsed_fmt));
        if !short_preview.is_empty() {
            lines.push(format!("└ {}", short_preview));
        }
    }

    let message = lines.join("\n");
    let reg = registry.read().await;
    let _ = reg.send_to_thread(platform, thread_id, &message).await;
}

/// Format and post audit summary to the thinking thread.
pub(crate) async fn post_audit_summary(
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str,
    thread_id: &str,
    audit: &Option<serde_json::Value>,
) {
    let Some(audit) = audit else { return };

    let verdict = audit["verdict"].as_str().unwrap_or("Unknown");
    let confidence = audit["confidence"].as_f64().unwrap_or(0.0);
    let retries = audit["retries"].as_u64().unwrap_or(0);
    let topic = audit["active_topic"].as_str().unwrap_or("");

    let icon = if verdict == "Allowed" { "✅" } else { "⚠️" };
    let mut message = format!("📋 **Observer Audit** — {} {} ({:.1} confidence)", icon, verdict, confidence);
    if retries > 0 {
        message.push_str(&format!(" [{} retries]", retries));
    }
    if !topic.is_empty() {
        message.push_str(&format!("\n📍 Topic: {}", topic));
    }

    let reg = registry.read().await;
    let _ = reg.send_to_thread(platform, thread_id, &message).await;
}

/// Post plan markdown content to the thinking thread for user review.
/// Splits into 1900-char chunks to respect Discord's 2000-char message limit.
pub(crate) async fn post_plan_to_thread(
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str,
    thread_id: &str,
    plan_markdown: &str,
) {
    let reg = registry.read().await;
    let _ = reg.send_to_thread(platform, thread_id, "📋 **Implementation Plan:**").await;

    // Split plan into Discord-safe chunks (1900 chars to leave room for formatting)
    for chunk in split_at_boundary(plan_markdown, 1900) {
        let _ = reg.send_to_thread(platform, thread_id, &chunk).await;
    }
}

/// Post an error message to the thinking thread on hub failure.
pub(crate) async fn finalize_thread_on_error(
    registry: &Arc<RwLock<PlatformRegistry>>,
    platform: &str,
    thinking_thread_id: &Option<String>,
    error: &anyhow::Error,
) {
    if let Some(tid) = thinking_thread_id {
        let reg = registry.read().await;
        let _ = reg.send_to_thread(platform, tid,
            &format!("❌ *Error: {}*", error),
        ).await;
    }
}

/// Split a string at char boundaries near `max_len`, preferring newline splits.
pub(crate) fn split_at_boundary(text: &str, max_len: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        // Find the last newline before max_len
        let boundary = remaining[..max_len].rfind('\n').unwrap_or(max_len);
        let boundary = if boundary == 0 { max_len } else { boundary };
        chunks.push(remaining[..boundary].to_string());
        remaining = &remaining[boundary..].trim_start_matches('\n');
    }
    chunks
}

/// Format milliseconds into a human-readable duration.
pub(crate) fn format_elapsed(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
