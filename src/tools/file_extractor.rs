//! Universal file extraction — detects file type and extracts readable content.
//!
//! Supports: PDF, DOCX, XLSX, CSV, JSON, YAML, TOML, XML, HTML, RTF, EPUB, PPTX,
//! images (→ vision), audio (→ transcription), archives, SQLite, email, logs, binary.

use anyhow::{Context, Result};
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

    let size = std::fs::metadata(p)
        .map(|m| m.len())
        .unwrap_or(0);

    tracing::info!(path = %path, ext = %ext, size, "FileExtractor: extracting");

    match ext.as_str() {
        // ─── Documents ───
        "pdf" => extract_pdf(p),
        "docx" => extract_docx(p),
        "doc" => extract_legacy_doc(p),
        "rtf" => extract_rtf(p),
        "odt" => extract_odt(p),
        "epub" => extract_epub(p),
        "pptx" => extract_pptx(p),
        "ppt" => extract_legacy_ppt(p),
        "html" | "htm" => extract_html(p),
        "pages" => extract_pages(p),

        // ─── Spreadsheets & Data ───
        "xlsx" | "xls" | "ods" => extract_spreadsheet(p),
        "csv" | "tsv" => extract_csv(p, &ext),
        "json" => extract_json(p),
        "jsonl" | "ndjson" => extract_jsonl(p),
        "yaml" | "yml" => extract_yaml(p),
        "toml" => extract_toml(p),
        "xml" => extract_xml(p),
        "ini" | "cfg" | "env" | "conf" => extract_ini(p),
        "sqlite" | "db" | "sqlite3" => extract_sqlite(p),

        // ─── Images → vision ───
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif"
        | "svg" | "ico" | "heic" | "heif" | "avif" => extract_image(p, &ext),

        // ─── Audio → transcription ───
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" => extract_audio(p),

        // ─── Video → keyframes ───
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" => extract_video(p),

        // ─── Archives ───
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" => extract_archive(p, &ext),

        // ─── Communication ───
        "eml" | "msg" => extract_email(p),
        "ics" => extract_calendar(p),
        "vcf" => extract_vcard(p),

        // ─── Log files ───
        "log" => extract_log(p),

        // ─── Certificates ───
        "pem" | "crt" | "key" | "cer" => extract_cert(p),

        // ─── Code & markup (read as text with language tag) ───
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp" | "cc"
        | "h" | "hpp" | "rb" | "swift" | "kt" | "kts" | "cs" | "php" | "lua" | "r"
        | "scala" | "zig" | "nim" | "v" | "dart" | "ex" | "exs" | "erl" | "hs" | "ml"
        | "clj" | "lisp" | "el" | "vim" | "ps1" | "fish" | "nu"
        | "sh" | "bash" | "zsh"
        | "sql" | "graphql" | "proto" | "thrift"
        | "tf" | "hcl" | "nix" | "dhall"
        | "md" | "mdx" | "rst" | "adoc" | "tex" | "org" | "txt" | "text"
        | "dockerfile" | "containerfile" => extract_text_with_lang(p, &ext),

        // Known config (text)
        "makefile" | "justfile" | "rakefile" | "gemfile" | "procfile"
        | "gitignore" | "dockerignore" | "editorconfig" => extract_text_with_lang(p, &ext),

        // ─── Binary fallback ───
        _ => extract_binary_fallback(p, &ext),
    }
}

fn make_result(content: String, mime: &str) -> ExtractionResult {
    ExtractionResult {
        content,
        mime_type: mime.to_string(),
        metadata: HashMap::new(),
        image_data_urls: vec![],
        language: None,
    }
}

