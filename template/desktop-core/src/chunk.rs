//! Split text into search-sized chunks along paragraph and sentence boundaries.

use serde::{Deserialize, Serialize};

/// Chunks aim for at most this many characters (a long sentence can go over it).
pub const MAX_CHARS: usize = 800;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// Where the text came from: a file path or a name the user gave.
    pub source: String,
    /// 1-based page for PDFs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    pub text: String,
}

/// Chunk size and overlap (from `IngestConfig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunking {
    pub max_chars: usize,
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

pub fn chunks_from_text_with(source: &str, text: &str, c: Chunking) -> Vec<Chunk> {
    split_with(text, c).into_iter().map(|t| Chunk { source: source.into(), page: None, text: t }).collect()
}

/// Chunks per page, so every chunk keeps its page number.
pub fn chunks_from_pages(source: &str, pages: &[(u32, String)]) -> Vec<Chunk> {
    chunks_from_pages_with(source, pages, Chunking::default())
}

pub fn chunks_from_pages_with(source: &str, pages: &[(u32, String)], c: Chunking) -> Vec<Chunk> {
    pages
        .iter()
        .flat_map(|(n, text)| split_with(text, c).into_iter().map(move |t| Chunk { source: source.into(), page: Some(*n), text: t }))
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
    let mut cur = String::new();
    let paragraphs = text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty());
    for para in paragraphs {
        for piece in sentences_within(&para, max) {
            if !cur.is_empty() && cur.chars().count() + 1 + piece.chars().count() > max {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(&piece);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `para` as pieces of at most `max` characters, cut after `.`, `!` or `?` where possible.
fn sentences_within(para: &str, max: usize) -> Vec<String> {
    if para.chars().count() <= max {
        return vec![para.to_string()];
    }
    let mut pieces = Vec::new();
    let mut cur = String::new();
    for word in para.split(' ') {
        let ends_sentence = word.ends_with(['.', '!', '?']);
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > max {
            pieces.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        if ends_sentence && cur.chars().count() > max / 2 {
            pieces.push(std::mem::take(&mut cur));
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
}
