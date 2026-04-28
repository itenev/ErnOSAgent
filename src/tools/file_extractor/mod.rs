//! Universal file extraction — detects file type and dispatches to specialized extractors.
//!
//! Supports 40+ file types: documents, spreadsheets, images, audio, archives,
//! databases, email, logs, code, and binary files.

mod documents;
mod data;
mod media;
mod archives;
mod communication;
mod text;

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// Result of extracting content from a file.
pub struct ExtractionResult {
    pub content: String,
    pub mime_type: String,
    pub metadata: HashMap<String, String>,
    pub image_data_urls: Vec<String>,
    pub language: Option<String>,
}

/// Extract readable content from any file type.
pub fn extract(path: &str) -> Result<ExtractionResult> {
    let p = Path::new(path);
    if !p.exists() {
        anyhow::bail!("File not found: {}", path);
    }

    let ext = p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    tracing::info!(path = %path, ext = %ext, size, "FileExtractor: extracting");

    extract_by_extension(p, &ext)
}

/// Route extraction by file extension category.
fn extract_by_extension(p: &Path, ext: &str) -> Result<ExtractionResult> {
    match ext {
        // Documents
        "pdf" => documents::extract_pdf(p),
        "docx" => documents::extract_docx(p),
        "doc" => documents::extract_legacy_doc(p),
        "rtf" => documents::extract_rtf(p),
        "odt" => documents::extract_odt(p),
        "epub" => documents::extract_epub(p),
        "pptx" => documents::extract_pptx(p),
        "ppt" => documents::extract_legacy_ppt(p),
        "html" | "htm" => documents::extract_html(p),
        "pages" => documents::extract_pages(p),
        // Data
        "xlsx" | "xls" | "ods" => data::extract_spreadsheet(p),
        "csv" | "tsv" => data::extract_csv(p, ext),
        "json" => data::extract_json(p),
        "jsonl" | "ndjson" => data::extract_jsonl(p),
        "yaml" | "yml" => data::extract_yaml(p),
        "toml" => data::extract_toml(p),
        "xml" => data::extract_xml(p),
        "ini" | "cfg" | "env" | "conf" => data::extract_ini(p),
        "sqlite" | "db" | "sqlite3" => data::extract_sqlite(p),
        // Media & archives
        _ => extract_media_or_text(p, ext),
    }
}

/// Route media, archive, communication, and text-based files.
fn extract_media_or_text(p: &Path, ext: &str) -> Result<ExtractionResult> {
    match ext {
        // Images
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif"
        | "svg" | "ico" | "heic" | "heif" | "avif" => media::extract_image(p, ext),
        // Audio
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma"
            => media::extract_audio(p),
        // Video
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" => media::extract_video(p),
        // Archives
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar"
            => archives::extract_archive(p, ext),
        // Communication
        "eml" | "msg" => communication::extract_email(p),
        "ics" => communication::extract_calendar(p),
        "vcf" => communication::extract_vcard(p),
        // Logs & certs
        "log" => text::extract_log(p),
        "pem" | "crt" | "key" | "cer" => text::extract_cert(p),
        // Code & text
        _ if is_code_or_text(ext) => text::extract_text_with_lang(p, ext),
        // Binary fallback
        _ => text::extract_binary_fallback(p, ext),
    }
}

/// Check if extension is a known code or text format.
fn is_code_or_text(ext: &str) -> bool {
    matches!(ext,
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp" | "cc"
        | "h" | "hpp" | "rb" | "swift" | "kt" | "kts" | "cs" | "php" | "lua" | "r"
        | "scala" | "zig" | "nim" | "v" | "dart" | "ex" | "exs" | "erl" | "hs" | "ml"
        | "clj" | "lisp" | "el" | "vim" | "ps1" | "fish" | "nu"
        | "sh" | "bash" | "zsh"
        | "sql" | "graphql" | "proto" | "thrift"
        | "tf" | "hcl" | "nix" | "dhall"
        | "md" | "mdx" | "rst" | "adoc" | "tex" | "org" | "txt" | "text"
        | "dockerfile" | "containerfile"
        | "makefile" | "justfile" | "rakefile" | "gemfile" | "procfile"
        | "gitignore" | "dockerignore" | "editorconfig"
    )
}

// ═══════════════════════════════════════════════════════════════════════
// Shared helpers — used by multiple submodules
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn make_result(content: String, mime: &str) -> ExtractionResult {
    ExtractionResult {
        content,
        mime_type: mime.to_string(),
        metadata: HashMap::new(),
        image_data_urls: vec![],
        language: None,
    }
}

pub(crate) fn make_result_with_meta(
    content: String,
    mime: &str,
    meta: HashMap<String, String>,
) -> ExtractionResult {
    ExtractionResult {
        content,
        mime_type: mime.to_string(),
        metadata: meta,
        image_data_urls: vec![],
        language: None,
    }
}

/// Strip HTML tags from content.
pub(crate) fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let in_script = false;
    let in_style = false;

    for c in html.chars() {
        if c == '<' { in_tag = true; continue; }
        if c == '>' { in_tag = false; continue; }
        if in_tag { continue; }
        if !in_script && !in_style { result.push(c); }
    }

    result.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Extract text content from any XML by reading all text nodes.
#[cfg(feature = "file-extract")]
pub(crate) fn extract_text_from_xml(xml: &str) -> String {
    let mut text = String::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Text(ref e)) => {
                if let Ok(t) = e.unescape() {
                    let s = t.trim();
                    if !s.is_empty() {
                        text.push_str(s);
                        text.push('\n');
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    text
}

#[cfg(not(feature = "file-extract"))]
pub(crate) fn extract_text_from_xml(_xml: &str) -> String {
    String::new()
}

/// Extract text from a ZIP file containing XML (ODT, Pages, etc.)
#[cfg(feature = "file-extract")]
pub(crate) fn extract_zip_xml(
    path: &Path,
    xml_name: &str,
    label: &str,
) -> Result<ExtractionResult> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let xml_content = {
        match archive.by_name(xml_name) {
            Ok(mut entry) => {
                let mut xml = String::new();
                std::io::Read::read_to_string(&mut entry, &mut xml)?;
                Some(xml)
            }
            Err(_) => None,
        }
    };

    if let Some(xml) = xml_content {
        let text = extract_text_from_xml(&xml);
        Ok(make_result(format!("[{}]\n\n{}", label, text), "application/xml"))
    } else {
        Ok(make_result(
            format!("[{}: {} not found in archive]", label, xml_name),
            "application/xml",
        ))
    }
}

#[cfg(not(feature = "file-extract"))]
pub(crate) fn extract_zip_xml(
    path: &Path,
    _xml_name: &str,
    label: &str,
) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("({} requires file-extract: {})", label, path.display()),
        "application/xml",
    ))
}
