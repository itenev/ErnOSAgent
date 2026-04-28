// Ern-OS — Post-inference output sanitization
// Created by @mettamazza (github.com/mettamazza)
// License: MIT
//! Scrubs leaked tool output patterns from outbound text before delivery.
//!
//! This is the last-resort architectural catcher — the prompt-level
//! Communication Boundary and the Observer should prevent most leaks.
//! This module catches anything that slipped through both layers.

/// Result of a sanitization pass.
pub struct ScrubResult {
    /// The cleaned text (may be identical to input if no leaks found).
    pub text: String,
    /// Whether any leak patterns were detected and removed.
    pub had_leak: bool,
    /// Description of what was caught (for logging/DPO capture).
    pub leak_description: Option<String>,
}

/// Scrub leaked tool output patterns from outbound text.
///
/// Catches patterns like:
/// - "Tool run_bash_command executed."
/// - "Tool call: web_search"
/// - Raw JSON payloads that look like tool results
/// - XML-style tool tags
/// - ReAct observation log lines
pub fn scrub_tool_leaks(text: &str) -> ScrubResult {
    let mut cleaned = text.to_string();
    let mut descriptions: Vec<String> = Vec::new();

    // Pattern 1: "Tool <name> executed." (exact leak from the Discord incident)
    let tool_executed = regex::Regex::new(
        r"(?mi)^Tool\s+\w+\s+executed\.?\s*$"
    ).expect("tool_executed regex");
    if tool_executed.is_match(&cleaned) {
        descriptions.push("Tool execution confirmation".into());
        cleaned = tool_executed.replace_all(&cleaned, "").to_string();
    }

    // Pattern 2: "Running/Executing/Calling tool: <name>"
    let tool_narration = regex::Regex::new(
        r"(?mi)^(?:Running|Executing|Calling|Invoking)\s+(?:tool:?\s+)?\w+\.?\s*$"
    ).expect("tool_narration regex");
    if tool_narration.is_match(&cleaned) {
        descriptions.push("Tool narration".into());
        cleaned = tool_narration.replace_all(&cleaned, "").to_string();
    }

    // Pattern 3: "Action: <tool_name>" / "Observation: <result>" (ReAct log lines)
    let react_log = regex::Regex::new(
        r"(?mi)^(?:Action|Observation|Observe):\s+.*$"
    ).expect("react_log regex");
    if react_log.is_match(&cleaned) {
        descriptions.push("ReAct log line".into());
        cleaned = react_log.replace_all(&cleaned, "").to_string();
    }

    // Pattern 4: XML-style tool tags
    let xml_tags = regex::Regex::new(
        r"</?(?:tool_call|tool_result|system_\w+|action|observation)>"
    ).expect("xml_tags regex");
    if xml_tags.is_match(&cleaned) {
        descriptions.push("XML tool tags".into());
        cleaned = xml_tags.replace_all(&cleaned, "").to_string();
    }

    // Pattern 5: Bare JSON objects on their own (full line is just JSON)
    let bare_json = regex::Regex::new(
        r#"(?m)^\s*\{"\w+":\s*["\d\[\{].*\}\s*$"#
    ).expect("bare_json regex");
    if bare_json.is_match(&cleaned) {
        // Only strip if the entire response is JSON (not embedded code blocks)
        let in_code_block = cleaned.contains("```");
        if !in_code_block {
            descriptions.push("Bare JSON payload".into());
            cleaned = bare_json.replace_all(&cleaned, "").to_string();
        }
    }

    // Clean up excessive blank lines left by removals
    let multi_blank = regex::Regex::new(r"\n{3,}").expect("multi_blank regex");
    cleaned = multi_blank.replace_all(&cleaned, "\n\n").to_string();
    cleaned = cleaned.trim().to_string();

    let had_leak = !descriptions.is_empty();
    let leak_description = if had_leak {
        Some(descriptions.join(", "))
    } else {
        None
    };

    if had_leak {
        tracing::warn!(
            patterns = ?descriptions,
            original_len = text.len(),
            cleaned_len = cleaned.len(),
            "Output sanitizer caught leaked tool output"
        );
    }

    ScrubResult {
        text: cleaned,
        had_leak,
        leak_description,
    }
}

/// Check if the scrubbed text is too empty to deliver (needs re-inference).
pub fn needs_reinference(result: &ScrubResult) -> bool {
    result.had_leak && result.text.len() < 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_text_passes_through() {
        let result = scrub_tool_leaks("Here's what I found in the file.");
        assert!(!result.had_leak);
        assert_eq!(result.text, "Here's what I found in the file.");
        assert!(result.leak_description.is_none());
    }

    #[test]
    fn test_catches_tool_executed() {
        let result = scrub_tool_leaks("Tool run_bash_command executed.");
        assert!(result.had_leak);
        assert!(result.text.is_empty());
        assert!(result.leak_description.as_deref().unwrap().contains("execution confirmation"));
    }

    #[test]
    fn test_catches_tool_narration() {
        let result = scrub_tool_leaks("Running tool: web_search");
        assert!(result.had_leak);
        assert!(result.text.is_empty());
    }

    #[test]
    fn test_catches_react_log_lines() {
        let result = scrub_tool_leaks("Action: run_bash_command\nObservation: file found at /tmp/test");
        assert!(result.had_leak);
        assert!(result.text.is_empty());
    }

    #[test]
    fn test_catches_xml_tags() {
        let input = "<tool_call>web_search</tool_call>";
        let result = scrub_tool_leaks(input);
        assert!(result.had_leak);
    }

    #[test]
    fn test_preserves_json_in_code_blocks() {
        let input = "Here's the config:\n```json\n{\"key\": \"value\"}\n```";
        let result = scrub_tool_leaks(input);
        assert!(!result.had_leak);
        assert!(result.text.contains("{\"key\""));
    }

    #[test]
    fn test_mixed_content_partial_strip() {
        let input = "I found the file on your desktop.\nTool run_bash_command executed.\nIt contains a novel called 'A Mind Is Born'.";
        let result = scrub_tool_leaks(input);
        assert!(result.had_leak);
        assert!(result.text.contains("I found the file"));
        assert!(result.text.contains("A Mind Is Born"));
        assert!(!result.text.contains("Tool run_bash_command"));
    }

    #[test]
    fn test_needs_reinference_empty() {
        let result = scrub_tool_leaks("Tool run_bash_command executed.");
        assert!(needs_reinference(&result));
    }

    #[test]
    fn test_needs_reinference_substantial() {
        let input = "I found your file.\nTool run_bash_command executed.";
        let result = scrub_tool_leaks(input);
        assert!(!needs_reinference(&result));
    }
}
