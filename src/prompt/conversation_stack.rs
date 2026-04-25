// Ern-OS — Conversation Stack
//! Tracks the semantic topic trajectory of a conversation.
//! Generated retroactively by the observer audit (piggybacked on the existing
//! per-turn chat_sync call — zero additional inference cost).
//!
//! The stack is injected into the HUD before every inference, giving the model
//! a "narrative compass" that prevents contextual drift.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A hierarchical semantic trace of the conversation's topic trajectory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationStack {
    /// The most granular, immediate focus of the current turn.
    pub active_topic: String,
    /// The reasoning that moved us from the previous topic to this one.
    pub transition: String,
    /// The broader thematic frame / ancestry.
    pub context: String,
    /// The last completed topic (for backtracking).
    pub previous_topic: String,
}

impl ConversationStack {
    /// Format the stack for injection into the HUD.
    pub fn to_hud_section(&self) -> String {
        if self.active_topic.is_empty() {
            return String::new();
        }

        let mut section = String::from("\n\n## Conversation Stack\n");
        section.push_str(&format!("[ACTIVE] {}\n", self.active_topic));
        if !self.transition.is_empty() {
            section.push_str(&format!("  ↳ Why: {}\n", self.transition));
        }
        if !self.context.is_empty() {
            section.push_str(&format!("[CONTEXT] {}\n", self.context));
        }
        if !self.previous_topic.is_empty() {
            section.push_str(&format!("[PREVIOUS] {}\n", self.previous_topic));
        }
        section
    }
}

/// Persistent store for per-session conversation stacks.
/// Each session gets its own stack file under `data/conversation_stacks/`.
pub struct ConversationStackStore {
    dir: PathBuf,
}

impl ConversationStackStore {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("conversation_stacks");
        std::fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    /// Load the conversation stack for a session.
    pub fn load(&self, session_id: &str) -> ConversationStack {
        let path = self.dir.join(format!("{}.json", sanitize_id(session_id)));
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => ConversationStack::default(),
        }
    }

    /// Save the conversation stack for a session.
    pub fn save(&self, session_id: &str, stack: &ConversationStack) -> Result<()> {
        let path = self.dir.join(format!("{}.json", sanitize_id(session_id)));
        let json = serde_json::to_string_pretty(stack)?;
        std::fs::write(&path, json)?;
        tracing::debug!(
            session = %session_id,
            active = %stack.active_topic,
            "Conversation stack saved"
        );
        Ok(())
    }

    /// Update the stack from observer audit output.
    /// Called after every observer audit — the observer generates topic fields
    /// as part of its standard verdict JSON (zero additional inference cost).
    pub fn update_from_audit(
        &self,
        session_id: &str,
        active_topic: &str,
        transition: &str,
        context: &str,
    ) -> Result<()> {
        let mut stack = self.load(session_id);

        // Rotate: current active becomes previous
        if !stack.active_topic.is_empty() && stack.active_topic != active_topic {
            stack.previous_topic = stack.active_topic.clone();
        }

        stack.active_topic = active_topic.to_string();
        stack.transition = transition.to_string();
        if !context.is_empty() {
            stack.context = context.to_string();
        }

        self.save(session_id, &stack)
    }
}

/// Sanitize a session ID for use as a filename.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_stack_no_hud() {
        let stack = ConversationStack::default();
        assert!(stack.to_hud_section().is_empty());
    }

    #[test]
    fn test_stack_hud_format() {
        let stack = ConversationStack {
            active_topic: "Observer false-flagging web search results".to_string(),
            transition: "User reported confabulation from truncated context".to_string(),
            context: "Discord platform adapter stabilization".to_string(),
            previous_topic: "Thinking thread token extraction".to_string(),
        };
        let hud = stack.to_hud_section();
        assert!(hud.contains("[ACTIVE] Observer false-flagging"));
        assert!(hud.contains("↳ Why: User reported confabulation"));
        assert!(hud.contains("[CONTEXT] Discord platform"));
        assert!(hud.contains("[PREVIOUS] Thinking thread"));
    }

    #[test]
    fn test_store_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStackStore::new(tmp.path());

        let stack = ConversationStack {
            active_topic: "Testing".to_string(),
            transition: "Unit test".to_string(),
            context: "CI".to_string(),
            previous_topic: "Setup".to_string(),
        };
        store.save("test_session", &stack).unwrap();

        let loaded = store.load("test_session");
        assert_eq!(loaded.active_topic, "Testing");
        assert_eq!(loaded.previous_topic, "Setup");
    }

    #[test]
    fn test_update_rotates_previous() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStackStore::new(tmp.path());

        store.update_from_audit("s1", "Topic A", "Started", "General").unwrap();
        store.update_from_audit("s1", "Topic B", "User changed subject", "General").unwrap();

        let stack = store.load("s1");
        assert_eq!(stack.active_topic, "Topic B");
        assert_eq!(stack.previous_topic, "Topic A");
    }

    #[test]
    fn test_sanitize_id() {
        assert_eq!(sanitize_id("discord_123_456"), "discord_123_456");
        assert_eq!(sanitize_id("a/b:c"), "a_b_c");
    }
}
