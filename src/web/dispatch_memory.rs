//! Memory tool dispatchers — routes memory/scratchpad/synaptic/timeline/lessons/skills
//! tool calls through AppState's shared MemoryManager.

use crate::web::state::AppState;

pub async fn dispatch_memory(state: &AppState, args: &serde_json::Value) -> anyhow::Result<String> {
    let action = args["action"].as_str().unwrap_or("");
    let query = args["query"].as_str();
    match action {
        "recall" => {
            let memory = state.memory.read().await;
            Ok(memory.recall_context(query.unwrap_or("general"), 2000))
        }
        "status" => {
            let memory = state.memory.read().await;
            Ok(memory.status_summary())
        }
        "search" => {
            let memory = state.memory.read().await;
            Ok(memory.recall_context(query.unwrap_or(""), 1000))
        }
        "reset" => {
            let mut memory = state.memory.write().await;
            memory.clear();
            Ok("All memory tiers cleared.".to_string())
        }
        "consolidate" => {
            let mut memory = state.memory.write().await;
            let timeline_count = memory.timeline.entry_count();
            memory.consolidation.record_consolidation(timeline_count, "Manual consolidation via tool call", 0)?;
            Ok(format!("Memory consolidation recorded. Timeline entries: {}", timeline_count))
        }
        other => Ok(format!("Unknown memory action: '{}'. Valid actions: recall, status, search, reset, consolidate", other)),
    }
}

pub async fn dispatch_scratchpad(state: &AppState, args: &serde_json::Value) -> anyhow::Result<String> {
    let action = args["action"].as_str().unwrap_or("");
    let key = args["key"].as_str().unwrap_or("");
    let value = args["value"].as_str().unwrap_or("");
    let mut memory = state.memory.write().await;
    match action {
        "pin" => { let _ = memory.scratchpad.pin(key, value); Ok(format!("Pinned: {} = {}", key, value)) }
        "unpin" => { let _ = memory.scratchpad.unpin(key); Ok(format!("Unpinned: {}", key)) }
        "list" => {
            let all = memory.scratchpad.all();
            let entries: Vec<String> = all.iter().map(|e| format!("{}: {}", e.key, e.value)).collect();
            Ok(if entries.is_empty() { "Scratchpad is empty.".to_string() } else { entries.join("\n") })
        }
        "get" => Ok(memory.scratchpad.get(key).map(|s| s.to_string())
            .unwrap_or_else(|| format!("No entry for '{}'", key))),
        other => Ok(format!("Unknown scratchpad action: {}", other)),
    }
}

pub async fn dispatch_synaptic(state: &AppState, args: &serde_json::Value) -> anyhow::Result<String> {
    let action = args["action"].as_str().unwrap_or("");
    let mut memory = state.memory.write().await;
    match action {
        "store" => synaptic_store(&mut memory, args),
        "store_relationship" => synaptic_store_relationship(&mut memory, args),
        "search" => synaptic_search(&memory, args),
        "beliefs" => synaptic_beliefs(&memory, args),
        "recent" => synaptic_recent(&memory, args),
        "stats" => Ok(format!(
            "Nodes: {}, Edges: {}, Layers: {:?}",
            memory.synaptic.node_count(), memory.synaptic.edge_count(), memory.synaptic.layers()
        )),
        "layers" => Ok(format!("Layers: {:?}", memory.synaptic.layers())),
        "co_activate" => {
            let a = args["concept"].as_str().unwrap_or("");
            let b = args["target"].as_str().unwrap_or("");
            memory.synaptic.co_activate(a, b, 0.1);
            Ok(format!("Co-activated: {} <-> {}", a, b))
        }
        other => Ok(format!("Unknown synaptic action: {}", other)),
    }
}

fn synaptic_store(memory: &mut crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let concept = args["concept"].as_str().unwrap_or("");
    let layer = args["layer"].as_str().unwrap_or("general");
    let mut data = std::collections::HashMap::new();
    if let Some(obj) = args["data"].as_object() {
        for (k, v) in obj { data.insert(k.clone(), v.as_str().unwrap_or("").to_string()); }
    }
    match memory.synaptic.upsert_node(concept, data, layer) {
        Ok(_) => Ok(format!("Stored concept '{}' in layer '{}'", concept, layer)),
        Err(e) => Ok(format!("Error: {}", e)),
    }
}

fn synaptic_store_relationship(memory: &mut crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let source = args["concept"].as_str().unwrap_or("");
    let target = args["target"].as_str().unwrap_or("");
    let edge_type = args["edge_type"].as_str().unwrap_or("related_to");
    match memory.synaptic.add_edge(source, target, edge_type) {
        Ok(_) => Ok(format!("{} --{}-> {}", source, edge_type, target)),
        Err(e) => Ok(format!("Error: {}", e)),
    }
}

