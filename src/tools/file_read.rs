// Ern-OS — File read tool (universal extraction)

use anyhow::{Context, Result};
use tracing;

pub async fn execute(args: &serde_json::Value) -> Result<String> {
    let path = args["path"].as_str().context("file_read requires 'path'")?;
    tracing::info!(path = %path, "file_read START");

    // Use universal file extractor for all file types
    match crate::tools::file_extractor::extract(path) {
        Ok(result) => {
            tracing::info!(
                path = %path,
                mime = %result.mime_type,
                len = result.content.len(),
                lang = ?result.language,
                images = result.image_data_urls.len(),
                "file_read OK"
            );

            // If there are images, include them for vision
            if !result.image_data_urls.is_empty() {
                let mut output = result.content;
                for url in &result.image_data_urls {
                    output.push_str(&format!("\n\n[IMAGE DATA]\n{}", url));
                }
                Ok(output)
            } else {
                Ok(result.content)
            }
        }
        Err(e) => {
            tracing::warn!(path = %path, err = %e, "file_read FAILED");
            Err(e).with_context(|| format!("Failed to read file: {}", path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_existing_file() {
        let args = serde_json::json!({"path": "Cargo.toml"});
        let result = execute(&args).await.unwrap();
        assert!(result.contains("[package]"));
    }

    #[tokio::test]
    async fn test_read_missing_file() {
        let args = serde_json::json!({"path": "/nonexistent/file.txt"});
        assert!(execute(&args).await.is_err());
    }
}
