// Ern-OS — High-performance, model-neutral Rust AI agent engine
// Created by @mettamazza (github.com/mettamazza)
// License: MIT
//! SSE stream parser — handles OpenAI-compatible SSE streams with
//! Gemma 4 thinking block extraction and tool call accumulation.

use crate::provider::StreamEvent;
use anyhow::Result;
use futures_util::StreamExt;
use reqwest::Response;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Accumulated tool call from streaming deltas.
#[derive(Debug, Default, Clone)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

/// OpenAI SSE delta chunk structure.
#[derive(Debug, Deserialize)]
struct SseChunk {
    choices: Option<Vec<SseChoice>>,
}

#[derive(Debug, Deserialize)]
struct SseChoice {
    delta: Option<SseDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<SseToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct SseToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    function: Option<SseFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct SseFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

/// State machine for extracting thinking blocks from content stream.
#[derive(Debug, PartialEq)]
enum ThinkingState {
    /// Normal text output
    Normal,
    /// Inside a `<|channel>thought` ... `<channel|>` block (Gemma 4)
    InsideGemma4,
    /// Inside a `<think>` ... `</think>` block (legacy)
    InsideLegacy,
}

/// Parse an SSE stream from an OpenAI-compatible endpoint.
///
/// Handles:
/// - Text content deltas
/// - Gemma 4 thinking: `<|channel>thought` ... `<channel|>` extraction
/// - Legacy thinking: `<think>` ... `</think>` extraction
/// - `reasoning_content` field (direct thinking support)
/// - Tool call delta accumulation by index
/// - Stream completion
pub async fn parse_sse_stream(
    response: Response,
    tx: mpsc::Sender<StreamEvent>,
    slots_url: Option<String>,
) -> Result<()> {
    let mut tool_calls: Vec<ToolCallAccumulator> = Vec::new();
    let mut thinking_state = ThinkingState::Normal;
    let mut content_buffer = String::new();

    let mut stream = response.bytes_stream();
    let mut chunk_count: u64 = 0;
    let mut has_content = false; // track if any real content was generated
    let start = Instant::now();
    let mut last_chunk_time = Instant::now();
    let mut line_buffer = String::new(); // Buffer for partial lines split across HTTP chunks
    let mut parsed_lines: Vec<String> = Vec::new(); // Track raw lines for empty response diagnostics
    let mut stall_interval = tokio::time::interval(Duration::from_secs(10));
    stall_interval.tick().await; // consume first immediate tick

    loop {
        let chunk_result = tokio::select! {
            chunk_opt = stream.next() => {
                match chunk_opt {
                    Some(r) => r,
                    None => break, // stream ended
                }
            }
            _ = stall_interval.tick() => {
                if let Some(ref url) = slots_url {
                    if let Some(stall) = check_server_stall(url, chunk_count, &last_chunk_time).await {
                        tracing::warn!(
                            server_decoded = stall.n_decoded,
                            client_chunks = chunk_count,
                            secs_since_last_chunk = last_chunk_time.elapsed().as_secs(),
                            "SSE stream: stall detected — server decoding but client not receiving"
                        );
                        let _ = tx.send(StreamEvent::Error(
                            "Stream stalled: server generating tokens but HTTP stream not flushing".into()
                        )).await;
                        return Ok(());
                    }
                }
                continue;
            }
        };
        // If the consumer dropped the receiver (e.g. spiral detection killed the stream),
        // abort immediately. This drops the HTTP response, freeing the llama-server slot
        // so the recovery request can proceed without blocking.
        if tx.is_closed() {
            tracing::info!(
                chunks = chunk_count,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "SSE stream: consumer disconnected — aborting to free server slot"
            );
            return Ok(());
        }

        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, chunks = chunk_count, "SSE stream: chunk read error");
                let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                break;
            }
        };

        chunk_count += 1;
        last_chunk_time = Instant::now();
        let text = String::from_utf8_lossy(&chunk);

        // Diagnostics: log raw content of early chunks at debug level
        // to diagnose the recurring empty response pattern.
        if chunk_count <= 5 {
            tracing::debug!(
                chunk_num = chunk_count,
                len = text.len(),
                content = %text.chars().take(300).collect::<String>(),
                "SSE raw chunk"
            );
        }

        // Append to line buffer and process complete lines
        line_buffer.push_str(&text);

        // Process all complete lines (terminated by \n)
        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos].trim().to_string();
            line_buffer = line_buffer[newline_pos + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            // Track raw lines for diagnostics when response is empty
            if parsed_lines.len() < 10 {
                parsed_lines.push(line.chars().take(200).collect::<String>());
            }

            if line == "data: [DONE]" {
                // CRITICAL: flush remaining content buffer before Done
                if !content_buffer.is_empty() {
                    let _ = tx.send(StreamEvent::TextDelta(content_buffer.clone())).await;
                    content_buffer.clear();
                }
                emit_accumulated_tools(&tool_calls, &tx).await;
                let _ = tx.send(StreamEvent::Done).await;
                if !has_content && tool_calls.is_empty() {
                    tracing::error!(
                        chunks = chunk_count,
                        content_buffer_len = content_buffer.len(),
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        raw_lines = ?parsed_lines,
                        "SSE stream: [DONE] with NO content and NO tool calls — raw lines logged for diagnostics"
                    );
                }
                tracing::debug!(
                    chunks = chunk_count,
                    tool_calls = tool_calls.len(),
                    has_content,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "SSE stream: [DONE] received"
                );
                return Ok(());
            }

            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<SseChunk>(json_str) {
                    let produced = process_chunk(
                        &chunk,
                        &mut tool_calls,
                        &mut thinking_state,
                        &mut content_buffer,
                        &tx,
                    )
                    .await;
                    if produced {
                        has_content = true;
                    }
                }
            }
        }
    }

    // Stream ended without [DONE] — flush remaining content buffer
    tracing::warn!(
        chunks = chunk_count,
        content_len = content_buffer.len(),
        tool_calls = tool_calls.len(),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "SSE stream: ended WITHOUT [DONE]"
    );
    if !content_buffer.is_empty() {
        let _ = tx.send(StreamEvent::TextDelta(content_buffer)).await;
    }
    emit_accumulated_tools(&tool_calls, &tx).await;
    let _ = tx.send(StreamEvent::Done).await;
    Ok(())
}

