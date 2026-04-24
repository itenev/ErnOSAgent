// Ern-OS — Platform adapter trait (ported from ErnOSAgent)
// Created by @mettamazza (github.com/mettamazza)
// License: MIT
//! Platform adapter trait — unified interface for chat platforms.
//! Each adapter connects as a WebSocket client to the Ern-OS WebUI hub.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Events from external platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMessage {
    pub platform: String,
    pub channel_id: String,
    pub user_id: String,
    pub user_name: String,
    pub content: String,
    pub attachments: Vec<String>,
    /// Original message ID for native reply threading.
    pub message_id: String,
    /// Whether this user is the admin (full tool access).
    pub is_admin: bool,
}

/// Status of a platform connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformStatus {
    pub name: String,
    pub connected: bool,
    pub error: Option<String>,
}

/// Unified interface for all chat platform adapters.
///
/// Each platform (Discord, Telegram, etc.) implements this trait.
/// In Ern-OS, adapters act as WebSocket clients connecting to the hub.
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Human-readable name of the platform.
    fn name(&self) -> &str;

    /// Whether the adapter has valid credentials configured.
    fn is_configured(&self) -> bool;

    /// Connect to the platform. Spawns background tasks.
    async fn connect(&mut self) -> Result<()>;

    /// Disconnect from the platform.
    async fn disconnect(&mut self) -> Result<()>;

    /// Send a message to a specific channel/chat.
    async fn send_message(&self, channel_id: &str, content: &str) -> Result<()>;

    /// Reply to a specific message (native threading).
    async fn reply_to_message(
        &self, channel_id: &str, message_id: &str, content: &str,
    ) -> Result<()> {
        let _ = message_id;
        self.send_message(channel_id, content).await
    }

    /// Send a typing indicator to a channel. Returns immediately.
    /// Discord typing lasts ~10 seconds; call repeatedly for longer inference.
    async fn start_typing(&self, channel_id: &str) -> Result<()> {
        let _ = channel_id;
        Ok(())
    }

    /// Create a temporary thinking thread for CoT visibility.
    /// Returns the thread/channel ID on success.
    async fn create_thinking_thread(&self, channel_id: &str, message_id: &str, title: &str) -> Result<String> {
        let _ = (channel_id, message_id, title);
        anyhow::bail!("Thinking threads not supported on this platform")
    }

    /// Send a message to a thinking thread.
    async fn send_to_thread(&self, thread_id: &str, content: &str) -> Result<()> {
        let _ = (thread_id, content);
        Ok(())
    }

    /// Delete a thinking thread after inference completes.
    async fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let _ = thread_id;
        Ok(())
    }

    /// Take the receiver end of the incoming message channel.
    fn take_message_receiver(&mut self) -> Option<mpsc::Receiver<PlatformMessage>>;

    /// Current connection status.
    fn status(&self) -> PlatformStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_message_fields() {
        let msg = PlatformMessage {
            platform: "discord".to_string(),
            channel_id: "123".to_string(),
            user_id: "456".to_string(),
            user_name: "user".to_string(),
            content: "hello".to_string(),
            attachments: Vec::new(),
            message_id: "789".to_string(),
            is_admin: false,
        };
        assert_eq!(msg.platform, "discord");
    }

    #[test]
    fn test_platform_status() {
        let status = PlatformStatus {
            name: "Discord".to_string(),
            connected: true,
            error: None,
        };
        assert!(status.connected);
    }
}