fn make_result_with_meta(content: String, mime: &str, meta: HashMap<String, String>) -> ExtractionResult {
    ExtractionResult {
        content,
        mime_type: mime.to_string(),
        metadata: meta,
        image_data_urls: vec![],
        language: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Document extractors
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
fn extract_pdf(path: &Path) -> Result<ExtractionResult> {
    use lopdf::Document;

    let doc = Document::load(path)
        .with_context(|| format!("Failed to load PDF: {}", path.display()))?;

    let pages = doc.get_pages();
    let page_count = pages.len();
    let mut all_text = String::new();

    for (page_num, _page_id) in &pages {
        match doc.extract_text(&[*page_num]) {
            Ok(text) => {
                if !text.trim().is_empty() {
                    all_text.push_str(&format!("--- Page {} ---\n{}\n\n", page_num, text.trim()));
                }
            }
            Err(_) => {
                all_text.push_str(&format!("--- Page {} --- (no extractable text)\n\n", page_num));
            }
        }
    }

    // If text extraction yielded very little, try OCR via tesseract
    let text_chars: usize = all_text.chars().filter(|c| c.is_alphanumeric()).count();
    let chars_per_page = if page_count > 0 { text_chars / page_count } else { 0 };

    if chars_per_page < 50 {
        // Likely scanned/image PDF — try OCR
        if let Ok(ocr_text) = run_tesseract_ocr(path) {
            if !ocr_text.trim().is_empty() {
                all_text.push_str("\n--- OCR Extracted Text ---\n");
                all_text.push_str(&ocr_text);
            }
        }
    }

    if all_text.trim().is_empty() {
        all_text = format!("(PDF has {} pages but no extractable text — may be image-only without OCR)", page_count);
    }

    let mut meta = HashMap::new();
    meta.insert("pages".into(), page_count.to_string());
    meta.insert("chars_per_page".into(), chars_per_page.to_string());

    Ok(make_result_with_meta(
        format!("[PDF: {} pages]\n\n{}", page_count, all_text),
        "application/pdf",
        meta,
    ))
}

#[cfg(not(feature = "file-extract"))]
fn extract_pdf(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("(PDF extraction requires file-extract feature: {})", path.display()),
        "application/pdf",
    ))
}

/// Run tesseract OCR on a PDF (converts to images first via ghostscript/pdftoppm).
fn run_tesseract_ocr(path: &Path) -> Result<String> {
    // Try pdftoppm + tesseract pipeline
    let tmp = std::env::temp_dir().join(format!("ernos_ocr_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp)?;

    // Convert PDF pages to images
    let ppm_result = std::process::Command::new("pdftoppm")
        .args(["-png", "-r", "300"])
        .arg(path)
        .arg(tmp.join("page").to_str().unwrap_or("page"))
        .output();

    if ppm_result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
        anyhow::bail!("pdftoppm not available for OCR");
    }

    let mut all_text = String::new();
    let mut entries: Vec<_> = std::fs::read_dir(&tmp)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "png"))
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let ocr = std::process::Command::new("tesseract")
            .arg(entry.path())
            .arg("stdout")
            .output();
        if let Ok(output) = ocr {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if !text.trim().is_empty() {
                    all_text.push_str(text.trim());
                    all_text.push_str("\n\n");
                }
            }
        }
    }

    let _ = std::fs::remove_dir_all(&tmp);
    Ok(all_text)
}

