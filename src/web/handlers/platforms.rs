//! Platform handler — REST API for managing platform adapter connections and config.

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

    update_discord_config(&mut config, &body);
    update_telegram_config(&mut config, &body);

    if let Err(e) = persist_config(&config) {
        return Json(serde_json::json!({
            "success": false,
            "error": format!("Failed to write config: {}", e),
        }));
    }

    let discord_ready = config.discord.enabled && config.discord.resolve_token().is_some();
    let telegram_ready = config.telegram.enabled && config.telegram.resolve_token().is_some();
    drop(config);

    let results = auto_connect_platforms(&state, discord_ready, telegram_ready).await;

    Json(serde_json::json!({
        "success": true,
        "message": "Platform config updated",
        "connections": results,
    }))
}

/// Apply Discord config fields from the JSON body.
fn update_discord_config(config: &mut crate::config::AppConfig, body: &serde_json::Value) {
    let Some(discord) = body.get("discord") else { return };
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

/// Apply Telegram config fields from the JSON body.
fn update_telegram_config(config: &mut crate::config::AppConfig, body: &serde_json::Value) {
    let Some(telegram) = body.get("telegram") else { return };
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

/// Persist config to ern-os.toml.
fn persist_config(config: &crate::config::AppConfig) -> Result<(), std::io::Error> {
    let serialized = toml::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write("ern-os.toml", serialized)
}

/// Auto-connect platforms that are enabled and have a token.
async fn auto_connect_platforms(
    state: &AppState,
    discord_ready: bool,
    telegram_ready: bool,
) -> Vec<&'static str> {
    let mut results = Vec::new();
    let mut reg = state.platforms.write().await;

    if discord_ready {
        match reg.connect_by_name("Discord").await {
            Ok(_) => {
                tracing::info!("Discord auto-connected after config save");
                results.push("discord: connected");
            }
            Err(e) => {
                tracing::error!(error = %e, "Discord auto-connect failed");
                results.push("discord: failed");
            }
        }
    }

    if telegram_ready {
        match reg.connect_by_name("Telegram").await {
            Ok(_) => {
                tracing::info!("Telegram auto-connected after config save");
                results.push("telegram: connected");
            }
            Err(e) => {
                tracing::error!(error = %e, "Telegram auto-connect failed");
                results.push("telegram: failed");
            }
        }
    }

    results
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
    fn test_persist_config_serializes() {
        let config = crate::config::AppConfig::default();
        let serialized = toml::to_string_pretty(&config);
        assert!(serialized.is_ok());
    }
}
