// Ern-OS — Extended tool schema definitions (session recall + introspection).
// Split from schema_definitions.rs for governance compliance (<500 lines).

pub fn session_recall_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "session_recall",
            "description": "Browse and read your prior chat sessions for additional context. Actions: 'list' (paginated index), 'get' (full session), 'summary' (topic digest), 'search' (query match), 'topics' (subject list)",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "get", "summary", "search", "topics"], "description": "Session operation" },
                    "session_id": { "type": "string", "description": "Session ID (for get/summary/topics)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "page": { "type": "integer", "description": "Page number (for list, default 1)" },
                    "per_page": { "type": "integer", "description": "Results per page (for list, default 10)" },
                    "limit": { "type": "integer", "description": "Max results (for search, default 10)" }
                },
                "required": ["action"]
            }
        }
    })
}

pub fn introspect_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "introspect",
            "description": "Access your own reasoning logs, agent activity, scheduler status, observer audits, and system health. Your self-awareness layer. Actions: 'reasoning_log', 'agent_activity', 'scheduler_status', 'observer_audit', 'system_status', 'my_tools'",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["reasoning_log", "agent_activity", "scheduler_status", "observer_audit", "system_status", "my_tools"], "description": "Introspection operation" },
                    "limit": { "type": "integer", "description": "Max entries (for reasoning_log/agent_activity/observer_audit)" },
                    "session_id": { "type": "string", "description": "Session ID (for reasoning_log — defaults to current/most recent)" }
                },
                "required": ["action"]
            }
        }
    })
}
