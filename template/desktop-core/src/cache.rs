//! A semantic answer cache for `Engine::ask` (feature `ai`, `llm.cache` in the config).
//!
//! An answer is kept under the normalised question (lower case, single spaces, no closing
//! punctuation) and the state of the index it came from (`Engine` uses the chunk count, the
//! embedding model and the AI settings), so a changed index never serves an old answer.
//! When an embedding of the question is given, a question whose embedding is at least
//! `similarity` (cosine) close to a cached one hits too ("what is X" / "what's X?").
//! Entries expire after `ttl_hours`; past `max_entries` the oldest go. The cache is one JSON
//! line per entry in the data folder (owner-only on Unix), and `clear` empties it, which the
//! engine does on every ingest, removal and clear.
//!
//! ```
//! use desktop_core::cache::{CacheConfig, SemanticCache};
//! let dir = tempfile::tempdir().unwrap();
//! let cache = SemanticCache::new(dir.path().join("answers.jsonl"));
//! let cfg = CacheConfig { enabled: true, ..CacheConfig::default() };
//! cache.put(&cfg, "What is Rust?", None, "42 chunks", &"A language.".to_string());
//! let hit: Option<String> = cache.get(&cfg, "  what is   rust ", None, "42 chunks");
//! assert_eq!(hit.as_deref(), Some("A language."));
//! // Another index state is a miss.
//! assert_eq!(cache.get::<String>(&cfg, "what is rust", None, "43 chunks"), None);
//! ```

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// `llm.cache` in the config. Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CacheConfig {
    /// Use the cache (off by default).
    pub enabled: bool,
    /// Answers older than this are not used (1 hour to 1 year).
    pub ttl_hours: u64,
    /// The most answers kept; the oldest go first.
    pub max_entries: usize,
    /// Cosine similarity (0.5 to 1) at which another question counts as the same one.
    /// Needs the embedding model; without it only the same normalised question hits.
    pub similarity: f32,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { enabled: false, ttl_hours: 24, max_entries: 500, similarity: 0.95 }
    }
}

impl CacheConfig {
    /// Values clamped into safe ranges (called from `LlmConfig::validated`).
    pub fn validated(mut self) -> Self {
        self.ttl_hours = self.ttl_hours.clamp(1, 24 * 365);
        self.max_entries = self.max_entries.clamp(1, 100_000);
        self.similarity = if self.similarity.is_finite() { self.similarity.clamp(0.5, 1.0) } else { 0.95 };
        self
    }
}

/// Lower case, runs of white space as one space, no closing `?`, `!` or `.`.
pub fn normalise_question(q: &str) -> String {
    let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    words.join(" ").trim_end_matches(['?', '!', '.', ' ']).to_string()
}

#[derive(Serialize, Deserialize)]
struct Entry<T> {
    question: String,
    state: String,
    /// Unix seconds when it was stored.
    at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embedding: Option<Vec<f32>>,
    value: T,
}

