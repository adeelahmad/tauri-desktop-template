//! PDF text through docling.rs (text layer only, no layout models).
//!
//! Text items are grouped by their page. Pages with no usable text layer (scans, or a
//! broken font encoding) are OCR'd with the system `tesseract`, rendered by poppler's
//! `pdftoppm`; without those programs such a page stays empty.

use crate::{Chunk, Error, Result};
use docling::{DocumentConverter, SourceDocument};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// `(page, text)` for every page, page 1 first.
pub fn pdf_pages(path: &Path) -> Result<Vec<(u32, String)>> {
    let source = SourceDocument::from_file(path).map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
    let result = DocumentConverter::new()
        .text_layer_only(true)
        .convert(source)
        .map_err(|e| Error::Source(format!("could not read {}: {e}", path.display())))?;
    let doc = result.document.export_to_json_value();
    let mut pages: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    if let Some(Value::Object(map)) = doc.get("pages") {
        for key in map.keys() {
            if let Ok(n) = key.parse() {
                pages.entry(n).or_default();
            }
        }
    }
    for item in doc.get("texts").and_then(Value::as_array).into_iter().flatten() {
        let text = item.get("text").and_then(Value::as_str).unwrap_or("").trim();
        let page = item
            .get("prov")
            .and_then(Value::as_array)
            .and_then(|p| p.first())
            .and_then(|p| p.get("page_no"))
            .and_then(Value::as_u64);
        if let (false, Some(page)) = (text.is_empty(), page) {
            pages.entry(page as u32).or_default().push(text.to_string());
        }
    }
    Ok(pages.into_iter().map(|(n, lines)| (n, lines.join("\n"))).collect())
}

/// Text that is mostly not letters, digits or spaces came from a broken font map.
pub fn looks_garbled(text: &str) -> bool {
    let total = text.chars().count();
    if total < 20 {
        return true;
    }
    let good = text.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace() || ".,;:-()'\"".contains(*c)).count();
    (good as f32) / (total as f32) < 0.8
}

/// OCR one page (1-based); empty when poppler or tesseract is missing or fails.
pub fn ocr_page(path: &Path, page: u32) -> String {
    use std::process::Command;
    let Ok(tmp) = tempfile::tempdir() else { return String::new() };
    let prefix = tmp.path().join("page");
    let p = page.to_string();
    let rendered = Command::new("pdftoppm").args(["-r", "216", "-png", "-gray", "-singlefile", "-f", &p, "-l", &p]).arg(path).arg(&prefix).output();
    if !rendered.is_ok_and(|o| o.status.success()) {
        return String::new();
    }
    match Command::new("tesseract").arg(prefix.with_extension("png")).args(["stdout", "--psm", "3", "-l", "eng"]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

pub fn chunks_from_pdf(path: &Path) -> Result<Vec<Chunk>> {
    chunks_from_pdf_with(path, crate::chunk::Chunking::default())
}

pub fn chunks_from_pdf_with(path: &Path, chunking: crate::chunk::Chunking) -> Result<Vec<Chunk>> {
    let mut ocr = std::collections::HashSet::new();
    let pages: Vec<(u32, String)> = pdf_pages(path)?
        .into_iter()
        .map(|(n, text)| {
            if looks_garbled(&text) {
                ocr.insert(n);
                (n, ocr_page(path, n))
            } else {
                (n, text)
            }
        })
        .collect();
    let mut chunks = crate::chunk::chunks_from_pages_with(&path.to_string_lossy(), &pages, chunking);
    for c in &mut chunks {
        let method = if c.page.is_some_and(|p| ocr.contains(&p)) { "pdf-ocr" } else { "pdf-text" };
        c.prov = Some(crate::chunk::Provenance { extractor: method.into(), ..Default::default() });
    }
    Ok(chunks)
}