#[cfg(feature = "file-extract")]
fn extract_docx(path: &Path) -> Result<ExtractionResult> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .context("Failed to open DOCX (not a valid ZIP)")?;

    let mut content = String::new();
    let mut tables_found = 0;

    // Extract document.xml (main body)
    if let Ok(mut doc_xml) = archive.by_name("word/document.xml") {
        let mut xml_str = String::new();
        std::io::Read::read_to_string(&mut doc_xml, &mut xml_str)?;
        let (text, tables) = parse_docx_xml(&xml_str);
        content.push_str(&text);
        tables_found = tables;
    }

    // Extract headers
    for i in 1..=3 {
        let name = format!("word/header{}.xml", i);
        if let Ok(mut header) = archive.by_name(&name) {
            let mut xml_str = String::new();
            std::io::Read::read_to_string(&mut header, &mut xml_str)?;
            let (text, _) = parse_docx_xml(&xml_str);
            if !text.trim().is_empty() {
                content = format!("[Header]\n{}\n\n{}", text.trim(), content);
            }
        }
    }

    // Extract footers
    for i in 1..=3 {
        let name = format!("word/footer{}.xml", i);
        if let Ok(mut footer) = archive.by_name(&name) {
            let mut xml_str = String::new();
            std::io::Read::read_to_string(&mut footer, &mut xml_str)?;
            let (text, _) = parse_docx_xml(&xml_str);
            if !text.trim().is_empty() {
                content.push_str(&format!("\n[Footer]\n{}\n", text.trim()));
            }
        }
    }

    // Extract footnotes
    if let Ok(mut footnotes) = archive.by_name("word/footnotes.xml") {
        let mut xml_str = String::new();
        std::io::Read::read_to_string(&mut footnotes, &mut xml_str)?;
        let (text, _) = parse_docx_xml(&xml_str);
        if !text.trim().is_empty() {
            content.push_str(&format!("\n[Footnotes]\n{}\n", text.trim()));
        }
    }

    let mut meta = HashMap::new();
    meta.insert("tables".into(), tables_found.to_string());

    Ok(make_result_with_meta(
        format!("[DOCX]\n\n{}", content),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        meta,
    ))
}

#[cfg(not(feature = "file-extract"))]
fn extract_docx(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(DOCX extraction requires file-extract feature: {})", path.display()), "application/docx"))
}

/// Parse DOCX XML — extracts paragraphs and tables as markdown.
#[cfg(feature = "file-extract")]
fn parse_docx_xml(xml: &str) -> (String, usize) {
    let mut text = String::new();
    let mut tables = 0;
    let mut in_table = false;
    let mut in_row = false;
    let mut row_cells: Vec<String> = Vec::new();
    let mut current_cell = String::new();
    let mut in_para = false;
    let mut current_para = String::new();
    let mut in_text = false;

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) | Ok(quick_xml::events::Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "w:tbl" => { in_table = true; tables += 1; }
                    "w:tr" => { in_row = true; row_cells.clear(); }
                    "w:tc" => { current_cell.clear(); }
                    "w:p" => { in_para = true; current_para.clear(); }
                    "w:t" => { in_text = true; }
                    "w:tab" => { current_para.push('\t'); }
                    "w:br" => { current_para.push('\n'); }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Text(ref e)) => {
                if in_text {
                    let t = e.unescape().unwrap_or_default();
                    if in_table {
                        current_cell.push_str(&t);
                    } else {
                        current_para.push_str(&t);
                    }
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "w:t" => { in_text = false; }
                    "w:p" => {
                        in_para = false;
                        if !in_table && !current_para.trim().is_empty() {
                            text.push_str(current_para.trim());
                            text.push('\n');
                        }
                        if in_table {
                            current_cell.push_str(current_para.trim());
                            current_cell.push(' ');
                        }
                    }
                    "w:tc" => {
                        row_cells.push(current_cell.trim().to_string());
                    }
                    "w:tr" => {
                        in_row = false;
                        text.push_str("| ");
                        text.push_str(&row_cells.join(" | "));
                        text.push_str(" |\n");
                    }
                    "w:tbl" => {
                        in_table = false;
                        text.push('\n');
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (text, tables)
}

fn extract_legacy_doc(path: &Path) -> Result<ExtractionResult> {
    // Try antiword, then catdoc
    for cmd in &["antiword", "catdoc"] {
        if let Ok(output) = std::process::Command::new(cmd).arg(path).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                return Ok(make_result(format!("[DOC via {}]\n\n{}", cmd, text), "application/msword"));
            }
        }
    }
    Ok(make_result(format!("(Legacy .doc requires antiword or catdoc: {})", path.display()), "application/msword"))
}

fn extract_rtf(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    // Strip RTF control words — simple approach
    let mut text = String::new();
    let mut in_group = 0i32;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' { in_group += 1; }
        else if c == '}' { in_group -= 1; }
        else if c == '\\' {
            // Skip control word
            i += 1;
            while i < chars.len() && chars[i].is_ascii_alphabetic() { i += 1; }
            if i < chars.len() && chars[i] == ' ' { i += 1; }
            continue;
        } else if in_group <= 1 && c != '\r' {
            text.push(c);
        }
        i += 1;
    }
    Ok(make_result(format!("[RTF]\n\n{}", text.trim()), "application/rtf"))
}

