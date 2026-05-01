// Ern-OS — Self-Skills tool — manage reusable procedural skills

use crate::memory::procedures::{ProcedureStep, ProcedureStore};
use anyhow::Result;

pub async fn execute(args: &serde_json::Value, procedures: &mut ProcedureStore) -> Result<String> {
    tracing::info!(tool = "self_skills", "tool START");
    let action = args["action"].as_str().unwrap_or("");
    match action {
        "list" => list_skills(procedures),
        "view" => view_skill(args, procedures),
        "create" => create_skill(args, procedures),
        "refine" => refine_skill(args, procedures),
        "delete" => delete_skill(args, procedures),
        other => Ok(format!("Unknown self_skills action: {}", other)),
    }
}

fn list_skills(procedures: &ProcedureStore) -> Result<String> {
    let all = procedures.all();
    if all.is_empty() { return Ok("No skills learned yet.".to_string()); }
    let lines: Vec<String> = all.iter().map(|p| {
        format!("[{}] **{}** — {} ({} steps, used {} times)",
            &p.id[..8], p.name, p.description, p.steps.len(), p.success_count)
    }).collect();
    Ok(lines.join("\n"))
}

fn view_skill(args: &serde_json::Value, procedures: &ProcedureStore) -> Result<String> {
    let name = args["name"].as_str().unwrap_or("");
    match procedures.find_by_name(name) {
        Some(p) => {
            let steps: Vec<String> = p.steps.iter().enumerate().map(|(i, s)| {
                format!("  {}. [{}] {}", i + 1, s.tool, s.instruction)
            }).collect();
            Ok(format!("**{}**\n{}\n\nSteps:\n{}", p.name, p.description, steps.join("\n")))
        }
        None => Ok(format!("Skill '{}' not found", name)),
    }
}

fn create_skill(args: &serde_json::Value, procedures: &mut ProcedureStore) -> Result<String> {
    let name = args["name"].as_str().unwrap_or("unnamed");
    let desc = args["description"].as_str().unwrap_or("");
    let steps = parse_steps(args);
    match procedures.add_if_new(name, desc, steps)? {
        true => Ok(format!("Skill '{}' created successfully", name)),
        false => Ok(format!("Skill '{}' already exists — use refine to update", name)),
    }
}

/// Resolve a skill identifier — tries `id` first, then falls back to `name` lookup.
/// If `id` is provided but doesn't match any procedure, returns None (not a silent
/// fallback to name — the caller produces a clear error per §2.6).
fn resolve_id(args: &serde_json::Value, procedures: &ProcedureStore) -> Option<String> {
    if let Some(id) = args["id"].as_str().filter(|s| !s.is_empty()) {
        // Validate that the id actually matches a procedure
        if procedures.all().iter().any(|p| p.id == id || p.id.starts_with(id)) {
            return Some(id.to_string());
        }
        // id field is set but doesn't match — return None so caller can produce
        // a clear error with available procedures (§2.6), not a silent fallback (§2.4)
        return None;
    }
    if let Some(name) = args["name"].as_str().filter(|s| !s.is_empty()) {
        return procedures.find_by_name(name).map(|p| p.id.clone());
    }
    None
}

fn refine_skill(args: &serde_json::Value, procedures: &mut ProcedureStore) -> Result<String> {
    let id = match resolve_id(args, procedures) {
        Some(id) => id,
        None => {
            let available = format_available_procedures(procedures);
            let attempted = args["id"].as_str()
                .or(args["name"].as_str())
                .unwrap_or("(none)");
            return Ok(format!(
                "Error: No procedure matching '{}'. \
                 Use 'list' to see available skills.\n{}",
                attempted, available
            ));
        }
    };
    let steps = parse_steps(args);
    procedures.refine(&id, steps)?;
    Ok(format!("Skill '{}' refined", id))
}

