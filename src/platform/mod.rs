// Ern-OS — Platform adapters module
// Ported from ErnOSAgent — adapted for WebUI-as-hub architecture.
pub mod adapter;
pub mod registry;
pub mod router;
pub mod router_stream;
pub mod router_thread;
pub mod router_interactions;

pub mod discord;
pub mod discord_handler;
pub mod discord_interaction;
pub mod discord_commands;
pub mod discord_cmd_handlers;

pub mod telegram;
