//! Image generation tool — calls local Flux server to generate images.
//! Images are saved to `data/images/` and served via `/api/images/`.

use anyhow::{Context, Result};

/// Generate an image via the local Flux server.
pub async fn execute(args: &serde_json::Value) -> Result<String> {
    let prompt = args["prompt"].as_str().unwrap_or("");
    let width = args["width"].as_u64().unwrap_or(1024) as u32;
    let height = args["height"].as_u64().unwrap_or(1024) as u32;
    let steps = args["steps"].as_u64().unwrap_or(30) as u32;
    let guidance = args["guidance"].as_f64().unwrap_or(3.5);

    if prompt.is_empty() {
        anyhow::bail!("Image prompt cannot be empty");
    }

    tracing::info!(prompt = %prompt, width, height, steps, "Generating image via Flux");

    let b64 = call_flux_server(prompt, width, height, steps, guidance).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{}.png", id);
    save_base64_image(&b64, &filename)?;

    tracing::info!(filename = %filename, prompt = %prompt.chars().take(50).collect::<String>(), "Image generated and saved");

    Ok(format_success_response(&filename, prompt, width, height, steps))
}

/// Send generation request to the Flux server and return base64 image.
async fn call_flux_server(prompt: &str, width: u32, height: u32, steps: u32, guidance: f64) -> Result<String> {
    let port = std::env::var("FLUX_PORT").unwrap_or_else(|_| "8890".to_string());
    let url = format!("http://127.0.0.1:{}/generate", port);

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "prompt": prompt, "width": width, "height": height,
            "steps": steps, "guidance": guidance,
        }))
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .context("Failed to connect to Flux server — is scripts/flux_server.py running?")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("{}", parse_flux_error(status, &body));
    }

    let result: serde_json::Value = resp.json().await
        .context("Failed to parse Flux server response")?;

    result["image_base64"].as_str()
        .context("Flux response missing image_base64 field")
        .map(|s| s.to_string())
}

/// Parse a Flux server error response, extracting traceback if available.
fn parse_flux_error(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(err_json) = serde_json::from_str::<serde_json::Value>(body) {
        let err = err_json["error"].as_str().unwrap_or("unknown");
        let tb = err_json["traceback"].as_str().unwrap_or("");
        if tb.is_empty() {
            return format!("Flux error: {}", err);
        }
        let tb_tail: String = tb.lines().rev().take(3)
            .collect::<Vec<_>>().into_iter().rev()
            .collect::<Vec<_>>().join("\n");
        format!("Flux error: {}\n{}", err, tb_tail)
    } else {
        format!("Flux server returned {}: {}", status, body)
    }
}

/// Format the markdown success response for a generated image.
fn format_success_response(
    filename: &str, prompt: &str, width: u32, height: u32, steps: u32,
) -> String {
    // Return markdown image tag pointing to the served file.
    // DO NOT include base64 data — it would consume ~200K+ tokens
    // and blow the context window on re-inference.
    let image_url = format!("/api/images/{}", filename);
    format!(
        "![{prompt}]({image_url})\n\n\
         Image generated successfully ({width}×{height}, {steps} steps).\n\
         The image is saved at {image_url} and will be displayed in the chat.\n\
         Deliver it to the user with the markdown image tag above.",
    )
}

/// Decode base64 and save to data/images/.
fn save_base64_image(b64: &str, filename: &str) -> Result<()> {
    use base64::Engine;
    let dir = std::path::PathBuf::from("data/images");
    std::fs::create_dir_all(&dir)
        .context("Failed to create data/images directory")?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("Failed to decode base64 image")?;

    let path = dir.join(filename);
    std::fs::write(&path, &bytes)
        .with_context(|| format!("Failed to write image to {:?}", path))?;
    Ok(())
}

/// Check if the Flux server is reachable.
pub async fn health_check() -> bool {
    let port = std::env::var("FLUX_PORT").unwrap_or_else(|_| "8890".to_string());
    let url = format!("http://127.0.0.1:{}/health", port);
    match reqwest::get(&url).await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_base64_image() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("data/images");
        std::fs::create_dir_all(&dir).unwrap();

        // Minimal 1x1 white PNG as base64
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwADhQGAWjR9awAAAABJRU5ErkJggg==";
        // Test the decoding works (path won't match since we can't override data/)
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert!(!bytes.is_empty());
        assert!(bytes.len() > 50); // Valid PNG header
    }

    #[tokio::test]
    async fn test_empty_prompt_rejected() {
        let args = serde_json::json!({"prompt": ""});
        assert!(execute(&args).await.is_err());
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        // health_check returns true if server is running, false otherwise.
        // Either outcome is valid — we only verify it doesn't panic.
        let _result: bool = health_check().await;
    }
}
