//! Text & code extractors — source code with language detection, logs, certs, binary fallback.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use super::{ExtractionResult, make_result, make_result_with_meta};

/// Extract text/code file with language annotation.
pub fn extract_text_with_lang(path: &Path, ext: &str) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read: {}", path.display()))?;

    let lang = detect_language(ext);
    let mut result = make_result(content, "text/plain");
    result.language = lang.map(|s| s.to_string());
    Ok(result)
}

/// Map file extension to programming language name.
fn detect_language(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" => Some("c"),
        "cpp" | "cc" => Some("cpp"),
        "h" | "hpp" => Some("c"),
        "rb" => Some("ruby"),
        "swift" => Some("swift"),
        "kt" | "kts" => Some("kotlin"),
        "cs" => Some("csharp"),
        "php" => Some("php"),
        "lua" => Some("lua"),
        "r" => Some("r"),
        "scala" => Some("scala"),
        "zig" => Some("zig"),
        "sh" | "bash" | "zsh" | "fish" => Some("bash"),
        "sql" => Some("sql"),
        "md" | "mdx" => Some("markdown"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "dockerfile" | "containerfile" => Some("dockerfile"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "json" => Some("json"),
        "xml" => Some("xml"),
        _ => None,
    }
}

/// Extract log file — tail last 200 lines, count errors.
pub fn extract_log(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = total.saturating_sub(200);
    let tail: Vec<&str> = lines[start..].to_vec();

    let errors = count_error_lines(&tail);

    let mut meta = HashMap::new();
    meta.insert("total_lines".into(), total.to_string());
    meta.insert("errors_in_tail".into(), errors.to_string());

    Ok(make_result_with_meta(
        format!("[Log: {} lines, {} errors in tail]\n\n{}", total, errors, tail.join("\n")),
        "text/plain",
        meta,
    ))
}

/// Count lines containing error/fatal/panic keywords.
fn count_error_lines(lines: &[&str]) -> usize {
    lines.iter().filter(|l| {
        let lower = l.to_lowercase();
        lower.contains("error") || lower.contains("fatal") || lower.contains("panic")
    }).count()
}

/// Extract certificate info via openssl, or raw PEM text.
pub fn extract_cert(path: &Path) -> Result<ExtractionResult> {
    if let Ok(output) = std::process::Command::new("openssl")
        .args(["x509", "-text", "-noout", "-in"])
        .arg(path)
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(make_result(format!("[Certificate]\n\n{}", text), "application/x-pem-file"));
        }
    }
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[PEM/Certificate]\n\n{}", content), "application/x-pem-file"))
}

/// Binary fallback — try as text first, then hex dump.
pub fn extract_binary_fallback(path: &Path, ext: &str) -> Result<ExtractionResult> {
    if let Ok(content) = std::fs::read_to_string(path) {
        return Ok(make_result(content, "text/plain"));
    }

    let bytes = std::fs::read(path)?;
    let size = bytes.len();

    let file_type = detect_file_type(path);
    let hex_dump = format_hex_dump(&bytes);

    Ok(make_result(
        format!("[Binary: {} — {} bytes]\nType: {}\n\n{}", ext, size, file_type, hex_dump),
        "application/octet-stream",
    ))
}

/// Run `file --brief` to detect binary file type.
fn detect_file_type(path: &Path) -> String {
    std::process::Command::new("file")
        .args(["--brief"])
        .arg(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Format first 512 bytes as a hex dump.
fn format_hex_dump(bytes: &[u8]) -> String {
    let hex_len = bytes.len().min(512);
    let hex: Vec<String> = bytes[..hex_len].iter().map(|b| format!("{:02x}", b)).collect();
    let lines: Vec<String> = hex.chunks(16).map(|chunk| chunk.join(" ")).collect();
    format!("Hex dump (first {} bytes):\n{}", hex_len, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language_rust() {
        assert_eq!(detect_language("rs"), Some("rust"));
    }

    #[test]
    fn test_detect_language_unknown() {
        assert_eq!(detect_language("xyz"), None);
    }

    #[test]
    fn test_count_error_lines() {
        let lines = vec!["INFO: ok", "ERROR: bad", "FATAL: crash", "DEBUG: fine"];
        assert_eq!(count_error_lines(&lines), 2);
    }

    #[test]
    fn test_format_hex_dump() {
        let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let dump = format_hex_dump(&bytes);
        assert!(dump.contains("de ad be ef"));
    }

    #[test]
    fn test_extract_text_with_lang() {
        let tmp = tempfile::NamedTempFile::with_suffix(".py").unwrap();
        std::fs::write(tmp.path(), "print('hello')").unwrap();
        let result = extract_text_with_lang(tmp.path(), "py").unwrap();
        assert_eq!(result.language.as_deref(), Some("python"));
        assert!(result.content.contains("print"));
    }

    #[test]
    fn test_extract_log() {
        let tmp = tempfile::NamedTempFile::with_suffix(".log").unwrap();
        std::fs::write(tmp.path(), "INFO ok\nERROR fail\nINFO done").unwrap();
        let result = extract_log(tmp.path()).unwrap();
        assert!(result.content.contains("1 errors in tail"));
        assert_eq!(result.metadata.get("total_lines").map(|s| s.as_str()), Some("3"));
    }

    #[test]
    fn test_binary_fallback_on_text() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "plain text content").unwrap();
        let result = extract_binary_fallback(tmp.path(), "unknown").unwrap();
        assert!(result.content.contains("plain text content"));
    }

    #[test]
    fn test_extract_cert_fallback() {
        let tmp = tempfile::NamedTempFile::with_suffix(".pem").unwrap();
        std::fs::write(tmp.path(), "-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----").unwrap();
        let result = extract_cert(tmp.path()).unwrap();
        assert!(result.content.contains("CERTIFICATE"));
    }
}
