//! Split text into search-sized chunks along paragraph and sentence boundaries.
//!
//! Short paragraphs are merged up to the size limit and long ones are cut at sentence ends,
//! so a chunk is a few sentences that belong together. An optional overlap repeats the end of
//! one chunk at the start of the next. Every chunk can carry its [`Provenance`] (file hash,
//! extractor, model, time), which travels into search hits and citations.
//!
//! Main items: [`Chunk`], [`Chunking`], [`chunks_from_text`], [`chunks_from_pages`].
//!
//! # Example
//!
//! ```
//! use desktop_core::chunk::{chunks_from_text, chunks_from_text_with, Chunking};
//!
//! let chunks = chunks_from_text("notes.md", "First paragraph.\n\nSecond paragraph.");
//! assert_eq!(chunks.len(), 1); // short paragraphs are merged
//! assert_eq!(chunks[0].text, "First paragraph. Second paragraph.");
//!
//! let long = "One sentence here. ".repeat(50);
//! let small = chunks_from_text_with("long.txt", &long, Chunking { max_chars: 200, overlap: 40 });
//! assert!(small.len() > 1);
//! assert!(small.iter().all(|c| c.text.chars().count() <= 201));
//! ```

use serde::{Deserialize, Serialize};

/// Chunks aim for at most this many characters (a long sentence can go over it).
pub const MAX_CHARS: usize = 800;

/// A piece of a document: the unit that is embedded, stored and returned by search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Chunk {
    /// Where the text came from: a file path or a name the user gave.
    pub source: String,
    /// 1-based page for PDFs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// The text itself, whitespace collapsed.
    pub text: String,
    /// How this chunk was made; travels with it into every search hit and citation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prov: Option<Provenance>,
}

/// Where a chunk came from and how it was made, kept through search and citations.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Provenance {
    /// SHA-256 of the source file (hex); empty for text given directly.
    pub sha256: String,
    /// Source file size in bytes.
    pub size: u64,
    /// Source file modification time (RFC 3339), when known.
    pub modified: String,
    /// "text", "pdf-text", "pdf-ocr" or "direct" (text handed to `ingest_text`).
    pub extractor: String,
    /// Position among the source's chunks (0-based).
    pub index: u32,
    /// Embedding model, empty for keyword-only indexes.
    pub model: String,
    /// When it was indexed (RFC 3339).
    pub ingested: String,
    /// The desktop-core version that indexed it.
    pub indexer: String,
}

/// Chunk size and overlap (from `IngestConfig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunking {
    /// Chunks aim for at most this many characters.
    pub max_chars: usize,
    /// Characters (whole words) repeated from the end of the previous chunk.
    pub overlap: usize,
}

impl Default for Chunking {
    fn default() -> Self {
        Self { max_chars: MAX_CHARS, overlap: 0 }
    }
}

impl From<&crate::config::IngestConfig> for Chunking {
    fn from(c: &crate::config::IngestConfig) -> Self {
        Self { max_chars: c.chunk_chars, overlap: c.chunk_overlap }
    }
}

/// Paragraphs (blank-line separated) merged up to `MAX_CHARS`; longer ones are cut at sentence ends.
pub fn chunks_from_text(source: &str, text: &str) -> Vec<Chunk> {
    chunks_from_text_with(source, text, Chunking::default())
}

/// [`chunks_from_text`] with a chosen size and overlap.
pub fn chunks_from_text_with(source: &str, text: &str, c: Chunking) -> Vec<Chunk> {
    split_with(text, c).into_iter().map(|t| Chunk { source: source.into(), page: None, text: t, prov: None }).collect()
}

/// Chunks per page, so every chunk keeps its page number.
pub fn chunks_from_pages(source: &str, pages: &[(u32, String)]) -> Vec<Chunk> {
    chunks_from_pages_with(source, pages, Chunking::default())
}

/// [`chunks_from_pages`] with a chosen size and overlap.
pub fn chunks_from_pages_with(source: &str, pages: &[(u32, String)], c: Chunking) -> Vec<Chunk> {
    pages
        .iter()
        .flat_map(|(n, text)| split_with(text, c).into_iter().map(move |t| Chunk { source: source.into(), page: Some(*n), text: t, prov: None }))
        .collect()
}

/// `split`, then each chunk after the first starts with the last `overlap` characters
/// (whole words) of the one before, so a fact cut at a boundary is still found.
fn split_with(text: &str, c: Chunking) -> Vec<String> {
    let chunks = split(text, c.max_chars.saturating_sub(c.overlap).max(1));
    if c.overlap == 0 {
        return chunks;
    }
    let mut out = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        match i.checked_sub(1).map(|p| tail_words(&chunks[p], c.overlap)) {
            Some(tail) if !tail.is_empty() => out.push(format!("{tail} {chunk}")),
            _ => out.push(chunk.clone()),
        }
    }
    out
}