fn extract_odt(path: &Path) -> Result<ExtractionResult> {
    extract_zip_xml(path, "content.xml", "ODT")
}

#[cfg(feature = "file-extract")]
fn extract_epub(path: &Path) -> Result<ExtractionResult> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut content = String::new();
    let mut chapter = 0;

    for i in 0..archive.len() {
        if let Ok(mut entry) = archive.by_index(i) {
            let name = entry.name().to_string();
            if name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm") {
                chapter += 1;
                let mut html = String::new();
                std::io::Read::read_to_string(&mut entry, &mut html)?;
                let text = strip_html_tags(&html);
                if !text.trim().is_empty() {
                    content.push_str(&format!("--- Chapter {} ---\n{}\n\n", chapter, text.trim()));
                }
            }
        }
    }

    let mut meta = HashMap::new();
    meta.insert("chapters".into(), chapter.to_string());
    Ok(make_result_with_meta(format!("[EPUB: {} chapters]\n\n{}", chapter, content), "application/epub+zip", meta))
}

#[cfg(not(feature = "file-extract"))]
fn extract_epub(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(EPUB requires file-extract: {})", path.display()), "application/epub+zip"))
}

#[cfg(feature = "file-extract")]
fn extract_pptx(path: &Path) -> Result<ExtractionResult> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut content = String::new();
    let mut slide_num = 0;

    for i in 0..archive.len() {
        if let Ok(mut entry) = archive.by_index(i) {
            let name = entry.name().to_string();
            if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                slide_num += 1;
                let mut xml = String::new();
                std::io::Read::read_to_string(&mut entry, &mut xml)?;
                let text = extract_text_from_xml(&xml);
                if !text.trim().is_empty() {
                    content.push_str(&format!("--- Slide {} ---\n{}\n\n", slide_num, text.trim()));
                }
            }
            // Speaker notes
            if name.starts_with("ppt/notesSlides/") && name.ends_with(".xml") {
                let mut xml = String::new();
                std::io::Read::read_to_string(&mut entry, &mut xml)?;
                let text = extract_text_from_xml(&xml);
                if !text.trim().is_empty() {
                    content.push_str(&format!("[Speaker Notes]\n{}\n\n", text.trim()));
                }
            }
        }
    }

    let mut meta = HashMap::new();
    meta.insert("slides".into(), slide_num.to_string());
    Ok(make_result_with_meta(format!("[PPTX: {} slides]\n\n{}", slide_num, content), "application/vnd.openxmlformats-officedocument.presentationml.presentation", meta))
}

#[cfg(not(feature = "file-extract"))]
fn extract_pptx(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(PPTX requires file-extract: {})", path.display()), "application/pptx"))
}

fn extract_legacy_ppt(path: &Path) -> Result<ExtractionResult> {
    if let Ok(output) = std::process::Command::new("catppt").arg(path).output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(make_result(format!("[PPT via catppt]\n\n{}", text), "application/vnd.ms-powerpoint"));
        }
    }
    Ok(make_result(format!("(Legacy .ppt requires catppt: {})", path.display()), "application/vnd.ms-powerpoint"))
}

fn extract_html(path: &Path) -> Result<ExtractionResult> {
    let html = std::fs::read_to_string(path)?;
    let text = strip_html_tags(&html);
    Ok(make_result(format!("[HTML]\n\n{}", text), "text/html"))
}

fn extract_pages(path: &Path) -> Result<ExtractionResult> {
    extract_zip_xml(path, "index.xml", "Pages")
}

