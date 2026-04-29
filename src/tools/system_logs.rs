// Ern-OS — High-performance, model-neutral Rust AI agent engine
// Created by @mettamazza (github.com/mettamazza)
// License: MIT
//! System logs tool — read-only access to engine logs for self-healing.
//!
//! The agent can read its own error/warn output to diagnose and fix
//! issues proactively. Write access is NOT provided.
//!
//! Reads daily rotating log files (`ern-os.log.YYYY-MM-DD`) produced by
//! `tracing_appender::rolling::daily` in `src/logging/mod.rs`.

use std::path::{Path, PathBuf};

fn paginate_lines(items: &[String], page: usize, per_page: usize) -> String {
    let total = items.len();
    if total == 0 { return String::new(); }
    let total_pages = (total + per_page - 1) / per_page;
    let page = page.min(total_pages);
    let start = (page - 1) * per_page;
    let end = (start + per_page).min(total);
    let mut out = items[start..end].join("\n");
    out.push_str(&format!("\n--- Page {}/{} ({} total) ---", page, total_pages, total));
    out
}

fn get_page(args: &serde_json::Value) -> usize {
    args["page"].as_u64().unwrap_or(1).max(1) as usize
}

fn get_per_page(args: &serde_json::Value) -> usize {
    args["per_page"].as_u64().unwrap_or(30).clamp(1, 100) as usize
}

/// Discover daily rotating log files (`ern-os.log.*`), newest first.
fn discover_log_files(data_dir: &Path) -> Vec<PathBuf> {
    let log_dir = data_dir.join("logs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&log_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("ern-os.log."))
                .unwrap_or(false)
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a)); // newest first
    files
}

/// Execute a system_logs action.
pub fn execute(args: &serde_json::Value, data_dir: &Path) -> anyhow::Result<String> {
    tracing::info!(tool = "system_logs", "tool START");
    let action = args["action"].as_str().unwrap_or("tail");
    match action {
        "tail" => tail_logs(data_dir, args),
        "errors" => grep_errors(data_dir, args),
        "search" => search_logs(data_dir, args),
        "self_edits" => list_self_edits(data_dir, args),
        other => Ok(format!("Unknown system_logs action: {}", other)),
    }
}

/// Return the last N lines from the most recent engine log file.
fn tail_logs(data_dir: &Path, args: &serde_json::Value) -> anyhow::Result<String> {
    let log_files = discover_log_files(data_dir);
    let log_path = match log_files.first() {
        Some(p) => p,
        None => return Ok("No log files found. The engine may not have generated logs yet.".into()),
    };

    let content = std::fs::read_to_string(log_path)?;
    let lines: Vec<String> = content.lines().rev().map(|s| s.to_string()).collect();
    if lines.is_empty() {
        return Ok("Log file is empty.".to_string());
    }
    Ok(paginate_lines(&lines, get_page(args), get_per_page(args)))
}

/// Grep for ERROR and WARN lines across all daily log files, newest first.
fn grep_errors(data_dir: &Path, args: &serde_json::Value) -> anyhow::Result<String> {
    let n = args["max"].as_u64().unwrap_or(30) as usize;
    let mut errors = Vec::new();

    for log_path in discover_log_files(data_dir) {
        if let Ok(content) = std::fs::read_to_string(&log_path) {
            let fname = log_path.file_name().unwrap_or_default().to_string_lossy();
            for line in content.lines() {
                let lower = line.to_lowercase();
                if lower.contains("\"error\"") || lower.contains("\"warn\"")
                    || lower.contains("panic") || lower.contains("failed")
                {
                    errors.push(format!("[{}] {}", fname, line));
                    if errors.len() >= n { break; }
                }
            }
            if errors.len() >= n { break; }
        }
    }

    // Return latest first
    errors.reverse();
    errors.truncate(n);

    if errors.is_empty() {
        Ok("No errors or warnings found in logs.".into())
    } else {
        Ok(paginate_lines(&errors, get_page(args), get_per_page(args)))
    }
}

/// Search logs for a specific pattern across all daily log files.
fn search_logs(data_dir: &Path, args: &serde_json::Value) -> anyhow::Result<String> {
    let pattern = args["pattern"].as_str().unwrap_or("");
    if pattern.is_empty() {
        anyhow::bail!("Missing 'pattern' parameter");
    }
    let n = args["max"].as_u64().unwrap_or(20) as usize;
    let lower_pattern = pattern.to_lowercase();
    let mut matches = Vec::new();

    // Search daily rotating logs
    for path in discover_log_files(data_dir) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let fname = path.file_name().unwrap_or_default().to_string_lossy();
            for (i, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&lower_pattern) {
                    matches.push(format!("[{}:{}] {}", fname, i + 1, line));
                    if matches.len() >= n { break; }
                }
            }
            if matches.len() >= n { break; }
        }
    }

    // Also search auxiliary log files
    let aux_files = [
        data_dir.join("recompile_log.md"),
        data_dir.join("self_edit_log.jsonl"),
    ];
    for path in &aux_files {
        if !path.exists() || matches.len() >= n { continue; }
        if let Ok(content) = std::fs::read_to_string(path) {
            let fname = path.file_name().unwrap_or_default().to_string_lossy();
            for (i, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&lower_pattern) {
                    matches.push(format!("[{}:{}] {}", fname, i + 1, line));
                    if matches.len() >= n { break; }
                }
            }
        }
    }

    if matches.is_empty() {
        Ok(format!("No matches for '{}' in logs.", pattern))
    } else {
        Ok(paginate_lines(&matches, get_page(args), get_per_page(args)))
    }
}

