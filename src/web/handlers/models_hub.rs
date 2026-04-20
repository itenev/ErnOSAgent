//! HuggingFace model hub — search and download GGUF models.

use crate::web::state::AppState;
use axum::{extract::{State, Query}, response::IntoResponse, Json};

/// Search HuggingFace for GGUF models.
pub async fn search_hf(Query(params): Query<std::collections::HashMap<String, String>>) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return Json(serde_json::json!({"models": [], "error": "query parameter 'q' required"}));
    }

    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=20",
        urlencoding::encode(&query)
    );

    match reqwest::get(&url).await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(data) => {
                let models: Vec<serde_json::Value> = data.as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|m| serde_json::json!({
                        "id": m["modelId"],
                        "author": m["author"],
                        "downloads": m["downloads"],
                        "likes": m["likes"],
                        "tags": m["tags"],
                        "last_modified": m["lastModified"],
                    }))
                    .collect();
                Json(serde_json::json!({"models": models, "count": models.len()}))
            }
            Err(e) => Json(serde_json::json!({"models": [], "error": format!("Parse error: {}", e)})),
        },
        Err(e) => Json(serde_json::json!({"models": [], "error": format!("Network error: {}", e)})),
    }
}

/// Start downloading a model from HuggingFace to ./models/.
pub async fn start_download(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let repo = body["repo"].as_str().unwrap_or("").to_string();
    let filename = body["filename"].as_str().unwrap_or("").to_string();

    if repo.is_empty() || filename.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "repo and filename required"})));
    }

    let progress_path = state.config.general.data_dir.join("model_download_progress.json");
    let models_dir = std::path::PathBuf::from("./models");
    let _ = tokio::fs::create_dir_all(&models_dir).await;

    let repo_ret = repo.clone();
    let filename_ret = filename.clone();

    // Spawn background download task
    tokio::spawn(async move {
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo, filename);
        let dest = models_dir.join(&filename);

        let progress = serde_json::json!({
            "downloading": true, "model": &filename, "repo": &repo,
            "progress": 0.0, "total_bytes": 0, "downloaded_bytes": 0,
        });
        let _ = tokio::fs::write(&progress_path, serde_json::to_string(&progress).unwrap()).await;

        match reqwest::get(&url).await {
            Ok(resp) => {
                let total = resp.content_length().unwrap_or(0);
                let mut file = match tokio::fs::File::create(&dest).await {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to create model file");
                        let _ = tokio::fs::remove_file(&progress_path).await;
                        return;
                    }
                };

                let mut downloaded: u64 = 0;
                let mut stream = resp.bytes_stream();
                use futures_util::StreamExt;
                use tokio::io::AsyncWriteExt;

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(bytes) => {
                            downloaded += bytes.len() as u64;
                            let _ = file.write_all(&bytes).await;
                            let pct = if total > 0 { downloaded as f64 / total as f64 } else { 0.0 };
                            let progress = serde_json::json!({
                                "downloading": true, "model": &filename, "repo": &repo,
                                "progress": pct, "total_bytes": total, "downloaded_bytes": downloaded,
                            });
                            let _ = tokio::fs::write(&progress_path, serde_json::to_string(&progress).unwrap()).await;
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Download chunk error");
                            break;
                        }
                    }
                }

                let _ = file.flush().await;
                let done = serde_json::json!({"downloading": false, "model": &filename, "complete": true});
                let _ = tokio::fs::write(&progress_path, serde_json::to_string(&done).unwrap()).await;
                tracing::info!(model = %filename, bytes = downloaded, "Model download complete");
            }
            Err(e) => {
                tracing::error!(error = %e, "Model download failed");
                let _ = tokio::fs::remove_file(&progress_path).await;
            }
        }
    });

    (axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({"status": "downloading", "repo": repo_ret, "filename": filename_ret})))
}
