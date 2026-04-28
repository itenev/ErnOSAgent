//! Document extractors — PDF, DOCX, RTF, ODT, EPUB, PPTX, HTML, Pages.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use super::{ExtractionResult, make_result, make_result_with_meta, strip_html_tags};

// ═══════════════════════════════════════════════════════════════════════
// PDF
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
pub fn extract_pdf(path: &Path) -> Result<ExtractionResult> {
    use lopdf::Document;

    let doc = Document::load(path)
        .with_context(|| format!("Failed to load PDF: {}", path.display()))?;

    let pages = doc.get_pages();
    let page_count = pages.len();
    let mut all_text = String::new();

    for (page_num, _page_id) in &pages {
        match doc.extract_text(&[*page_num]) {
            Ok(text) if !text.trim().is_empty() => {
                all_text.push_str(&format!("--- Page {} ---\n{}\n\n", page_num, text.trim()));
            }
            _ => {
                all_text.push_str(&format!("--- Page {} --- (no extractable text)\n\n", page_num));
            }
        }
    }

    // Hybrid: if text layer is sparse, also try OCR
    let text_chars: usize = all_text.chars().filter(|c| c.is_alphanumeric()).count();
    let chars_per_page = if page_count > 0 { text_chars / page_count } else { 0 };

    if chars_per_page < 50 {
        if let Ok(ocr_text) = try_pdf_ocr(path) {
            if !ocr_text.trim().is_empty() {
                all_text.push_str("\n--- OCR Extracted Text ---\n");
                all_text.push_str(&ocr_text);
            }
        }
    }

    if all_text.trim().is_empty() {
        all_text = format!("(PDF has {} pages but no extractable text)", page_count);
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
pub fn extract_pdf(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("(PDF extraction requires file-extract feature: {})", path.display()),
        "application/pdf",
    ))
}

/// Run OCR on a PDF via pdftoppm + tesseract pipeline.
fn try_pdf_ocr(path: &Path) -> Result<String> {
    let tmp = std::env::temp_dir().join(format!("ernos_ocr_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp)?;

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
        if let Ok(output) = std::process::Command::new("tesseract")
            .arg(entry.path()).arg("stdout").output()
        {
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

// ═══════════════════════════════════════════════════════════════════════
// DOCX
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
pub fn extract_docx(path: &Path) -> Result<ExtractionResult> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .context("Failed to open DOCX (not a valid ZIP)")?;

    let mut content = String::new();
    let mut tables_found = 0;

    content.push_str(&extract_docx_part(&mut archive, "word/document.xml", &mut tables_found)?);
    content = prepend_docx_parts(&mut archive, &content, "header")?;
    content.push_str(&extract_docx_section(&mut archive, "footer")?);
    content.push_str(&extract_docx_section(&mut archive, "footnotes")?);

    let mut meta = HashMap::new();
    meta.insert("tables".into(), tables_found.to_string());

    Ok(make_result_with_meta(
        format!("[DOCX]\n\n{}", content),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        meta,
    ))
}

/// Extract a named XML part from a DOCX archive.
#[cfg(feature = "file-extract")]
fn extract_docx_part(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    tables: &mut usize,
) -> Result<String> {
    let xml = match read_zip_entry(archive, name) {
        Some(xml) => xml,
        None => return Ok(String::new()),
    };
    let (text, t) = parse_docx_xml(&xml);
    *tables += t;
    Ok(text)
}

/// Prepend header sections from the DOCX.
#[cfg(feature = "file-extract")]
fn prepend_docx_parts(
    archive: &mut zip::ZipArchive<std::fs::File>,
    content: &str,
    kind: &str,
) -> Result<String> {
    let mut prefix = String::new();
    for i in 1..=3 {
        let name = format!("word/{}{}.xml", kind, i);
        if let Some(xml) = read_zip_entry(archive, &name) {
            let (text, _) = parse_docx_xml(&xml);
            if !text.trim().is_empty() {
                prefix.push_str(&format!("[{}]\n{}\n\n", kind.to_uppercase(), text.trim()));
            }
        }
    }
    Ok(format!("{}{}", prefix, content))
}

/// Extract footer/footnotes section.
#[cfg(feature = "file-extract")]
fn extract_docx_section(
    archive: &mut zip::ZipArchive<std::fs::File>,
    kind: &str,
) -> Result<String> {
    let name = format!("word/{}.xml", kind);
    if let Some(xml) = read_zip_entry(archive, &name) {
        let (text, _) = parse_docx_xml(&xml);
        if !text.trim().is_empty() {
            return Ok(format!("\n[{}]\n{}\n", kind.to_uppercase(), text.trim()));
        }
    }
    Ok(String::new())
}

/// Read a named entry from a ZIP archive into a String.
#[cfg(feature = "file-extract")]
fn read_zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    match archive.by_name(name) {
        Ok(mut entry) => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut entry, &mut s).ok()?;
            Some(s)
        }
        Err(_) => None,
    }
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_docx(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("(DOCX extraction requires file-extract feature: {})", path.display()),
        "application/docx",
    ))
}

/// DOCX XML parse state.
#[cfg(feature = "file-extract")]
struct DocxParseState {
    text: String,
    tables: usize,
    in_table: bool,
    row_cells: Vec<String>,
    current_cell: String,
    current_para: String,
    in_text: bool,
}

