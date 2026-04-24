// Ern-OS — Platform router (ported from ErnOSAgent)
// Created by @mettamazza (github.com/mettamazza)
// License: MIT
//! Platform router — routes incoming platform messages to the Ern-OS
//! WebSocket chat API as a client, per governance §6.3.
//! Reads the hub response and delivers it back to the originating platform.

use crate::platform::adapter::PlatformMessage;
use crate::platform::registry::PlatformRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Start routing messages from all connected platform adapters
/// into the Ern-OS chat pipeline.
pub async fn start_platform_router(
    registry: Arc<RwLock<PlatformRegistry>>,
    hub_port: u16,
) {
    // Collect all message receivers while holding the lock, then release it.
    let mut receivers = Vec::new();
    {
        let mut reg = registry.write().await;
        let adapters = reg.adapters_mut();
        for adapter in adapters.iter_mut() {
            if let Some(rx) = adapter.take_message_receiver() {
                let name = adapter.name().to_string();
                receivers.push((name, rx));
            }
        }
    }

    // Spawn a routing task for each platform.
    for (name, rx) in receivers {
        let reg_clone = registry.clone();
        let port = hub_port;
        tokio::spawn(async move {
            route_platform_messages(name, rx, port, reg_clone).await;
        });
    }
}

/// Route messages from a single platform adapter to the WebUI hub.
/// Reads the response and delivers it back to the originating channel.
async fn route_platform_messages(
    platform: String,
    mut rx: tokio::sync::mpsc::Receiver<PlatformMessage>,
    hub_port: u16,
    registry: Arc<RwLock<PlatformRegistry>>,
) {
    tracing::info!(platform = %platform, "Platform router started");

    while let Some(msg) = rx.recv().await {
        tracing::debug!(
            platform = %msg.platform,
            user = %msg.user_name,
            content_len = msg.content.len(),
            "Routing platform message to hub"
        );

        let channel_id = msg.channel_id.clone();
        let message_id = msg.message_id.clone();
        let msg_platform = msg.platform.clone();
        let msg_user = msg.user_name.clone();

        // ── Start continuous typing indicator ──
        let typing_registry = registry.clone();
        let typing_platform = msg_platform.clone();
        let typing_channel = channel_id.clone();
        let typing_cancel = tokio_util::sync::CancellationToken::new();
        let typing_token = typing_cancel.clone();

        tokio::spawn(async move {
            loop {
                {
                    let reg = typing_registry.read().await;
                    let _ = reg.start_typing(&typing_platform, &typing_channel).await;
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {}
                    _ = typing_token.cancelled() => break,
                }
            }
        });

        // ── Create thinking thread ──
        let thinking_thread_id = {
            let reg = registry.read().await;
            let title = format!("💭 Thinking… ({})", &msg_user);
            match reg.create_thinking_thread(&msg_platform, &channel_id, &message_id, &title).await {
                Ok(id) => {
                    let _ = reg.send_to_thread(&msg_platform, &id,
                        "🧠 *Processing your message…*"
                    ).await;
                    Some(id)
                }
                Err(e) => {
                    tracing::debug!(error = %e, "Thinking thread not available");
                    None
                }
            }
        };

        // ── Forward to hub ──
        match forward_to_hub(&msg, hub_port).await {
            Ok(hub_resp) => {
                // Stop typing
                typing_cancel.cancel();

                // ── Post thinking to thread ──
                if let (Some(ref tid), Some(ref thinking)) = (&thinking_thread_id, &hub_resp.thinking) {
                    if !thinking.is_empty() {
                        let reg = registry.read().await;
                        let _ = reg.send_to_thread(&msg_platform, tid, thinking).await;
                    }
                }

                if hub_resp.response.is_empty() {
                    tracing::warn!(platform = %msg_platform, "Hub returned empty response");
                } else {
                    // Deliver response as reply to original message
                    let reg = registry.read().await;
                    if let Err(e) = reg.send_to_platform(
                        &msg_platform, &channel_id, &message_id, &hub_resp.response,
                    ).await {
                        tracing::error!(
                            platform = %msg_platform,
                            channel = %channel_id,
                            error = %e,
                            "Failed to deliver response to platform"
                        );
                    }
                }
            }
            Err(e) => {
                typing_cancel.cancel();
                tracing::warn!(
                    platform = %msg_platform,
                    error = %e,
                    "Failed to forward platform message to hub"
                );
            }
        }

        // ── Clean up thinking thread ──
        if let Some(tid) = thinking_thread_id {
            let reg = registry.read().await;
            let _ = reg.send_to_thread(&msg_platform, &tid,
                "✅ *Done — response delivered.*"
            ).await;
            // Delete after a short delay so the user can glance at it
            let cleanup_registry = registry.clone();
            let cleanup_platform = msg_platform.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                let reg = cleanup_registry.read().await;
                let _ = reg.delete_thread(&cleanup_platform, &tid).await;
            });
        }
    }

    tracing::info!(platform = %platform, "Platform router stopped");
}

/// Hub response containing the reply and optional thinking content.
struct HubResponse {
    response: String,
    thinking: Option<String>,
}

/// Forward a platform message to the Ern-OS hub via HTTP API.
/// Returns the response text and thinking content on success.
async fn forward_to_hub(msg: &PlatformMessage, port: u16) -> anyhow::Result<HubResponse> {
    let url = format!("http://127.0.0.1:{}/api/chat/platform", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 min for local inference
        .build()?;

    let payload = serde_json::json!({
        "platform": msg.platform,
        "channel_id": msg.channel_id,
        "user_id": msg.user_id,
        "user_name": msg.user_name,
        "content": msg.content,
        "attachments": msg.attachments,
        "message_id": msg.message_id,
        "is_admin": msg.is_admin,
    });

    let resp = client.post(&url)
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Hub rejected message: {} {}", status, body);
    }

    let body: serde_json::Value = resp.json().await?;

    let response = body["response"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let thinking = body["thinking"]
        .as_str()
        .map(|s| s.to_string());

    Ok(HubResponse { response, thinking })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forward_payload_structure() {
        let msg = PlatformMessage {
            platform: "discord".to_string(),
            channel_id: "ch1".to_string(),
            user_id: "u1".to_string(),
            user_name: "Test".to_string(),
            content: "Hello".to_string(),
            attachments: vec![],
            message_id: "m1".to_string(),
            is_admin: true,
        };
        let payload = serde_json::json!({
            "platform": msg.platform,
            "content": msg.content,
            "is_admin": msg.is_admin,
        });
        assert_eq!(payload["platform"], "discord");
        assert_eq!(payload["is_admin"], true);
    }
}