/// The last whole words of `text` that fit in `n` characters.
fn tail_words(text: &str, n: usize) -> String {
    let mut words: Vec<&str> = Vec::new();
    let mut len = 0;
    for w in text.split(' ').rev() {
        let add = w.chars().count() + usize::from(!words.is_empty());
        if len + add > n {
            break;
        }
        len += add;
        words.push(w);
    }
    words.reverse();
    words.join(" ")
}

fn split(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let (mut cur, mut cur_len) = (String::new(), 0usize);
    let paragraphs = text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty());
    for para in paragraphs {
        for piece in sentences_within(&para, max) {
            let len = piece.chars().count();
            if !cur.is_empty() && cur_len + 1 + len > max {
                out.push(std::mem::take(&mut cur));
                cur_len = 0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_len += 1;
            }
            cur.push_str(&piece);
            cur_len += len;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `para` as pieces of at most `max` characters, cut after `.`, `!` or `?` where possible.
/// A word longer than `max` (a hostile file with no spaces) is cut into `max`-character parts.
/// Linear in the length of `para`: lengths are counted once per word.
fn sentences_within(para: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    if para.len() <= max || para.chars().count() <= max {
        return vec![para.to_string()];
    }
    let mut pieces = Vec::new();
    let (mut cur, mut cur_len) = (String::new(), 0usize);
    for word in para.split(' ') {
        let ends_sentence = word.ends_with(['.', '!', '?']);
        let mut word = word;
        let mut len = word.chars().count();
        while len > max {
            if !cur.is_empty() {
                pieces.push(std::mem::take(&mut cur));
                cur_len = 0;
            }
            let cut = word.char_indices().nth(max).map_or(word.len(), |(i, _)| i);
            pieces.push(word[..cut].to_string());
            word = &word[cut..];
            len -= max;
        }
        if !cur.is_empty() && cur_len + 1 + len > max {
            pieces.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if !cur.is_empty() {
            cur.push(' ');
            cur_len += 1;
        }
        cur.push_str(word);
        cur_len += len;
        if ends_sentence && cur_len > max / 2 {
            pieces.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paragraphs_merge_and_long_ones_split() {
        let chunks = chunks_from_text("a", "One.\n\nTwo  words.\n\n\n");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "One. Two words.");
        let long = "This is a sentence. ".repeat(100);
        let chunks = chunks_from_text("b", &long);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.text.chars().count() <= MAX_CHARS));
        assert!(chunks.iter().all(|c| c.text.ends_with('.')));
    }

    #[test]
    fn overlap_repeats_the_end_of_the_previous_chunk() {
        let text = (1..=60).map(|i| format!("word{i}.")).collect::<Vec<_>>().join(" ");
        let c = Chunking { max_chars: 120, overlap: 30 };
        let chunks = chunks_from_text_with("o", &text, c);
        assert!(chunks.len() > 2);
        for pair in chunks.windows(2) {
            let first_word = pair[1].text.split(' ').next().unwrap();
            assert!(pair[0].text.contains(first_word), "{pair:?}");
        }
        assert!(chunks.iter().all(|c| c.text.chars().count() <= 120 + 1));
    }

    #[test]
    fn pages_keep_their_numbers() {
        let chunks = chunks_from_pages("p.pdf", &[(1, "first".into()), (2, String::new()), (3, "third".into())]);
        let pages: Vec<_> = chunks.iter().map(|c| c.page).collect();
        assert_eq!(pages, vec![Some(1), Some(3)]);
    }

    #[test]
    fn a_long_word_is_cut_to_the_chunk_size() {
        // A file with no spaces used to come back as one chunk the size of the file.
        let text = "é".repeat(5000);
        let chunks = chunks_from_text_with("w", &text, Chunking { max_chars: 300, overlap: 40 });
        assert!(
            chunks.iter().all(|c| c.text.chars().count() <= 300 + 40 + 1),
            "{:?}",
            chunks.iter().map(|c| c.text.len()).collect::<Vec<_>>()
        );
        assert!(chunks.iter().map(|c| c.text.chars().filter(|&ch| ch == 'é').count()).sum::<usize>() >= 5000);
    }

    #[test]
    fn many_tiny_words_stay_linear() {
        // Counting the current chunk's characters for every word was quadratic in the chunk size.
        let text = "a ".repeat(2_000_000);
        let started = std::time::Instant::now();
        let chunks = chunks_from_text_with("t", &text, Chunking { max_chars: 20_000, overlap: 0 });
        assert!(chunks.len() >= 199);
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }
}
