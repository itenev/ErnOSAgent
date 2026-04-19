//! Communication extractors — email, calendar, vCard.

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use super::{ExtractionResult, make_result, make_result_with_meta};

/// Parse RFC 822 email — extract headers + body.
pub fn extract_email(path: &Path) -> Result<ExtractionResult> {
    let raw = std::fs::read_to_string(path)?;
    let parts: Vec<&str> = raw.splitn(2, "\n\n").collect();
    let headers = parts.first().unwrap_or(&"");
    let body = parts.get(1).unwrap_or(&"");

    let meta = parse_email_headers(headers);

    Ok(make_result_with_meta(
        format!(
            "[Email]\nFrom: {}\nTo: {}\nSubject: {}\nDate: {}\n\n{}",
            meta.get("from").unwrap_or(&"?".into()),
            meta.get("to").unwrap_or(&"?".into()),
            meta.get("subject").unwrap_or(&"?".into()),
            meta.get("date").unwrap_or(&"?".into()),
            body,
        ),
        "message/rfc822",
        meta,
    ))
}

/// Extract key headers from email header block.
fn parse_email_headers(headers: &str) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    for line in headers.lines() {
        if let Some((key, val)) = line.split_once(": ") {
            let k = key.to_lowercase();
            if matches!(k.as_str(), "from" | "to" | "subject" | "date") {
                meta.insert(k, val.to_string());
            }
        }
    }
    meta
}

/// Parse iCalendar .ics files.
pub fn extract_calendar(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[Calendar]\n\n{}", content), "text/calendar"))
}

/// Parse vCard .vcf files.
pub fn extract_vcard(path: &Path) -> Result<ExtractionResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(make_result(format!("[vCard]\n\n{}", content), "text/vcard"))
}