// ═══════════════════════════════════════════════════════════════════════
// Spreadsheet & Data extractors
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
fn extract_spreadsheet(path: &Path) -> Result<ExtractionResult> {
    use calamine::{Reader, open_workbook_auto};

    let mut workbook = open_workbook_auto(path)
        .with_context(|| format!("Failed to open spreadsheet: {}", path.display()))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
    let mut content = String::new();

    for name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(name) {
            content.push_str(&format!("--- Sheet: {} ---\n", name));
            let mut row_count = 0;
            for row in range.rows() {
                let cells: Vec<String> = row.iter().map(|c| format!("{}", c)).collect();
                content.push_str("| ");
                content.push_str(&cells.join(" | "));
                content.push_str(" |\n");
                row_count += 1;
                // Separator after header row
                if row_count == 1 {
                    let sep: Vec<&str> = cells.iter().map(|_| "---").collect();
                    content.push_str("| ");
                    content.push_str(&sep.join(" | "));
                    content.push_str(" |\n");
                }
                if row_count > 200 {
                    content.push_str(&format!("... ({} more rows)\n", range.rows().count() - 200));
                    break;
                }
            }
            content.push('\n');
        }
    }

    let mut meta = HashMap::new();
    meta.insert("sheets".into(), sheet_names.len().to_string());
    Ok(make_result_with_meta(
        format!("[Spreadsheet: {} sheets]\n\n{}", sheet_names.len(), content),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        meta,
    ))
}

#[cfg(not(feature = "file-extract"))]
fn extract_spreadsheet(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(Spreadsheet requires file-extract: {})", path.display()), "application/spreadsheet"))
}

#[cfg(feature = "file-extract")]
fn extract_csv(path: &Path, ext: &str) -> Result<ExtractionResult> {
    let delimiter = if ext == "tsv" { b'\t' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)?;

    let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
    let mut content = String::new();

    // Header row
    content.push_str("| ");
    content.push_str(&headers.join(" | "));
    content.push_str(" |\n| ");
    content.push_str(&headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    content.push_str(" |\n");

    let mut count = 0;
    for record in rdr.records() {
        if let Ok(rec) = record {
            let cells: Vec<&str> = rec.iter().collect();
            content.push_str("| ");
            content.push_str(&cells.join(" | "));
            content.push_str(" |\n");
            count += 1;
            if count >= 500 { break; }
        }
    }

    let mut meta = HashMap::new();
    meta.insert("columns".into(), headers.len().to_string());
    meta.insert("rows_shown".into(), count.to_string());
    Ok(make_result_with_meta(
        format!("[CSV: {} columns, {} rows]\n\n{}", headers.len(), count, content),
        "text/csv", meta,
    ))
}

#[cfg(not(feature = "file-extract"))]
fn extract_csv(path: &Path, _ext: &str) -> Result<ExtractionResult> {
    // Fallback: read as text
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(content, "text/csv"))
}

fn extract_json(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let val: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or(serde_json::Value::String(raw.clone()));
    let pretty = serde_json::to_string_pretty(&val).unwrap_or(raw);
    Ok(make_result(format!("[JSON]\n\n{}", pretty), "application/json"))
}

fn extract_jsonl(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().collect();
    let total = lines.len();
    let sample: Vec<&str> = lines.iter().take(20).copied().collect();
    Ok(make_result(
        format!("[JSONL: {} lines]\n\nFirst {}:\n{}", total, sample.len(), sample.join("\n")),
        "application/x-ndjson",
    ))
}

fn extract_yaml(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[YAML]\n\n{}", content), "application/yaml"))
}

fn extract_toml(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[TOML]\n\n{}", content), "application/toml"))
}

#[cfg(feature = "file-extract")]
fn extract_xml(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let text = extract_text_from_xml(&raw);
    if text.trim().is_empty() {
        // Return raw XML if no text extracted
        Ok(make_result(format!("[XML]\n\n{}", &raw[..raw.len().min(50000)]), "application/xml"))
    } else {
        Ok(make_result(format!("[XML]\n\n{}", text), "application/xml"))
    }
}

#[cfg(not(feature = "file-extract"))]
fn extract_xml(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(content, "application/xml"))
}

fn extract_ini(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[Config]\n\n{}", content), "text/plain"))
}

