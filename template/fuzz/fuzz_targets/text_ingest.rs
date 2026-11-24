//! Watch-folder and drag-and-drop text ingest: the ignore globs against the file name, then
//! chunking the file's text with any chunk size and overlap the settings allow.
#![no_main]

use desktop_core::{chunk::chunks_from_text_with, chunks_from_text, watch_glob, Chunking};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let max_chars = 100 + usize::from(u16::from_le_bytes([data[0], data[1]])) % 20_000;
    let overlap = usize::from(data[2]) * max_chars / 512;
    let Ok(text) = std::str::from_utf8(&data[3..]) else { return };
    let chunks = chunks_from_text_with("f.txt", text, Chunking { max_chars, overlap });
    for c in &chunks {
        // Cut at the chunk size (plus the repeated overlap and its space), never longer.
        assert!(c.text.chars().count() <= max_chars + overlap + 1, "{} > {max_chars}+{overlap}", c.text.chars().count());
    }
    let _ = chunks_from_text("f.txt", text);
    // Name and pattern from the same input: must finish quickly whatever they are.
    let (pattern, name) = text.split_once('\n').unwrap_or((text, "name.txt"));
    let _ = watch_glob(pattern, name);
});