/// Server stall detection result — carries the server's own reported state.
struct StallInfo {
    n_decoded: u64,
}

/// Check if llama-server is generating tokens that aren't reaching the HTTP stream.
///
/// Queries the server's `/slots` endpoint (model-derived data per §2.1) to read
/// `n_decoded` — the server's real decoded token count. A stall is detected when:
/// - The server is actively processing (`is_processing == true`)
/// - The server has decoded significantly more tokens than the client received
/// - No HTTP chunks have arrived in the recent interval
///
/// HEURISTIC: The `n_decoded > 500 && client_chunks < 10` thresholds are derived
/// from observed minimum generation speed (~13 tok/s on M3 Ultra). At 10-second
/// intervals, the server generates ≥130 tokens. 500 provides ~4x margin against
/// prompt-processing spikes. The `<10 chunks` guard prevents false-positives
/// during normal streaming. Error margin: could false-positive during initial
/// KV cache fill on very large prompts (>200K tokens), mitigated by the
/// `is_processing` check and the `last_chunk_time` age requirement.
async fn check_server_stall(
    slots_url: &str,
    client_chunks: u64,
    last_chunk_time: &Instant,
) -> Option<StallInfo> {
    // Only check if we haven't received data for at least 8 seconds
    if last_chunk_time.elapsed() < Duration::from_secs(8) {
        return None;
    }

    let resp = reqwest::get(slots_url).await.ok()?;
    let slots: Vec<serde_json::Value> = resp.json().await.ok()?;
    let slot = slots.first()?;

    if !slot["is_processing"].as_bool().unwrap_or(false) {
        return None; // Server isn't processing — not a stall
    }

    let n_decoded = slot["next_token"][0]["n_decoded"].as_u64().unwrap_or(0);

    // HEURISTIC: see doc comment above for derivation and error margin.
    if n_decoded > 500 && client_chunks < 10 {
        return Some(StallInfo { n_decoded });
    }

    None
}