#[cfg(feature = "file-extract")]
impl DocxParseState {
    fn new() -> Self {
        Self { text: String::new(), tables: 0, in_table: false, row_cells: Vec::new(), current_cell: String::new(), current_para: String::new(), in_text: false }
    }

    fn handle_start(&mut self, name: &str) {
        match name {
            "w:tbl" => { self.in_table = true; self.tables += 1; }
            "w:tr" => { self.row_cells.clear(); }
            "w:tc" => { self.current_cell.clear(); }
            "w:p" => { self.current_para.clear(); }
            "w:t" => { self.in_text = true; }
            "w:tab" => { self.current_para.push('\t'); }
            "w:br" => { self.current_para.push('\n'); }
            _ => {}
        }
    }

    fn handle_text(&mut self, content: &str) {
        if self.in_text {
            if self.in_table { self.current_cell.push_str(content); }
            else { self.current_para.push_str(content); }
        }
    }

    fn handle_end(&mut self, name: &str) {
        match name {
            "w:t" => { self.in_text = false; }
            "w:p" => {
                if !self.in_table && !self.current_para.trim().is_empty() {
                    self.text.push_str(self.current_para.trim());
                    self.text.push('\n');
                }
                if self.in_table {
                    self.current_cell.push_str(self.current_para.trim());
                    self.current_cell.push(' ');
                }
            }
            "w:tc" => { self.row_cells.push(self.current_cell.trim().to_string()); }
            "w:tr" => {
                self.text.push_str("| ");
                self.text.push_str(&self.row_cells.join(" | "));
                self.text.push_str(" |\n");
            }
            "w:tbl" => { self.in_table = false; self.text.push('\n'); }
            _ => {}
        }
    }
}

/// Parse DOCX XML — extract paragraphs as text, tables as markdown.
#[cfg(feature = "file-extract")]
fn parse_docx_xml(xml: &str) -> (String, usize) {
    let mut state = DocxParseState::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e))
            | Ok(quick_xml::events::Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                state.handle_start(&name);
            }
            Ok(quick_xml::events::Event::Text(ref e)) => {
                let t = e.unescape().unwrap_or_default();
                state.handle_text(&t);
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                state.handle_end(&name);
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (state.text, state.tables)
}

// ═══════════════════════════════════════════════════════════════════════
// Other document formats
// ═══════════════════════════════════════════════════════════════════════

pub fn extract_legacy_doc(path: &Path) -> Result<ExtractionResult> {
    for cmd in &["antiword", "catdoc"] {
        if let Ok(output) = std::process::Command::new(cmd).arg(path).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                return Ok(make_result(format!("[DOC via {}]\n\n{}", cmd, text), "application/msword"));
            }
        }
    }
    Ok(make_result(
        format!("(Legacy .doc requires antiword or catdoc: {})", path.display()),
        "application/msword",
    ))
}

pub fn extract_rtf(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let mut text = String::new();
    let mut in_group = 0i32;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' { in_group += 1; }
        else if c == '}' { in_group -= 1; }
        else if c == '\\' {
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

pub fn extract_odt(path: &Path) -> Result<ExtractionResult> {
    super::extract_zip_xml(path, "content.xml", "ODT")
}

#[cfg(feature = "file-extract")]
pub fn extract_epub(path: &Path) -> Result<ExtractionResult> {
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
    Ok(make_result_with_meta(
        format!("[EPUB: {} chapters]\n\n{}", chapter, content),
        "application/epub+zip",
        meta,
    ))
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_epub(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(EPUB requires file-extract: {})", path.display()), "application/epub+zip"))
}

#[cfg(feature = "file-extract")]
pub fn extract_pptx(path: &Path) -> Result<ExtractionResult> {
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
                let text = super::extract_text_from_xml(&xml);
                if !text.trim().is_empty() {
                    content.push_str(&format!("--- Slide {} ---\n{}\n\n", slide_num, text.trim()));
                }
            }
            if name.starts_with("ppt/notesSlides/") && name.ends_with(".xml") {
                let mut xml = String::new();
                std::io::Read::read_to_string(&mut entry, &mut xml)?;
                let text = super::extract_text_from_xml(&xml);
                if !text.trim().is_empty() {
                    content.push_str(&format!("[Speaker Notes]\n{}\n\n", text.trim()));
                }
            }
        }
    }

    let mut meta = HashMap::new();
    meta.insert("slides".into(), slide_num.to_string());
    Ok(make_result_with_meta(
        format!("[PPTX: {} slides]\n\n{}", slide_num, content),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        meta,
    ))
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_pptx(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(format!("(PPTX requires file-extract: {})", path.display()), "application/pptx"))
}

pub fn extract_legacy_ppt(path: &Path) -> Result<ExtractionResult> {
    if let Ok(output) = std::process::Command::new("catppt").arg(path).output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(make_result(format!("[PPT via catppt]\n\n{}", text), "application/vnd.ms-powerpoint"));
        }
    }
    Ok(make_result(
        format!("(Legacy .ppt requires catppt: {})", path.display()),
        "application/vnd.ms-powerpoint",
    ))
}

pub fn extract_html(path: &Path) -> Result<ExtractionResult> {
    let html = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[HTML]\n\n{}", strip_html_tags(&html)), "text/html"))
}

pub fn extract_pages(path: &Path) -> Result<ExtractionResult> {
    super::extract_zip_xml(path, "index.xml", "Pages")
}
