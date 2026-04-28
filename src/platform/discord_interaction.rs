//! Discord interaction handling — parses button clicks and builds component rows.
//!
//! Translates between platform-neutral `MessageComponent` definitions and
//! serenity's `CreateButton` / `CreateActionRow` builders. Handles interaction
//! routing to the hub REST API for feedback and regeneration.

use crate::platform::adapter::{ComponentStyle, MessageComponent, PlatformInteraction};

/// Build the standard response buttons (👍 👎 🔄 🔊) for a delivered message.
pub fn build_response_buttons(session_id: &str, message_index: usize) -> Vec<MessageComponent> {
    vec![
        MessageComponent {
            custom_id: format!("thumbsup:{}:{}", session_id, message_index),
            label: "Good".to_string(),
            emoji: "👍".to_string(),
            style: ComponentStyle::Secondary,
        },
        MessageComponent {
            custom_id: format!("thumbsdown:{}:{}", session_id, message_index),
            label: "Bad".to_string(),
            emoji: "👎".to_string(),
            style: ComponentStyle::Secondary,
        },
        MessageComponent {
            custom_id: format!("regenerate:{}:{}", session_id, message_index),
            label: "Redo".to_string(),
            emoji: "🔄".to_string(),
            style: ComponentStyle::Secondary,
        },
        MessageComponent {
            custom_id: format!("speak:{}:{}", session_id, message_index),
            label: "Speak".to_string(),
            emoji: "🔊".to_string(),
            style: ComponentStyle::Secondary,
        },
    ]
}

/// Build plan approval buttons for a plan response.
pub fn build_plan_buttons(session_id: &str) -> Vec<MessageComponent> {
    vec![
        MessageComponent {
            custom_id: format!("plan_approve:{}", session_id),
            label: "Approve & Execute".to_string(),
            emoji: "✅".to_string(),
            style: ComponentStyle::Success,
        },
        MessageComponent {
            custom_id: format!("plan_revise:{}", session_id),
            label: "Revise".to_string(),
            emoji: "✏️".to_string(),
            style: ComponentStyle::Secondary,
        },
        MessageComponent {
            custom_id: format!("plan_cancel:{}", session_id),
            label: "Cancel".to_string(),
            emoji: "❌".to_string(),
            style: ComponentStyle::Danger,
        },
    ]
}

/// Parse a button custom_id into (action, rest_of_id).
/// Format: "action:session_id:message_index" or "action:session_id"
pub fn parse_custom_id(custom_id: &str) -> Option<ParsedInteraction> {
    let parts: Vec<&str> = custom_id.splitn(3, ':').collect();
    if parts.len() < 2 {
        return None;
    }

    let action = parts[0].to_string();
    let session_id = parts[1].to_string();
    let message_index = parts.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);

    Some(ParsedInteraction { action, session_id, message_index })
}

/// Result of parsing a button custom_id.
#[derive(Debug, Clone)]
pub struct ParsedInteraction {
    pub action: String,
    pub session_id: String,
    pub message_index: usize,
}

/// Convert platform-neutral components to serenity action rows.
pub fn to_serenity_action_rows(
    components: &[MessageComponent],
) -> Vec<serenity::builder::CreateActionRow> {
    use serenity::builder::{CreateActionRow, CreateButton};
    use serenity::all::ButtonStyle;

    let buttons: Vec<CreateButton> = components.iter().map(|c| {
        let style = match c.style {
            ComponentStyle::Primary => ButtonStyle::Primary,
            ComponentStyle::Secondary => ButtonStyle::Secondary,
            ComponentStyle::Success => ButtonStyle::Success,
            ComponentStyle::Danger => ButtonStyle::Danger,
        };
        CreateButton::new(&c.custom_id)
            .label(&c.label)
            .emoji(serenity::all::ReactionType::Unicode(c.emoji.clone()))
            .style(style)
    }).collect();

    // Discord allows max 5 buttons per action row
    buttons.chunks(5).map(|chunk| {
        CreateActionRow::Buttons(chunk.to_vec())
    }).collect()
}

/// Call the hub feedback API (thumbs up/down).
pub async fn call_feedback_api(
    hub_port: u16,
    session_id: &str,
    message_index: usize,
    reaction: &str,
) -> anyhow::Result<()> {
    let url = format!(
        "http://127.0.0.1:{}/api/sessions/{}/messages/{}/react",
        hub_port, session_id, message_index,
    );
    let client = reqwest::Client::new();
    let resp = client.post(&url)
        .json(&serde_json::json!({ "reaction": reaction }))
        .send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("Feedback API returned {}", resp.status());
    }
    Ok(())
}

/// Call the hub platform ingest API for regeneration.
pub async fn call_regenerate_api(
    hub_port: u16,
    interaction: &PlatformInteraction,
    content: &str,
) -> anyhow::Result<serde_json::Value> {
    let url = format!("http://127.0.0.1:{}/api/chat/platform", hub_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let payload = serde_json::json!({
        "platform": interaction.platform,
        "channel_id": interaction.channel_id,
        "user_id": interaction.user_id,
        "user_name": "User",
        "content": content,
        "attachments": [],
        "message_id": interaction.interaction_id,
        "is_admin": true,
    });

    let resp = client.post(&url).json(&payload).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Regenerate API returned {}", resp.status());
    }
    resp.json().await.map_err(Into::into)
}

/// Call the hub TTS API and return WAV audio bytes.
pub async fn call_tts_api(hub_port: u16, text: &str) -> anyhow::Result<Vec<u8>> {
    let url = format!("http://127.0.0.1:{}/api/tts", hub_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "text": text,
        "voice": "am_michael",
        "speed": 1.0,
    });

    let resp = client.post(&url).json(&payload).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("TTS API returned {}", resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_custom_id_thumbsup() {
        let result = parse_custom_id("thumbsup:discord_123_456:3").unwrap();
        assert_eq!(result.action, "thumbsup");
        assert_eq!(result.session_id, "discord_123_456");
        assert_eq!(result.message_index, 3);
    }

    #[test]
    fn test_parse_custom_id_plan() {
        let result = parse_custom_id("plan_approve:discord_123_456").unwrap();
        assert_eq!(result.action, "plan_approve");
        assert_eq!(result.session_id, "discord_123_456");
        assert_eq!(result.message_index, 0);
    }

    #[test]
    fn test_parse_custom_id_invalid() {
        assert!(parse_custom_id("nocolon").is_none());
    }

    #[test]
    fn test_build_response_buttons() {
        let buttons = build_response_buttons("session_1", 5);
        assert_eq!(buttons.len(), 4);
        assert_eq!(buttons[0].custom_id, "thumbsup:session_1:5");
        assert_eq!(buttons[1].custom_id, "thumbsdown:session_1:5");
        assert_eq!(buttons[2].custom_id, "regenerate:session_1:5");
        assert_eq!(buttons[3].custom_id, "speak:session_1:5");
    }

    #[test]
    fn test_build_plan_buttons() {
        let buttons = build_plan_buttons("session_2");
        assert_eq!(buttons.len(), 3);
        assert!(buttons[0].custom_id.starts_with("plan_approve:"));
        assert!(buttons[1].custom_id.starts_with("plan_revise:"));
        assert!(buttons[2].custom_id.starts_with("plan_cancel:"));
    }

    #[test]
    fn test_to_serenity_action_rows() {
        let components = build_response_buttons("s1", 0);
        let rows = to_serenity_action_rows(&components);
        assert_eq!(rows.len(), 1); // 3 buttons fit in 1 row
    }
}