/// The cache file and a lock so one process never interleaves its own writes.
pub struct SemanticCache {
    file: PathBuf,
    lock: Mutex<()>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl SemanticCache {
    /// A cache kept in `file` (it need not exist yet).
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into(), lock: Mutex::new(()) }
    }

    /// The cache file.
    pub fn path(&self) -> &Path {
        &self.file
    }

    fn read<T: DeserializeOwned>(&self, cfg: &CacheConfig) -> Vec<Entry<T>> {
        let Ok(text) = std::fs::read_to_string(&self.file) else { return Vec::new() };
        let oldest = now().saturating_sub(cfg.ttl_hours.saturating_mul(3600));
        // A line that does not parse (an older format, a torn write) is skipped.
        text.lines().filter_map(|l| serde_json::from_str::<Entry<T>>(l).ok()).filter(|e| e.at >= oldest).collect()
    }

    /// The cached value for `question` on index `state`: the same normalised question, or
    /// (with `embedding`) the closest one at or above `cfg.similarity`.
    pub fn get<T: DeserializeOwned>(&self, cfg: &CacheConfig, question: &str, embedding: Option<&[f32]>, state: &str) -> Option<T> {
        if !cfg.enabled {
            return None;
        }
        let _guard = self.lock.lock().ok()?;
        let q = normalise_question(question);
        let mut best: Option<(f32, Entry<T>)> = None;
        for e in self.read::<T>(cfg).into_iter().rev().filter(|e| e.state == state) {
            let score = if e.question == q {
                f32::INFINITY
            } else {
                match (embedding, e.embedding.as_deref()) {
                    (Some(a), Some(b)) if a.len() == b.len() => crate::cosine(a, b),
                    _ => continue,
                }
            };
            if score >= cfg.similarity && best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, e));
            }
        }
        best.map(|(_, e)| e.value)
    }

    /// Store `value` for `question` (an existing entry for the same question and state is
    /// replaced), then drop expired entries and the oldest beyond `max_entries`.
    /// Failures are logged: the cache only ever saves work.
    pub fn put<T: Serialize + DeserializeOwned>(&self, cfg: &CacheConfig, question: &str, embedding: Option<&[f32]>, state: &str, value: &T) {
        if !cfg.enabled {
            return;
        }
        let Ok(_guard) = self.lock.lock() else { return };
        let q = normalise_question(question);
        let mut entries: Vec<Entry<serde_json::Value>> = self.read(cfg);
        entries.retain(|e| !(e.question == q && e.state == state));
        let Ok(value) = serde_json::to_value(value) else { return };
        entries.push(Entry { question: q, state: state.into(), at: now(), embedding: embedding.map(<[f32]>::to_vec), value });
        let skip = entries.len().saturating_sub(cfg.max_entries);
        if let Err(e) = self.write(&entries[skip..]) {
            log::warn!("answer cache {}: {e}", self.file.display());
        }
    }

    fn write(&self, entries: &[Entry<serde_json::Value>]) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.file.with_extension("jsonl.tmp");
        let mut o = std::fs::OpenOptions::new();
        o.create(true).write(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = std::io::BufWriter::new(o.open(&tmp)?);
        for e in entries {
            serde_json::to_writer(&mut f, e)?;
            f.write_all(b"\n")?;
        }
        f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(tmp, &self.file)
    }

    /// Forget every answer (the index changed).
    pub fn clear(&self) {
        let _guard = self.lock.lock();
        match std::fs::remove_file(&self.file) {
            Ok(()) => log::debug!("answer cache cleared"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("answer cache {}: {e}", self.file.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> CacheConfig {
        CacheConfig { enabled: true, ..CacheConfig::default() }
    }

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut v = v.to_vec();
        crate::normalise(&mut v);
        v
    }

    #[test]
    fn normalises_questions() {
        assert_eq!(normalise_question("  What IS\tRust?? "), "what is rust");
        assert_eq!(normalise_question("Hi!"), "hi");
    }

    #[test]
    fn off_by_default_and_when_disabled() {
        let d = tempfile::tempdir().unwrap();
        let c = SemanticCache::new(d.path().join("a.jsonl"));
        let off = CacheConfig::default();
        assert!(!off.enabled);
        c.put(&off, "q", None, "s", &1);
        assert!(!c.path().exists());
        c.put(&on(), "q", None, "s", &1);
        assert_eq!(c.get::<i32>(&off, "q", None, "s"), None);
        assert_eq!(c.get::<i32>(&on(), "q", None, "s"), Some(1));
    }

    #[test]
    fn near_duplicates_hit_by_cosine() {
        let d = tempfile::tempdir().unwrap();
        let c = SemanticCache::new(d.path().join("a.jsonl"));
        let cfg = CacheConfig { similarity: 0.9, ..on() };
        c.put(&cfg, "what is rust", Some(&unit(&[1.0, 0.0, 0.0])), "s", &"rust".to_string());
        c.put(&cfg, "what is go", Some(&unit(&[0.0, 1.0, 0.0])), "s", &"go".to_string());
        let near = unit(&[1.0, 0.2, 0.0]);
        assert_eq!(c.get::<String>(&cfg, "tell me about rust", Some(&near), "s").as_deref(), Some("rust"));
        let far = unit(&[1.0, 1.0, 0.0]);
        assert_eq!(c.get::<String>(&cfg, "rust or go", Some(&far), "s"), None, "0.71 is under 0.9");
        assert_eq!(c.get::<String>(&cfg, "tell me about rust", None, "s"), None, "no embedding: exact only");
        assert_eq!(c.get::<String>(&cfg, "tell me about rust", Some(&[1.0, 0.0]), "s"), None, "other dimension");
    }

    #[test]
    fn ttl_cap_replace_and_clear() {
        let d = tempfile::tempdir().unwrap();
        let c = SemanticCache::new(d.path().join("a.jsonl"));
        let cfg = CacheConfig { max_entries: 2, ..on() };
        for (i, q) in ["a", "b", "c"].iter().enumerate() {
            c.put(&cfg, q, None, "s", &i);
        }
        assert_eq!(c.get::<usize>(&cfg, "a", None, "s"), None, "the oldest went");
        assert_eq!(c.get::<usize>(&cfg, "c", None, "s"), Some(2));
        c.put(&cfg, "C?", None, "s", &9);
        assert_eq!(c.get::<usize>(&cfg, "c", None, "s"), Some(9), "same question replaced");
        assert_eq!(std::fs::read_to_string(c.path()).unwrap().lines().count(), 2);
        // An entry stored two hours ago is past a one-hour TTL.
        let old = format!("{}\n", serde_json::json!({"question": "old", "state": "s", "at": now() - 7200, "value": 1}));
        std::fs::write(c.path(), old).unwrap();
        assert_eq!(c.get::<usize>(&CacheConfig { ttl_hours: 1, ..on() }, "old", None, "s"), None);
        assert_eq!(c.get::<usize>(&CacheConfig { ttl_hours: 3, ..on() }, "old", None, "s"), Some(1));
        c.clear();
        assert!(!c.path().exists());
        let v = CacheConfig { ttl_hours: 0, max_entries: 0, similarity: f32::NAN, ..on() }.validated();
        assert_eq!((v.ttl_hours, v.max_entries, v.similarity), (1, 1, 0.95));
    }
}