/// Process a single SSE chunk.
/// Returns `true` if the chunk contained any content or tool call data.
async fn process_chunk(
    chunk: &SseChunk,
    tool_calls: &mut Vec<ToolCallAccumulator>,
    thinking_state: &mut ThinkingState,
    content_buffer: &mut String,
    tx: &mpsc::Sender<StreamEvent>,
) -> bool {
    let choices = match &chunk.choices {
        Some(c) => c,
        None => return false,
    };

    let mut produced = false;

    for choice in choices {
        let delta = match &choice.delta {
            Some(d) => d,
            None => {
                // No delta — this is a finish-only chunk
                if let Some(reason) = &choice.finish_reason {
                    match reason.as_str() {
                        "length" => tracing::warn!("SSE: finish_reason=length — model hit max_tokens"),
                        "stop" => tracing::debug!("SSE: finish_reason=stop"),
                        other => tracing::info!(finish_reason = other, "SSE: finish_reason received"),
                    }
                }
                continue;
            }
        };

        // Direct reasoning_content field (some providers)
        if let Some(reasoning) = &delta.reasoning_content {
            if !reasoning.is_empty() {
                produced = true;
                let _ = tx.send(StreamEvent::ThinkingDelta(reasoning.clone())).await;
            }
        }

        // Content with thinking block extraction
        if let Some(content) = &delta.content {
            if !content.is_empty() {
                produced = true;
            }
            process_content_delta(
                content,
                thinking_state,
                content_buffer,
                tx,
            )
            .await;
        }

        // Tool call delta accumulation
        if let Some(tc_deltas) = &delta.tool_calls {
            produced = true;
            for tc_delta in tc_deltas {
                accumulate_tool_call(tool_calls, tc_delta);
            }
        }

        // Check finish_reason for truncation detection
        if let Some(reason) = &choice.finish_reason {
            match reason.as_str() {
                "length" => tracing::warn!("SSE: finish_reason=length — response truncated"),
                _ => {}
            }
        }
    }
    produced
}

/// Process content delta with thinking block extraction.
///
/// Handles both Gemma 4 (`<|channel>thought` ... `<channel|>`) and
/// legacy (`<think>` ... `</think>`) thinking formats.
async fn process_content_delta(
    content: &str,
    state: &mut ThinkingState,
    buffer: &mut String,
    tx: &mpsc::Sender<StreamEvent>,
) {
    buffer.push_str(content);

    loop {
        match state {
            ThinkingState::Normal => {
                // Check for Gemma 4 thinking start
                if let Some(pos) = buffer.find("<|channel>thought") {
                    let before = &buffer[..pos];
                    if !before.is_empty() {
                        let _ = tx.send(StreamEvent::TextDelta(before.to_string())).await;
                    }
                    *buffer = buffer[pos + "<|channel>thought".len()..].to_string();
                    *state = ThinkingState::InsideGemma4;
                    continue;
                }

                // Check for legacy thinking start
                if let Some(pos) = buffer.find("<think>") {
                    let before = &buffer[..pos];
                    if !before.is_empty() {
                        let _ = tx.send(StreamEvent::TextDelta(before.to_string())).await;
                    }
                    *buffer = buffer[pos + "<think>".len()..].to_string();
                    *state = ThinkingState::InsideLegacy;
                    continue;
                }

                // No thinking tags — check if buffer might contain partial tag
                let safe_len = safe_emit_length(buffer);
                if safe_len > 0 {
                    let emit = buffer[..safe_len].to_string();
                    let _ = tx.send(StreamEvent::TextDelta(emit)).await;
                    *buffer = buffer[safe_len..].to_string();
                }
                break;
            }

            ThinkingState::InsideGemma4 => {
                if let Some(pos) = buffer.find("<channel|>") {
                    let thinking = &buffer[..pos];
                    if !thinking.is_empty() {
                        let _ = tx.send(StreamEvent::ThinkingDelta(thinking.to_string())).await;
                    }
                    *buffer = buffer[pos + "<channel|>".len()..].to_string();
                    *state = ThinkingState::Normal;
                    continue;
                }

                // Emit partial thinking, keep last 15 chars for tag detection
                let safe = buffer.len().saturating_sub(15);
                if safe > 0 {
                    let emit = buffer[..safe].to_string();
                    let _ = tx.send(StreamEvent::ThinkingDelta(emit)).await;
                    *buffer = buffer[safe..].to_string();
                }
                break;
            }

            ThinkingState::InsideLegacy => {
                if let Some(pos) = buffer.find("</think>") {
                    let thinking = &buffer[..pos];
                    if !thinking.is_empty() {
                        let _ = tx.send(StreamEvent::ThinkingDelta(thinking.to_string())).await;
                    }
                    *buffer = buffer[pos + "</think>".len()..].to_string();
                    *state = ThinkingState::Normal;
                    continue;
                }

                let safe = buffer.len().saturating_sub(10);
                if safe > 0 {
                    let emit = buffer[..safe].to_string();
                    let _ = tx.send(StreamEvent::ThinkingDelta(emit)).await;
                    *buffer = buffer[safe..].to_string();
                }
                break;
            }
        }
    }
}

