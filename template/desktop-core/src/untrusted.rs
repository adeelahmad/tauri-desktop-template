//! Guards for input the app did not write: size caps for every kind of input, capped
//! reads, and [`guard`], which turns a panic inside a parser into an [`Error`].
//!
//! Every parser of outside bytes (WAV and base64 audio, PDFs, zip bundles, settings and
//! config JSON, watched text files, MCP messages, HTTP bodies, backups, `.env` files)
//! checks its input against a cap from [`limits`] before it allocates, and is fuzzed by a
//! target in the app's `fuzz/` folder. PDFs can also be parsed in a separate, limited
//! process (see the `pdf` module, feature `pdf`).
//!
//! # Example
//!
//! ```
//! use desktop_core::untrusted::{check_size, guard, limits};
//!
//! assert!(check_size("WAV", 1024, limits::WAV_BYTES).is_ok());
//! assert!(check_size("WAV", limits::WAV_BYTES + 1, limits::WAV_BYTES).is_err());
//!
//! // A panic in a parser comes back as an error, not a crash.
//! let r: desktop_core::Result<u8> = guard("demo", || panic!("bad input"));
//! assert!(r.unwrap_err().to_string().contains("demo"));
//! ```

use crate::{Error, Result};
use std::io::{BufRead, Read};
use std::path::Path;

/// The largest input each parser accepts. Larger input is refused before it is read.
pub mod limits {
    /// One mebibyte.
    pub const MB: usize = 1024 * 1024;
    /// A WAV file or recording (about 25 minutes of 48 kHz stereo 32-bit float).
    pub const WAV_BYTES: usize = 512 * MB;
    /// Base64 text handed over by the web view (a [`WAV_BYTES`] file, encoded).
    pub const BASE64_CHARS: usize = WAV_BYTES / 3 * 4 + 4;
    /// A PDF file.
    pub const PDF_BYTES: usize = 512 * MB;
    /// The text a PDF parser may return (all pages together).
    pub const PDF_TEXT_BYTES: usize = 256 * MB;
    /// A settings, config-layer or other JSON settings file.
    pub const CONFIG_BYTES: usize = 8 * MB;
    /// A `.env` file.
    pub const DOTENV_BYTES: usize = MB;
    /// One line-delimited MCP (JSON-RPC) message.
    pub const MCP_LINE_BYTES: usize = 16 * MB;
    /// The manifest (first line) of a backup file.
    pub const BACKUP_HEADER_BYTES: usize = 64 * 1024;
    /// Entries in a zip bundle.
    pub const ZIP_ENTRIES: usize = 10_000;
    /// Bytes unpacked from one zip bundle (all entries together).
    pub const ZIP_UNPACKED_BYTES: u64 = 2 * 1024 * MB as u64;
}

/// An error unless `len` is at most `max` (`what` names the input for the message).
pub fn check_size(what: &str, len: usize, max: usize) -> Result<()> {
    if len > max {
        return Err(Error::Source(format!("{what} is too large ({} bytes; the limit is {} MB)", len, max.div_ceil(limits::MB))));
    }
    Ok(())
}

/// Read a whole file, refusing one larger than `max` bytes (checked before and while reading,
/// so a file that grows is caught too).
pub fn read_capped(path: &Path, max: usize) -> Result<Vec<u8>> {
    let failed = |e: std::io::Error| Error::Source(format!("{}: {e}", path.display()));
    let file = std::fs::File::open(path).map_err(failed)?;
    let len = file.metadata().map_err(failed)?.len();
    check_size(&path.display().to_string(), usize::try_from(len).unwrap_or(usize::MAX), max)?;
    let mut out = Vec::with_capacity(len as usize);
    file.take(max as u64 + 1).read_to_end(&mut out).map_err(failed)?;
    check_size(&path.display().to_string(), out.len(), max)?;
    Ok(out)
}

/// One line (without its `\n`) of at most `max` bytes from `input`.
///
/// `Ok(None)` at the end of input. A longer line is read to its end and dropped, and comes
/// back as `Ok(Some(Err(len)))` with the bytes it had, so the caller can answer it and go on.
pub fn read_line_capped(input: &mut impl BufRead, max: usize) -> std::io::Result<Option<std::result::Result<Vec<u8>, usize>>> {
    let mut line = Vec::new();
    let mut total = 0usize;
    let mut too_long = false;
    loop {
        let buf = match input.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            if total == 0 {
                return Ok(None);
            }
            break;
        }
        let (part, done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (&buf[..i], i + 1),
            None => (buf, buf.len()),
        };
        total = total.saturating_add(part.len());
        if total > max {
            too_long = true;
            line = Vec::new();
        } else {
            line.extend_from_slice(part);
        }
        let ended = done > part.len();
        input.consume(done);
        if ended {
            break;
        }
    }
    Ok(Some(if too_long { Err(total) } else { Ok(line) }))
}

/// Run `parse`, turning a panic into [`Error::Source`] that names `what`.
///
/// Parsers should not panic; this is the backstop for one that does (in this crate or a
/// dependency), so one bad file fails alone instead of taking the app down.
pub fn guard<T>(what: &str, parse: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(parse)).unwrap_or_else(|panic| {
        let why = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        Err(Error::Source(format!("{what}: the parser failed on this input ({why})")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_turns_a_panic_into_an_error() {
        let r: Result<()> = guard("x.wav", || {
            let v: Vec<u8> = Vec::new();
            let _ = v[3];
            Ok(())
        });
        let msg = r.unwrap_err().to_string();
        assert!(msg.starts_with("x.wav: the parser failed"), "{msg}");
        assert_eq!(guard("ok", || Ok(5)).unwrap(), 5);
    }

    #[test]
    fn read_capped_refuses_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("big");
        std::fs::write(&f, vec![b'a'; 100]).unwrap();
        assert_eq!(read_capped(&f, 100).unwrap().len(), 100);
        assert!(read_capped(&f, 99).unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn long_lines_are_dropped_and_reading_goes_on() {
        let text = format!("short\n{}\nnext\nlast", "x".repeat(50));
        let mut input = std::io::BufReader::with_capacity(8, text.as_bytes());
        let mut got = Vec::new();
        while let Some(line) = read_line_capped(&mut input, 10).unwrap() {
            got.push(line.map(|l| String::from_utf8(l).unwrap()));
        }
        assert_eq!(got, vec![Ok("short".to_string()), Err(50), Ok("next".into()), Ok("last".into())]);
    }
}