#[cfg(feature = "file-extract")]
fn extract_sqlite(path: &Path) -> Result<ExtractionResult> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;

    let mut content = String::new();

    // Get table names
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
    let tables: Vec<String> = stmt.query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    content.push_str(&format!("[SQLite: {} tables]\n\n", tables.len()));

    for table in &tables {
        // Schema
        let schema_sql = format!("SELECT sql FROM sqlite_master WHERE name = '{}'", table);
        if let Ok(schema) = conn.query_row(&schema_sql, [], |row| row.get::<_, String>(0)) {
            content.push_str(&format!("--- {} ---\n{}\n\n", table, schema));
        }

        // Sample rows (first 10)
        let query = format!("SELECT * FROM \"{}\" LIMIT 10", table);
        if let Ok(mut stmt) = conn.prepare(&query) {
            let col_count = stmt.column_count();
            let col_names: Vec<String> = (0..col_count).map(|i| stmt.column_name(i).unwrap_or("?").to_string()).collect();
            content.push_str("| ");
            content.push_str(&col_names.join(" | "));
            content.push_str(" |\n| ");
            content.push_str(&col_names.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
            content.push_str(" |\n");

            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    let cells: Vec<String> = (0..col_count).map(|i| {
                        row.get::<_, String>(i).unwrap_or_else(|_| "NULL".to_string())
                    }).collect();
                    content.push_str("| ");
                    content.push_str(&cells.join(" | "));
                    content.push_str(" |\n");
                }
            }
            content.push('\n');
        }
    }

    let mut meta = HashMap::new();
    meta.insert("tables".into(), tables.len().to_string());
    Ok(make_result_with_meta(content, "application/x-sqlite3", meta))
}

#[cfg(not(feature = "file-extract"))]
fn extract_sqlite(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(SQLite requires file-extract: {})", path.display()), "application/x-sqlite3"))
}

// ═══════════════════════════════════════════════════════════════════════
// Image / Audio / Video
// ═══════════════════════════════════════════════════════════════════════

fn extract_image(path: &Path, ext: &str) -> Result<ExtractionResult> {
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

fn extract_audio(path: &Path) -> Result<ExtractionResult> {
    // Try whisper.cpp first, then ffmpeg
    let whisper_result = std::process::Command::new("whisper-cpp")
        .args(["--model", "base", "--file"])
        .arg(path)
        .output();

    if let Ok(output) = whisper_result {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(make_result(format!("[Audio Transcription]\n\n{}", text), "audio/mpeg"));
        }
    }

    // Try python whisper
    let py_result = std::process::Command::new("python3")
        .args(["-c", &format!(
            "import whisper; m = whisper.load_model('base'); r = m.transcribe('{}'); print(r['text'])",
            path.display()
        )])
        .output();

    if let Ok(output) = py_result {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(make_result(format!("[Audio Transcription]\n\n{}", text), "audio/mpeg"));
        }
    }

    // Fallback: file info via ffprobe
    if let Ok(output) = std::process::Command::new("ffprobe")
        .args(["-show_format", "-show_streams", "-print_format", "json"])
        .arg(path)
        .output()
    {
        if output.status.success() {
            let info = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(make_result(format!("[Audio — no transcription available]\nffprobe info:\n{}", info), "audio/mpeg"));
        }
    }

    Ok(make_result(
        format!("[Audio file: {} — transcription requires whisper-cpp or ffprobe]", path.display()),
        "audio/mpeg",
    ))
}

