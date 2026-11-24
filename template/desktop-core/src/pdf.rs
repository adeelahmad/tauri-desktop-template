//! PDF text through docling.rs (text layer only, no layout models).
//!
//! Text items are grouped by their page. Pages with no usable text layer (scans, or a
//! broken font encoding) are OCR'd with the system `tesseract`, rendered by poppler's
//! `pdftoppm`; without those programs such a page stays empty.
//!
//! PDFs are untrusted input. A file over
//! [`limits::PDF_BYTES`](crate::untrusted::limits::PDF_BYTES) is refused, a panic in the
//! parser becomes an error, and with [`PdfConfig::isolate`] (on by default) the parsing runs
//! in a child process, `<app> --parse`, with a time and a memory limit (see
//! [`crate::sandbox`]). The app's `main` hands `--parse` to [`serve_parse`].
//!
//! # Example
//!
//! ```no_run
//! use desktop_core::pdf::chunks_from_pdf;
//!
//! for chunk in chunks_from_pdf("report.pdf".as_ref())? {
//!     println!("page {:?}: {}", chunk.page, chunk.text);
//! }
//! # Ok::<(), desktop_core::Error>(())
//! ```
//!
//! Parsing bytes in this process (what the child does):
//!
//! ```
//! use desktop_core::pdf::pdf_pages_from_bytes;
//!
//! assert!(pdf_pages_from_bytes(b"%PDF-1.4 not really a PDF".to_vec()).is_err());
//! ```

use crate::untrusted::{guard, limits, read_capped};
use crate::{Chunk, Error, Result};
use docling::{DocumentConverter, InputFormat, SourceDocument};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

/// The command-line flag that makes the app a one-shot parser over stdio.
pub const PARSE_FLAG: &str = "--parse";

/// How PDFs are parsed (`ingest.pdf` in settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct PdfConfig {
    /// Parse each PDF in a separate process (`<app> --parse`) with the limits below, so a
    /// hostile file can only crash or stall that process. On by default.
    pub isolate: bool,
    /// Seconds before the parser process is stopped.
    pub timeout_secs: u64,
    /// Memory the parser process may use, in MiB.
    pub memory_mb: u64,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self { isolate: true, timeout_secs: 120, memory_mb: 2048 }
    }
}

impl PdfConfig {
    /// The same settings with the limits in range (5 s to 1 h, 256 MiB to 64 GiB).
    pub fn validated(mut self) -> Self {
        self.timeout_secs = self.timeout_secs.clamp(5, 3600);
        self.memory_mb = self.memory_mb.clamp(256, 64 * 1024);
        self
    }

    /// The child-process limits these settings stand for.
    pub fn limits(&self) -> crate::sandbox::Limits {
        crate::sandbox::Limits { timeout: Duration::from_secs(self.timeout_secs), memory_mb: self.memory_mb }
    }
}

/// `(page, text)` for every page, page 1 first, parsed in this process.
pub fn pdf_pages(path: &Path) -> Result<Vec<(u32, String)>> {
    let bytes = read_capped(path, limits::PDF_BYTES)?;
    pdf_pages_from_bytes(bytes).map_err(|e| Error::Source(format!("could not read {}: {e}", path.display())))
}

/// `(page, text)` for every page of the PDF in `bytes`, parsed in this process. A panic in
/// the parser comes back as an error.
pub fn pdf_pages_from_bytes(bytes: Vec<u8>) -> Result<Vec<(u32, String)>> {
    crate::untrusted::check_size("the PDF", bytes.len(), limits::PDF_BYTES)?;
    guard("PDF", || {
        let source = SourceDocument::from_bytes("input.pdf", InputFormat::Pdf, bytes);
        let result = DocumentConverter::new().text_layer_only(true).convert(source).map_err(|e| Error::Source(e.to_string()))?;
        Ok(pages_of(&result.document.export_to_json_value()))
    })
}

/// The pages of a docling document (as JSON).
fn pages_of(doc: &Value) -> Vec<(u32, String)> {
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
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok());
        if let (false, Some(page)) = (text.is_empty(), page) {
            pages.entry(page).or_default().push(text.to_string());
        }
    }
    pages.into_iter().map(|(n, lines)| (n, lines.join("\n"))).collect()
}

/// `(page, text)` for every page, parsed by `<app> --parse` in a child process with the
/// limits of `cfg` (the app's own executable is the parser).
pub fn pdf_pages_isolated(path: &Path, cfg: &PdfConfig) -> Result<Vec<(u32, String)>> {
    let exe = std::env::current_exe().map_err(|e| Error::Source(format!("cannot find the app to parse with: {e}")))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(PARSE_FLAG);
    pdf_pages_with(cmd, path, cfg)
}

/// [`pdf_pages_isolated`] with another parser command (one that speaks the
/// [`serve_parse`] protocol), for tests and custom parsers.
pub fn pdf_pages_with(cmd: std::process::Command, path: &Path, cfg: &PdfConfig) -> Result<Vec<(u32, String)>> {
    let bytes = read_capped(path, limits::PDF_BYTES)?;
    let mut input = b"pdf\n".to_vec();
    input.extend_from_slice(&bytes);
    let out = crate::sandbox::run(cmd, &input, &cfg.clone().validated().limits(), limits::PDF_TEXT_BYTES)
        .map_err(|e| Error::Source(format!("could not read {}: {e}", path.display())))?;
    match serde_json::from_slice::<ParseReply>(&out) {
        Ok(ParseReply::Pages(pages)) => Ok(pages),
        Ok(ParseReply::Error(e)) => Err(Error::Source(format!("could not read {}: {e}", path.display()))),
        Err(e) => Err(Error::Source(format!("could not read {}: the parser's reply is not valid ({e})", path.display()))),
    }
}

