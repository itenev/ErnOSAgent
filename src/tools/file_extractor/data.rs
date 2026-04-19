//! Data extractors — spreadsheets, CSV, JSON, YAML, TOML, XML, INI, SQLite.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use super::{ExtractionResult, make_result, make_result_with_meta};

// ═══════════════════════════════════════════════════════════════════════
// Spreadsheets
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
pub fn extract_spreadsheet(path: &Path) -> Result<ExtractionResult> {
    use calamine::{Reader, open_workbook_auto};

    let mut workbook = open_workbook_auto(path)
        .with_context(|| format!("Failed to open spreadsheet: {}", path.display()))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
    let mut content = String::new();

    for name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(name) {
            content.push_str(&format!("--- Sheet: {} ---\n", name));
            content.push_str(&format_sheet_rows(&range));
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

/// Format spreadsheet rows as markdown table (max 200 rows).
#[cfg(feature = "file-extract")]
fn format_sheet_rows(range: &calamine::Range<calamine::Data>) -> String {
    let mut out = String::new();
    let mut row_count = 0;

    for row in range.rows() {
        let cells: Vec<String> = row.iter().map(|c| format!("{}", c)).collect();
        out.push_str("| ");
        out.push_str(&cells.join(" | "));
        out.push_str(" |\n");
        row_count += 1;
        if row_count == 1 {
            let sep: Vec<&str> = cells.iter().map(|_| "---").collect();
            out.push_str("| ");
            out.push_str(&sep.join(" | "));
            out.push_str(" |\n");
        }
        if row_count > 200 {
            out.push_str(&format!("... ({} more rows)\n", range.rows().count() - 200));
            break;
        }
    }
    out
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_spreadsheet(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("(Spreadsheet requires file-extract: {})", path.display()),
        "application/spreadsheet",
    ))
}

// ═══════════════════════════════════════════════════════════════════════
// CSV / TSV
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
pub fn extract_csv(path: &Path, ext: &str) -> Result<ExtractionResult> {
    let delimiter = if ext == "tsv" { b'\t' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)?;

    let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
    let mut content = format_csv_header(&headers);
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
        "text/csv",
        meta,
    ))
}

/// Format CSV header row with separator.
fn format_csv_header(headers: &[String]) -> String {
    let mut out = String::new();
    out.push_str("| ");
    out.push_str(&headers.join(" | "));
    out.push_str(" |\n| ");
    out.push_str(&headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    out.push_str(" |\n");
    out
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_csv(path: &Path, _ext: &str) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(content, "text/csv"))
}

// ═══════════════════════════════════════════════════════════════════════
// Structured data: JSON, JSONL, YAML, TOML, XML, INI
// ═══════════════════════════════════════════════════════════════════════

pub fn extract_json(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let val: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or(serde_json::Value::String(raw.clone()));
    let pretty = serde_json::to_string_pretty(&val).unwrap_or(raw);
    Ok(make_result(format!("[JSON]\n\n{}", pretty), "application/json"))
}

pub fn extract_jsonl(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().collect();
    let total = lines.len();
    let sample: Vec<&str> = lines.iter().take(20).copied().collect();
    Ok(make_result(
        format!("[JSONL: {} lines]\n\nFirst {}:\n{}", total, sample.len(), sample.join("\n")),
        "application/x-ndjson",
    ))
}

pub fn extract_yaml(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[YAML]\n\n{}", content), "application/yaml"))
}

pub fn extract_toml(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[TOML]\n\n{}", content), "application/toml"))
}

#[cfg(feature = "file-extract")]
pub fn extract_xml(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let text = super::extract_text_from_xml(&raw);
    if text.trim().is_empty() {
        Ok(make_result(format!("[XML]\n\n{}", &raw[..raw.len().min(50000)]), "application/xml"))
    } else {
        Ok(make_result(format!("[XML]\n\n{}", text), "application/xml"))
    }
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_xml(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(content, "application/xml"))
}

pub fn extract_ini(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[Config]\n\n{}", content), "text/plain"))
}

// ═══════════════════════════════════════════════════════════════════════
// SQLite
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "file-extract")]
pub fn extract_sqlite(path: &Path) -> Result<ExtractionResult> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;

    let tables = list_sqlite_tables(&conn)?;
    let mut content = format!("[SQLite: {} tables]\n\n", tables.len());

    for table in &tables {
        content.push_str(&dump_sqlite_table(&conn, table)?);
    }

    let mut meta = HashMap::new();
    meta.insert("tables".into(), tables.len().to_string());
    Ok(make_result_with_meta(content, "application/x-sqlite3", meta))
}

/// List all table names in a SQLite database.
#[cfg(feature = "file-extract")]
fn list_sqlite_tables(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(tables)
}

/// Dump schema + first 10 rows of a single SQLite table.
#[cfg(feature = "file-extract")]
fn dump_sqlite_table(conn: &rusqlite::Connection, table: &str) -> Result<String> {
    let mut out = String::new();

    let schema_sql = format!("SELECT sql FROM sqlite_master WHERE name = '{}'", table);
    if let Ok(schema) = conn.query_row(&schema_sql, [], |row| row.get::<_, String>(0)) {
        out.push_str(&format!("--- {} ---\n{}\n\n", table, schema));
    }

    let query = format!("SELECT * FROM \"{}\" LIMIT 10", table);
    if let Ok(mut stmt) = conn.prepare(&query) {
        let col_count = stmt.column_count();
        let col_names: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();

        out.push_str("| ");
        out.push_str(&col_names.join(" | "));
        out.push_str(" |\n| ");
        out.push_str(&col_names.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
        out.push_str(" |\n");

        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(row)) = rows.next() {
                let cells: Vec<String> = (0..col_count)
                    .map(|i| row.get::<_, String>(i).unwrap_or_else(|_| "NULL".to_string()))
                    .collect();
                out.push_str("| ");
                out.push_str(&cells.join(" | "));
                out.push_str(" |\n");
            }
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(not(feature = "file-extract"))]
pub fn extract_sqlite(path: &Path) -> Result<ExtractionResult> {
    Ok(make_result(
        format!("(SQLite requires file-extract: {})", path.display()),
        "application/x-sqlite3",
    ))
}