fn extract_video(path: &Path) -> Result<ExtractionResult> {
    // Get video info via ffprobe
    let info = if let Ok(output) = std::process::Command::new("ffprobe")
        .args(["-show_format", "-show_streams", "-print_format", "json", "-v", "quiet"])
        .arg(path)
        .output()
    {
        if output.status.success() {
            String::from_utf8_lossy(&output.stdout).to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let size_mb = std::fs::metadata(path).map(|m| m.len() / (1024 * 1024)).unwrap_or(0);
    Ok(make_result(
        format!("[Video: {}MB]\n\nffprobe:\n{}", size_mb, if info.is_empty() { "(ffprobe not available)".to_string() } else { info }),
        "video/mp4",
    ))
}

// ═══════════════════════════════════════════════════════════════════════
// Archives
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
fn extract_archive(path: &Path, ext: &str) -> Result<ExtractionResult> {
    if ext == "zip" {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let mut listing = String::new();
        let total = archive.len();

        for i in 0..total.min(200) {
            if let Ok(entry) = archive.by_index(i) {
                listing.push_str(&format!("  {} ({} bytes)\n", entry.name(), entry.size()));
            }
        }

        let mut meta = HashMap::new();
        meta.insert("entries".into(), total.to_string());
        return Ok(make_result_with_meta(
            format!("[ZIP: {} entries]\n\n{}", total, listing),
            "application/zip", meta,
        ));
    }

    // For tar/gz/bz2 — use tar command
    let list_output = std::process::Command::new("tar")
        .args(["tf"])
        .arg(path)
        .output();

    if let Ok(output) = list_output {
        if output.status.success() {
            let listing = String::from_utf8_lossy(&output.stdout);
            let count = listing.lines().count();
            return Ok(make_result(format!("[Archive: {} entries]\n\n{}", count, listing), "application/archive"));
        }
    }

    Ok(make_result(format!("[Archive: {}]", path.display()), "application/archive"))
}

#[cfg(not(feature = "file-extract"))]
fn extract_archive(path: &Path, _ext: &str) -> Result<ExtractionResult> {
    Ok(make_result(format!("(Archive requires file-extract: {})", path.display()), "application/archive"))
}

// ═══════════════════════════════════════════════════════════════════════
// Communication
// ═══════════════════════════════════════════════════════════════════════

fn extract_email(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    // Simple header + body split
    let parts: Vec<&str> = raw.splitn(2, "\n\n").collect();
    let headers = parts.first().unwrap_or(&"");
    let body = parts.get(1).unwrap_or(&"");

    let mut meta = HashMap::new();
    for line in headers.lines() {
        if let Some((key, val)) = line.split_once(": ") {
            let k = key.to_lowercase();
            if matches!(k.as_str(), "from" | "to" | "subject" | "date") {
                meta.insert(k, val.to_string());
            }
        }
    }

    Ok(make_result_with_meta(
        format!("[Email]\nFrom: {}\nTo: {}\nSubject: {}\nDate: {}\n\n{}",
            meta.get("from").unwrap_or(&"?".into()),
            meta.get("to").unwrap_or(&"?".into()),
            meta.get("subject").unwrap_or(&"?".into()),
            meta.get("date").unwrap_or(&"?".into()),
            body,
        ),
        "message/rfc822", meta,
    ))
}

fn extract_calendar(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[Calendar]\n\n{}", content), "text/calendar"))
}

fn extract_vcard(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[vCard]\n\n{}", content), "text/vcard"))
}

// ═══════════════════════════════════════════════════════════════════════
// Log / Cert / Text / Binary
// ═══════════════════════════════════════════════════════════════════════

fn extract_log(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    // Show last 200 lines (tail)
    let start = total.saturating_sub(200);
    let tail: Vec<&str> = lines[start..].to_vec();

    let errors = tail.iter().filter(|l| {
        let lower = l.to_lowercase();
        lower.contains("error") || lower.contains("fatal") || lower.contains("panic")
    }).count();

    let mut meta = HashMap::new();
    meta.insert("total_lines".into(), total.to_string());
    meta.insert("errors_in_tail".into(), errors.to_string());

    Ok(make_result_with_meta(
        format!("[Log: {} lines, {} errors in tail]\n\n{}", total, errors, tail.join("\n")),
        "text/plain", meta,
    ))
}

fn extract_cert(path: &Path) -> Result<ExtractionResult> {
    // Try openssl for certificate info
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

fn extract_text_with_lang(path: &Path, ext: &str) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read: {}", path.display()))?;

    let lang = match ext {
        "rs" => Some("rust"), "py" => Some("python"), "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"), "go" => Some("go"), "java" => Some("java"),
        "c" => Some("c"), "cpp" | "cc" => Some("cpp"), "h" | "hpp" => Some("c"),
        "rb" => Some("ruby"), "swift" => Some("swift"), "kt" | "kts" => Some("kotlin"),
        "cs" => Some("csharp"), "php" => Some("php"), "lua" => Some("lua"),
        "r" => Some("r"), "scala" => Some("scala"), "zig" => Some("zig"),
        "sh" | "bash" | "zsh" | "fish" => Some("bash"),
        "sql" => Some("sql"), "md" | "mdx" => Some("markdown"),
        "html" | "htm" => Some("html"), "css" => Some("css"),
        "dockerfile" | "containerfile" => Some("dockerfile"),
        "yaml" | "yml" => Some("yaml"), "toml" => Some("toml"),
        "json" => Some("json"), "xml" => Some("xml"),
        _ => None,
    };

    let mut result = make_result(content, "text/plain");
    result.language = lang.map(|s| s.to_string());
    Ok(result)
}

fn extract_binary_fallback(path: &Path, ext: &str) -> Result<ExtractionResult> {
    // Try as text first
    if let Ok(content) = std::fs::read_to_string(path) {
        return Ok(make_result(content, "text/plain"));
    }

    // Binary: file type + hex dump
    let bytes = std::fs::read(path)?;
    let size = bytes.len();

    let file_type = std::process::Command::new("file")
        .args(["--brief"])
        .arg(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Hex dump of first 512 bytes
    let hex_len = size.min(512);
    let hex: Vec<String> = bytes[..hex_len].iter().map(|b| format!("{:02x}", b)).collect();
    let hex_display: Vec<String> = hex.chunks(16)
        .map(|chunk| chunk.join(" "))
        .collect();

    Ok(make_result(
        format!("[Binary: {} — {} bytes]\nType: {}\n\nHex dump (first {} bytes):\n{}",
            ext, size, file_type, hex_len, hex_display.join("\n")),
        "application/octet-stream",
    ))
}

// ═══════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════

/// Strip HTML tags from content.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let in_script = false;
    let in_style = false;

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            continue;
        }
        if in_tag {
            // Detect script/style tags
            continue;
        }
        if !in_script && !in_style {
            result.push(c);
        }
    }

    // Clean up excessive whitespace
    let lines: Vec<&str> = result.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("\n")
}