fn delete_skill(args: &serde_json::Value, procedures: &mut ProcedureStore) -> Result<String> {
    let id = match resolve_id(args, procedures) {
        Some(id) => id,
        None => {
            let available = format_available_procedures(procedures);
            let attempted = args["id"].as_str()
                .or(args["name"].as_str())
                .unwrap_or("(none)");
            return Ok(format!(
                "Error: No procedure matching '{}'. \
                 Use 'list' to see available skills.\n{}",
                attempted, available
            ));
        }
    };
    procedures.remove(&id)?;
    Ok(format!("Skill '{}' deleted", id))
}

/// Format available procedures for clear error messages (§2.6).
fn format_available_procedures(procedures: &ProcedureStore) -> String {
    let all = procedures.all();
    if all.is_empty() {
        return "Available procedures: (none)".to_string();
    }
    let entries: Vec<String> = all.iter()
        .map(|p| format!("  [{}] {}", &p.id[..8.min(p.id.len())], p.name))
        .collect();
    format!("Available procedures:\n{}", entries.join("\n"))
}

fn parse_steps(args: &serde_json::Value) -> Vec<ProcedureStep> {
    args["steps"].as_array().map(|arr| {
        arr.iter().map(|s| ProcedureStep {
            tool: s["tool"].as_str().unwrap_or("").to_string(),
            purpose: s["purpose"].as_str().unwrap_or("").to_string(),
            instruction: s["instruction"].as_str().unwrap_or("").to_string(),
        }).collect()
    }).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_empty() {
        let mut store = ProcedureStore::new();
        let args = serde_json::json!({"action": "list"});
        let result = execute(&args, &mut store).await.unwrap();
        assert!(result.contains("No skills"));
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let mut store = ProcedureStore::new();
        let args = serde_json::json!({
            "action": "create",
            "name": "Deploy",
            "description": "Deploy to production",
            "steps": [{"tool": "shell", "instruction": "cargo build --release"}]
        });
        execute(&args, &mut store).await.unwrap();
        let list_args = serde_json::json!({"action": "list"});
        let result = execute(&list_args, &mut store).await.unwrap();
        assert!(result.contains("Deploy"));
    }

    #[tokio::test]
    async fn test_refine_with_name_in_id_field_fails_with_hint() {
        let mut store = ProcedureStore::new();
        // Create a skill
        let args = serde_json::json!({
            "action": "create", "name": "Deploy",
            "steps": [{"tool": "shell", "instruction": "build"}]
        });
        execute(&args, &mut store).await.unwrap();
        // Try to refine using the name in the id field (model error)
        let refine_args = serde_json::json!({
            "action": "refine", "id": "Deploy",
            "steps": [{"tool": "shell", "instruction": "build --release"}]
        });
        let result = execute(&refine_args, &mut store).await.unwrap();
        assert!(result.contains("Error"), "Should fail: {}", result);
        assert!(result.contains("Deploy"), "Should mention the attempted name");
        assert!(result.contains("Available procedures"), "Should list available procedures");
    }

    #[tokio::test]
    async fn test_refine_by_name_succeeds() {
        let mut store = ProcedureStore::new();
        let args = serde_json::json!({
            "action": "create", "name": "Deploy",
            "steps": [{"tool": "shell", "instruction": "build"}]
        });
        execute(&args, &mut store).await.unwrap();
        // Refine using the name field (correct usage)
        let refine_args = serde_json::json!({
            "action": "refine", "name": "Deploy",
            "steps": [{"tool": "shell", "instruction": "build --release"}]
        });
        let result = execute(&refine_args, &mut store).await.unwrap();
        assert!(result.contains("refined"), "Should succeed: {}", result);
    }

    #[tokio::test]
    async fn test_delete_with_invalid_id_fails_with_hint() {
        let mut store = ProcedureStore::new();
        let args = serde_json::json!({
            "action": "create", "name": "Deploy",
            "steps": [{"tool": "shell", "instruction": "build"}]
        });
        execute(&args, &mut store).await.unwrap();
        let delete_args = serde_json::json!({"action": "delete", "id": "nonexistent"});
        let result = execute(&delete_args, &mut store).await.unwrap();
        assert!(result.contains("Error"), "Should fail: {}", result);
        assert!(result.contains("Available procedures"), "Should list available");
    }
}