/// What `<app> --parse` writes to stdout: one JSON value.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ParseReply {
    /// `(page, text)` per page.
    Pages(Vec<(u32, String)>),
    /// Why the input could not be parsed.
    Error(String),
}

/// The child side of `<app> --parse`: read `pdf\n` and then the PDF's bytes from `input`,
/// write `{"pages": [[1, "text"], ...]}` or `{"error": "..."}` to `output`. Returns the
/// process exit code (0 when a reply was written, even an error reply).
pub fn serve_parse(mut input: impl Read, mut output: impl Write) -> i32 {
    let reply = match read_request(&mut input) {
        Ok(bytes) => match pdf_pages_from_bytes(bytes) {
            Ok(pages) => ParseReply::Pages(pages),
            Err(e) => ParseReply::Error(e.to_string()),
        },
        Err(e) => ParseReply::Error(e.to_string()),
    };
    match serde_json::to_writer(&mut output, &reply).ok().and_then(|_| output.flush().ok()) {
        Some(()) => 0,
        None => 1,
    }
}

/// The PDF bytes of a `--parse` request: a `pdf` line, then at most
/// [`limits::PDF_BYTES`] bytes.
pub fn read_request(input: &mut impl Read) -> Result<Vec<u8>> {
    let mut kind = Vec::new();
    let mut byte = [0u8; 1];
    while input.read(&mut byte)? == 1 && byte[0] != b'\n' {
        if kind.len() >= 16 {
            return Err(Error::Source("bad parse request".into()));
        }
        kind.push(byte[0]);
    }
    if kind != b"pdf" {
        return Err(Error::Source(format!("cannot parse {:?} input", String::from_utf8_lossy(&kind))));
    }
    let mut bytes = Vec::new();
    input.take(limits::PDF_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    crate::untrusted::check_size("the PDF", bytes.len(), limits::PDF_BYTES)?;
    Ok(bytes)
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

/// Chunks of a PDF with their page numbers, OCR'ing pages without a text layer (parsed in
/// this process; the engine uses [`chunks_from_pdf_config`]).
pub fn chunks_from_pdf(path: &Path) -> Result<Vec<Chunk>> {
    chunks_from_pdf_with(path, crate::chunk::Chunking::default())
}

/// [`chunks_from_pdf`] with a chosen chunk size and overlap.
pub fn chunks_from_pdf_with(path: &Path, chunking: crate::chunk::Chunking) -> Result<Vec<Chunk>> {
    chunks_from(path, pdf_pages(path)?, chunking)
}

/// [`chunks_from_pdf_with`], parsing in a child process when `cfg.isolate` is on (desktop
/// only: iOS and Android parse in-process).
pub fn chunks_from_pdf_config(path: &Path, chunking: crate::chunk::Chunking, cfg: &PdfConfig) -> Result<Vec<Chunk>> {
    // iOS and Android do not let an app start processes: there the parse stays in-process
    // (still size-capped and panic-guarded).
    let isolate = cfg.isolate && !cfg!(any(target_os = "ios", target_os = "android"));
    let pages = if isolate { pdf_pages_isolated(path, cfg)? } else { pdf_pages(path)? };
    chunks_from(path, pages, chunking)
}

fn chunks_from(path: &Path, pages: Vec<(u32, String)>, chunking: crate::chunk::Chunking) -> Result<Vec<Chunk>> {
    let mut ocr = std::collections::HashSet::new();
    let pages: Vec<(u32, String)> = pages
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for bytes in [&b""[..], b"%PDF-", b"%PDF-1.7\n1 0 obj << /Length 99999999999 >> stream\n", b"\xff\xfe\x00"] {
            assert!(pdf_pages_from_bytes(bytes.to_vec()).is_err());
        }
    }

    #[test]
    fn parse_requests_are_checked() {
        let mut ok = &b"pdf\n%PDF-1.4"[..];
        assert_eq!(read_request(&mut ok).unwrap(), b"%PDF-1.4");
        assert!(read_request(&mut &b"exe\nMZ"[..]).is_err());
        assert!(read_request(&mut &b"pdfpdfpdfpdfpdfpdfpdf"[..]).is_err());
        let mut out = Vec::new();
        assert_eq!(serve_parse(&b"pdf\nnot a pdf"[..], &mut out), 0);
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert!(reply.get("error").is_some(), "{reply}");
    }

    #[test]
    fn settings_are_clamped() {
        let c = PdfConfig { isolate: true, timeout_secs: 0, memory_mb: u64::MAX }.validated();
        assert_eq!((c.timeout_secs, c.memory_mb), (5, 64 * 1024));
        assert!(PdfConfig::default().isolate);
    }

    #[cfg(unix)]
    #[test]
    fn isolated_parse_uses_the_child_reply_and_its_limits() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.pdf");
        std::fs::write(&file, b"%PDF-1.4").unwrap();
        let sh = |script: &str| {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", script]);
            c
        };
        let cfg = PdfConfig::default();
        let pages = pdf_pages_with(sh(r#"cat >/dev/null; echo '{"pages": [[1, "hi"]]}'"#), &file, &cfg).unwrap();
        assert_eq!(pages, vec![(1, "hi".to_string())]);
        let e = pdf_pages_with(sh(r#"cat >/dev/null; echo '{"error": "nope"}'"#), &file, &cfg).unwrap_err();
        assert!(e.to_string().contains("nope"));
        let e = pdf_pages_with(sh("kill -SEGV $$"), &file, &cfg).unwrap_err();
        assert!(e.to_string().contains("crashed"), "{e}");
    }
}