/// Calculate how many bytes can be safely emitted without splitting a potential tag.
fn safe_emit_length(buffer: &str) -> usize {
    // Keep enough chars to detect partial `<|channel>thought` (17 chars) or `<think>` (7 chars)
    let reserve = 20;
    let raw = buffer.len().saturating_sub(reserve);
    // Snap to a valid char boundary — never slice mid-codepoint.
    let mut pos = raw;
    while pos > 0 && !buffer.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Accumulate tool call deltas by index.
fn accumulate_tool_call(
    tool_calls: &mut Vec<ToolCallAccumulator>,
    delta: &SseToolCallDelta,
) {
    let idx = delta.index.unwrap_or(0);

    // Extend the vector if needed
    while tool_calls.len() <= idx {
        tool_calls.push(ToolCallAccumulator::default());
    }

    let tc = &mut tool_calls[idx];

    if let Some(id) = &delta.id {
        tc.id.clone_from(id);
    }
    if let Some(func) = &delta.function {
        if let Some(name) = &func.name {
            tc.name.clone_from(name);
        }
        if let Some(args) = &func.arguments {
            let clean = strip_special_tokens(args);
            if !clean.is_empty() {
                tc.arguments.push_str(&clean);
            }
        }
    }
}

/// Strip model-specific control tokens that should never appear in tool call arguments.
/// These are generation artifacts from Gemma 4's channel/tool_call system, not valid JSON.
fn strip_special_tokens(text: &str) -> String {
    text.replace("<tool_call|>", "")
        .replace("<|tool_call>", "")
        .replace("<|channel>thought", "")
        .replace("<|channel>", "")
        .replace("<channel|>", "")
}

/// Emit all accumulated tool calls as StreamEvents.
/// Validates that accumulated arguments are parseable JSON — skips corrupt calls.
async fn emit_accumulated_tools(
    tool_calls: &[ToolCallAccumulator],
    tx: &mpsc::Sender<StreamEvent>,
) {
    for tc in tool_calls {
        if !tc.name.is_empty() {
            let args = tc.arguments.trim();
            if !args.is_empty() && serde_json::from_str::<serde_json::Value>(args).is_err() {
                tracing::error!(
                    tool = %tc.name,
                    args_len = args.len(),
                    args_preview = %args.chars().take(200).collect::<String>(),
                    "Corrupt tool call arguments — skipping (model emitted control tokens in args)"
                );
                continue;
            }
            let _ = tx
                .send(StreamEvent::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .await;
        }
    }
}

/// Re-export spiral detection from dedicated module.
pub use super::spiral_detector::detect_thought_spiral;

include!("stream_parser_tests.rs");