/// List recent self-edit audit entries.
fn list_self_edits(data_dir: &Path, args: &serde_json::Value) -> anyhow::Result<String> {
    let path = data_dir.join("self_edit_log.jsonl");

    if !path.exists() {
        return Ok("No self-edit log found. No codebase edits have been made.".into());
    }

    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<String> = content.lines().rev().map(|s| s.to_string()).collect();
    if lines.is_empty() {
        return Ok("No self-edit entries.".to_string());
    }
    Ok(paginate_lines(&lines, get_page(args), get_per_page(args)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let d = std::path::PathBuf::from(format!("target/test_system_logs_{}", name));
        let _ = fs::create_dir_all(d.join("logs"));
        d
    }

    #[test]
    fn test_tail_empty() {
        let dir = test_dir("tail_empty2");
        // Remove any existing log files
        let _ = fs::remove_dir_all(dir.join("logs"));
        let _ = fs::create_dir_all(dir.join("logs"));
        let args = serde_json::json!({"action": "tail"});
        let result = execute(&args, &dir);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("No log files"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tail_with_content() {
        let dir = test_dir("tail_content2");
        let log = dir.join("logs/ern-os.log.2026-04-28");
        fs::write(&log, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let args = serde_json::json!({"action": "tail"});
        let result = execute(&args, &dir).unwrap();
        // tail returns lines in reverse order (newest first)
        assert!(result.contains("line3"));
        assert!(result.contains("line5"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_errors_grep() {
        let dir = test_dir("errors_grep2");
        let log = dir.join("logs/ern-os.log.2026-04-28");
        fs::write(&log, "{\"level\":\"INFO\",\"fields\":{\"message\":\"all good\"}}\n\
                         {\"level\":\"ERROR\",\"fields\":{\"message\":\"something broke\"}}\n\
                         {\"level\":\"WARN\",\"fields\":{\"message\":\"caution\"}}\n\
                         {\"level\":\"INFO\",\"fields\":{\"message\":\"ok\"}}\n").unwrap();

        let args = serde_json::json!({"action": "errors"});
        let result = execute(&args, &dir).unwrap();
        assert!(result.contains("something broke"));
        assert!(result.contains("caution"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_search_pattern() {
        let dir = test_dir("search2");
        let log = dir.join("logs/ern-os.log.2026-04-28");
        fs::write(&log, "Starting server\nListening on 3000\nConnection established\n").unwrap();

        let args = serde_json::json!({"action": "search", "pattern": "3000"});
        let result = execute(&args, &dir).unwrap();
        assert!(result.contains("Listening on 3000"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_self_edits_empty() {
        let dir = test_dir("edits_empty2");
        let _ = fs::remove_file(dir.join("self_edit_log.jsonl"));
        let args = serde_json::json!({"action": "self_edits"});
        let result = execute(&args, &dir).unwrap();
        assert!(result.contains("No self-edit log"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_discover_log_files_ordering() {
        let dir = test_dir("discover2");
        let _ = fs::create_dir_all(dir.join("logs"));
        fs::write(dir.join("logs/ern-os.log.2026-04-27"), "old").unwrap();
        fs::write(dir.join("logs/ern-os.log.2026-04-28"), "new").unwrap();
        fs::write(dir.join("logs/other.txt"), "ignore").unwrap();
        let files = discover_log_files(&dir);
        assert_eq!(files.len(), 2);
        // Newest first
        assert!(files[0].to_str().unwrap().contains("04-28"));
        assert!(files[1].to_str().unwrap().contains("04-27"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_errors_grep_across_multiple_files() {
        let dir = test_dir("errors_multi2");
        fs::write(
            dir.join("logs/ern-os.log.2026-04-27"),
            "{\"level\":\"ERROR\",\"fields\":{\"message\":\"old error\"}}\n",
        ).unwrap();
        fs::write(
            dir.join("logs/ern-os.log.2026-04-28"),
            "{\"level\":\"ERROR\",\"fields\":{\"message\":\"new error\"}}\n",
        ).unwrap();

        let args = serde_json::json!({"action": "errors"});
        let result = execute(&args, &dir).unwrap();
        assert!(result.contains("old error"));
        assert!(result.contains("new error"));

        let _ = fs::remove_dir_all(&dir);
    }
}
