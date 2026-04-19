//! Upload handler — multipart file upload endpoint.
//! POST /api/upload — accepts any file, saves to data/uploads/, returns path.

use axum::extract::Multipart;
use axum::response::Json;
use serde_json::json;

/// POST /api/upload — accept a file upload, save it, return the path.
pub async fn upload_file(mut multipart: Multipart) -> Json<serde_json::Value> {
    let upload_dir = std::path::PathBuf::from("data/uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        return Json(json!({"error": format!("Failed to create upload dir: {}", e)}));
    }

    let mut uploaded = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name()
            .unwrap_or("unnamed")
            .to_string();

        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(err = %e, "Failed to read upload field");
                continue;
            }
        };

        // Sanitize filename — keep extension, use UUID prefix
        let ext = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        let safe_name = format!("{}_{}.{}", 
            chrono::Utc::now().format("%Y%m%d_%H%M%S"),
            &uuid::Uuid::new_v4().to_string()[..8],
            ext,
        );

        let dest = upload_dir.join(&safe_name);
        if let Err(e) = std::fs::write(&dest, &data) {
            tracing::error!(err = %e, file = %safe_name, "Failed to write upload");
            continue;
        }

        let size = data.len();
        tracing::info!(
            file = %safe_name,
            original = %filename,
            size,
            "File uploaded"
        );

        uploaded.push(json!({
            "original_name": filename,
            "path": dest.display().to_string(),
            "size": size,
        }));
    }

    if uploaded.is_empty() {
        Json(json!({"error": "No files received"}))
    } else {
        Json(json!({"files": uploaded}))
    }
}