/// Extract text content from any XML by reading all text nodes.
#[cfg(feature = "file-extract")]
fn extract_text_from_xml(xml: &str) -> String {
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

/// Extract text from a ZIP file containing XML (ODT, Pages, etc.)
#[cfg(feature = "file-extract")]
fn extract_zip_xml(path: &Path, xml_name: &str, label: &str) -> Result<ExtractionResult> {
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
        Ok(make_result(format!("[{}: {} not found in archive]", label, xml_name), "application/xml"))
    }
}

#[cfg(not(feature = "file-extract"))]
fn extract_zip_xml(path: &Path, _xml_name: &str, label: &str) -> Result<ExtractionResult> {
    Ok(make_result(format!("({} requires file-extract: {})", label, path.display()), "application/xml"))
}

#[cfg(not(feature = "file-extract"))]
fn extract_text_from_xml(_xml: &str) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = strip_html_tags(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn test_extract_json() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"{"key": "value"}"#).unwrap();
        let result = extract_json(tmp.path()).unwrap();
        assert!(result.content.contains("key"));
        assert!(result.content.contains("value"));
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
    fn test_binary_fallback_on_text() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "plain text content").unwrap();
        let result = extract_binary_fallback(tmp.path(), "unknown").unwrap();
        assert!(result.content.contains("plain text content"));
    }

    #[test]
    fn test_extract_ini() {
        let tmp = tempfile::NamedTempFile::with_suffix(".ini").unwrap();
        std::fs::write(tmp.path(), "[section]\nkey=value").unwrap();
        let result = extract_ini(tmp.path()).unwrap();
        assert!(result.content.contains("key=value"));
    }
}
