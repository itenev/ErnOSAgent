//! Archive extractors — ZIP, TAR, GZ, BZ2.

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use super::{ExtractionResult, make_result, make_result_with_meta};

#[cfg(feature = "file-extract")]
pub fn extract_archive(path: &Path, ext: &str) -> Result<ExtractionResult> {
    if ext == "zip" {
        return extract_zip(path);
    }
    extract_tar(path)
}

/// List ZIP archive contents.
#[cfg(feature = "file-extract")]
fn extract_zip(path: &Path) -> Result<ExtractionResult> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let total = archive.len();
    let mut listing = String::new();

    for i in 0..total.min(200) {
        if let Ok(entry) = archive.by_index(i) {
            listing.push_str(&format!("  {} ({} bytes)\n", entry.name(), entry.size()));
        }
    }

    let mut meta = HashMap::new();
    meta.insert("entries".into(), total.to_string());
    Ok(make_result_with_meta(
        format!("[ZIP: {} entries]\n\n{}", total, listing),
        "application/zip",
        meta,
    ))
}

/// List TAR/GZ/BZ2 archive contents via the `tar` command.
#[cfg(feature = "file-extract")]
fn extract_tar(path: &Path) -> Result<ExtractionResult> {
    if let Ok(output) = std::process::Command::new("tar").args(["tf"]).arg(path).output() {
        if output.status.success() {
            let listing = String::from_utf8_lossy(&output.stdout);
            let count = listing.lines().count();
            return Ok(make_result(
                format!("[Archive: {} entries]\n\n{}", count, listing),
                "application/archive",
            ));
        }
    }
    Ok(make_result(format!("[Archive: {}]", path.display()), "application/archive"))
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_archive(path: &Path, _ext: &str) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("(Archive requires file-extract: {})", path.display()),
        "application/archive",
    ))
}
