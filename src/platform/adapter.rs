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

/// Platform-neutral interactive component (button) definition.
#[derive(Debug, Clone)]
pub struct MessageComponent {
    pub custom_id: String,
    pub label: String,
    pub emoji: String,
    pub style: ComponentStyle,
}

/// Button style for platform-neutral component definitions.
#[derive(Debug, Clone, Copy)]
pub enum ComponentStyle {
    Primary,
    Secondary,
    Success,
    Danger,
}

/// An interaction event from a platform (e.g., button click).
#[derive(Debug, Clone)]
pub struct PlatformInteraction {
    pub platform: String,
    pub action: String,
    pub session_id: String,
    pub message_index: usize,
    pub user_id: String,
    pub channel_id: String,
    /// Opaque token for deferred interaction responses.
    pub interaction_token: String,
    /// Interaction ID for response routing.
    pub interaction_id: String,
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

    /// Archive a thinking thread after inference completes.
    /// Archival preserves the thread for audit trail visibility.
    async fn archive_thread(&self, thread_id: &str) -> Result<()> {
        let _ = thread_id;
        Ok(())
    }

    /// Reply to a message with interactive button components.
    /// Returns the sent message's ID for later reference.
    async fn reply_with_components(
        &self, channel_id: &str, message_id: &str, content: &str,
        components: &[MessageComponent],
    ) -> Result<String> {
        let _ = components;
        self.reply_to_message(channel_id, message_id, content).await?;
        Ok(String::new())
    }

    /// Delete a message by ID.
    async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let _ = (channel_id, message_id);
        Ok(())
    }

    /// Take the receiver for interaction events (button clicks).
    fn take_interaction_receiver(&mut self) -> Option<mpsc::Receiver<PlatformInteraction>> {
        None
    }

    /// Send an audio file as an attachment to a channel.
    async fn send_audio_file(
        &self, channel_id: &str, audio_bytes: Vec<u8>, filename: &str,
    ) -> Result<()> {
        let _ = (channel_id, audio_bytes, filename);
        Ok(())
    }

    /// Send an image file as an attachment to a channel, with optional caption text.
    async fn send_image_file(
        &self, channel_id: &str, message_id: &str, image_bytes: Vec<u8>,
        filename: &str, caption: &str,
    ) -> Result<()> {
        let _ = (channel_id, message_id, image_bytes, filename, caption);
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
