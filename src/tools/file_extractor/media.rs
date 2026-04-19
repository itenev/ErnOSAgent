//! Media extractors — images, audio, video.

use anyhow::Result;
use std::path::Path;

use super::{ExtractionResult, make_result};

/// Extract image as base64 data URL for vision model consumption.
pub fn extract_image(path: &Path, ext: &str) -> Result<ExtractionResult> {
    let bytes = std::fs::read(path)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

    let mime = match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tiff" | "tif" => "image/tiff",
        "ico" => "image/x-icon",
        "heic" | "heif" => "image/heic",
        "avif" => "image/avif",
        _ => "image/png",
    };

    let data_url = format!("data:{};base64,{}", mime, &b64[..b64.len().min(100000)]);
    let size_kb = bytes.len() / 1024;

    let mut result = make_result(
        format!("[Image: {} — {}KB]\n\n{}", ext.to_uppercase(), size_kb, data_url),
        mime,
    );
    result.image_data_urls.push(data_url);
    Ok(result)
}

/// Extract audio — try whisper transcription, fall back to ffprobe metadata.
pub fn extract_audio(path: &Path) -> Result<ExtractionResult> {
    if let Some(text) = try_whisper_transcription(path) {
        return Ok(make_result(format!("[Audio Transcription]\n\n{}", text), "audio/mpeg"));
    }

    if let Some(info) = try_ffprobe_info(path) {
        return Ok(make_result(
            format!("[Audio — no transcription available]\nffprobe info:\n{}", info),
            "audio/mpeg",
        ));
    }

    Ok(make_result(
        format!("[Audio file: {} — transcription requires whisper-cpp or ffprobe]", path.display()),
        "audio/mpeg",
    ))
}

/// Extract video metadata via ffprobe.
pub fn extract_video(path: &Path) -> Result<ExtractionResult> {
    let info = try_ffprobe_info(path).unwrap_or_else(|| "(ffprobe not available)".to_string());
    let size_mb = std::fs::metadata(path).map(|m| m.len() / (1024 * 1024)).unwrap_or(0);
    Ok(make_result(
        format!("[Video: {}MB]\n\nffprobe:\n{}", size_mb, info),
        "video/mp4",
    ))
}

/// Try whisper-cpp or python whisper for audio transcription.
fn try_whisper_transcription(path: &Path) -> Option<String> {
    // Try whisper.cpp
    if let Ok(output) = std::process::Command::new("whisper-cpp")
        .args(["--model", "base", "--file"])
        .arg(path)
        .output()
    {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).to_string());
        }
    }

    // Try python whisper
    let cmd = format!(
        "import whisper; m = whisper.load_model('base'); r = m.transcribe('{}'); print(r['text'])",
        path.display()
    );
    if let Ok(output) = std::process::Command::new("python3").args(["-c", &cmd]).output() {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).to_string());
        }
    }

    None
}

/// Try ffprobe for media metadata.
fn try_ffprobe_info(path: &Path) -> Option<String> {
    let output = std::process::Command::new("ffprobe")
        .args(["-show_format", "-show_streams", "-print_format", "json", "-v", "quiet"])
        .arg(path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}
