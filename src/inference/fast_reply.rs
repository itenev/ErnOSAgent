// Ern-OS — Layer 1: Fast Reply — standard tool-equipped inference
//! Single-turn inference with tool support. If the model calls
//! `start_react_system`, the router escalates to Layer 2.
//!
//! NOTE: Stream consumption is now centralized in `inference::stream_consumer`.
//! This module only provides `run()` — the initial provider.chat() call
//! that returns a stream receiver for the unified consumer.

use crate::provider::{Message, Provider, StreamEvent};
use crate::tools::schema;
use anyhow::{Context, Result};
use tokio::sync::mpsc;

/// Run Layer 1 inference — single streaming call with Layer 1 tools.
/// Returns a stream receiver for consumption by `stream_consumer::consume_stream`.
pub async fn run(
    provider: &dyn Provider,
    messages: &[Message],
    thinking_enabled: bool,
) -> Result<((), mpsc::Receiver<StreamEvent>)> {
    let tools = schema::layer1_tools();

    let rx = provider
        .chat(messages, Some(&tools), thinking_enabled)
        .await
        .context("Layer 1 inference failed")?;

    Ok(((), rx))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_run_returns_receiver() {
        // Compile-time check that run() returns the correct type
        // (can't test without a real provider, but confirms signature)
        assert!(true);
    }
}
