// Ern-OS — Shell command execution tool

use anyhow::Result;
use std::process::Stdio;

/// Execute a shell command with timeout and capture output.
pub async fn run_command(command: &str, working_dir: Option<&str>) -> Result<String> {
    if command.is_empty() {
        anyhow::bail!("Empty command");
    }

    let cmd_display: String = command.chars().take(200).collect();
    tracing::info!(command = %cmd_display, working_dir = ?working_dir, "shell START");
    let start = std::time::Instant::now();

    let output = spawn_with_timeout(command, working_dir, &cmd_display).await?;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    tracing::info!(
        command = %cmd_display, exit_code, stdout_len = stdout.len(),
        stderr_len = stderr.len(), elapsed_ms, "shell COMPLETE"
    );

    Ok(format_command_output(&stdout, &stderr, exit_code))
}

/// Spawn a bash command with a 120s timeout.
async fn spawn_with_timeout(
    command: &str,
    working_dir: Option<&str>,
    cmd_display: &str,
) -> Result<std::process::Output> {
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c").arg(command).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(wd) = working_dir {
        cmd.current_dir(wd);
    }

    match tokio::time::timeout(tokio::time::Duration::from_secs(120), cmd.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => {
            tracing::error!(command = %cmd_display, err = %e, "shell SPAWN FAILED");
            anyhow::bail!("Failed to execute: {}", e);
        }
        Err(_) => {
            tracing::warn!(command = %cmd_display, timeout_secs = 120, "shell TIMEOUT");
            anyhow::bail!("Command timed out after 120s");
        }
    }
}

/// Format stdout + stderr into a single result string.
fn format_command_output(stdout: &str, stderr: &str, exit_code: i32) -> String {
    let mut result = String::new();
    if !stdout.is_empty() { result.push_str(stdout); }
    if !stderr.is_empty() {
        if !result.is_empty() { result.push('\n'); }
        result.push_str("[stderr] ");
        result.push_str(stderr);
    }
    if result.is_empty() {
        result = format!("Exit code: {}", exit_code);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_echo() {
        let result = run_command("echo hello", None).await.unwrap();
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_empty_command() {
        assert!(run_command("", None).await.is_err());
    }

    #[tokio::test]
    async fn test_working_dir() {
        let result = run_command("pwd", Some("/tmp")).await.unwrap();
        assert!(result.contains("/tmp") || result.contains("private/tmp"));
    }
}