fn synaptic_search(memory: &crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let q = args["concept"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let nodes = memory.synaptic.search_nodes(q, limit);
    let results: Vec<String> = nodes.iter()
        .map(|n| format!("{} [{}] (strength: {:.2})", n.id, n.layer, n.strength))
        .collect();
    Ok(if results.is_empty() { format!("No nodes matching '{}'", q) } else { results.join("\n") })
}

fn synaptic_beliefs(memory: &crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let concept = args["concept"].as_str().unwrap_or("");
    match memory.synaptic.get_node(concept) {
        Some(node) => {
            let data: Vec<String> = node.data.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
            Ok(format!("{} [{}]\n{}", node.id, node.layer, data.join("\n")))
        }
        None => Ok(format!("No concept '{}'", concept)),
    }
}

fn synaptic_recent(memory: &crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let n = args["limit"].as_u64().unwrap_or(10) as usize;
    let nodes = memory.synaptic.recent_nodes(n);
    let results: Vec<String> = nodes.iter().map(|n| format!("{} [{}]", n.id, n.layer)).collect();
    Ok(results.join("\n"))
}

pub async fn dispatch_timeline(state: &AppState, args: &serde_json::Value) -> anyhow::Result<String> {
    let action = args["action"].as_str().unwrap_or("");
    let memory = state.memory.read().await;
    match action {
        "recent" => timeline_recent(&memory, args),
        "search" => timeline_search(&memory, args),
        "session" => timeline_session(&memory, args),
        other => Ok(format!("Unknown timeline action: {}", other)),
    }
}

fn timeline_recent(memory: &crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let n = args["limit"].as_u64().unwrap_or(10) as usize;
    let entries = memory.timeline.recent(n);
    let results: Vec<String> = entries.iter()
        .map(|e| format!("[{}] {}", e.timestamp.format("%Y-%m-%d %H:%M"), e.transcript))
        .collect();
    Ok(if results.is_empty() { "No timeline entries.".to_string() } else { results.join("\n") })
}

fn timeline_search(memory: &crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let q = args["query"].as_str().unwrap_or("");
    let entries = memory.timeline.search(q, 20);
    let results: Vec<String> = entries.iter()
        .map(|e| format!("[{}] {}", e.timestamp.format("%Y-%m-%d %H:%M"), e.transcript))
        .collect();
    Ok(if results.is_empty() { format!("No matches for '{}'", q) } else { results.join("\n") })
}

fn timeline_session(memory: &crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let sid = args["session_id"].as_str().unwrap_or("");
    let entries = memory.timeline.search(sid, 50);
    let results: Vec<String> = entries.iter()
        .map(|e| format!("[{}] {}", e.timestamp.format("%H:%M"), e.transcript))
        .collect();
    Ok(if results.is_empty() { format!("No entries for session '{}'", sid) } else { results.join("\n") })
}

pub async fn dispatch_lessons(state: &AppState, args: &serde_json::Value) -> anyhow::Result<String> {
    let action = args["action"].as_str().unwrap_or("");
    let mut memory = state.memory.write().await;
    match action {
        "add" => {
            let rule = args["rule"].as_str().unwrap_or("");
            let conf = args["confidence"].as_f64().unwrap_or(0.8) as f32;
            let _ = memory.lessons.add(rule, "agent", conf);
            Ok(format!("Learned: '{}' (confidence: {:.0}%)", rule, conf * 100.0))
        }
        "remove" => remove_lesson(&mut memory, args),
        "list" => {
            let all = memory.lessons.all();
            let results: Vec<String> = all.iter()
                .map(|l| format!("[{}] [{:.0}%] {}", &l.id[..8], l.confidence * 100.0, l.rule))
                .collect();
            Ok(if results.is_empty() { "No lessons learned yet.".to_string() } else { results.join("\n") })
        }
        "search" => {
            let q = args["query"].as_str().unwrap_or("");
            let matches = memory.lessons.search(q, 20);
            let results: Vec<String> = matches.iter()
                .map(|l| format!("[{}] [{:.0}%] {}", &l.id[..8], l.confidence * 100.0, l.rule))
                .collect();
            Ok(if results.is_empty() { format!("No lessons matching '{}'", q) } else { results.join("\n") })
        }
        other => Ok(format!("Unknown lessons action: {}", other)),
    }
}

/// Handle lesson removal by ID or query match.
fn remove_lesson(memory: &mut crate::memory::MemoryManager, args: &serde_json::Value) -> anyhow::Result<String> {
    let id = args["id"].as_str().unwrap_or("");
    let query = args["query"].as_str().unwrap_or("");
    if !id.is_empty() {
        match memory.lessons.remove(id) {
            Ok(()) => Ok(format!("Removed lesson: {}", id)),
            Err(e) => Ok(format!("Error removing lesson '{}': {}", id, e)),
        }
    } else if !query.is_empty() {
        let matches: Vec<String> = memory.lessons.search(query, 100)
            .iter().map(|l| l.id.clone()).collect();
        if matches.is_empty() { return Ok(format!("No lessons matching '{}' to remove", query)); }
        let count = matches.len();
        for mid in &matches { let _ = memory.lessons.remove(mid); }
        Ok(format!("Removed {} lesson(s) matching '{}'", count, query))
    } else {
        Ok("Error: 'id' or 'query' required for remove.".to_string())
    }
}

pub async fn dispatch_self_skills(state: &AppState, args: &serde_json::Value) -> anyhow::Result<String> {
    let mut memory = state.memory.write().await;
    crate::tools::self_skills_tool::execute(args, &mut memory.procedures).await
}
