//! Local documents and search for the desktop app.
//!
//! Text (and PDFs) is split into chunks, embedded with a local ONNX model, kept in a
//! store (a JSON file, or embedded Postgres + pgvector) and searched. Every optional
//! part is a cargo feature: `embed`, `postgres`, `pdf`. Without `embed`, search falls
//! back to keywords, so the app works (and tests run) with no model download.

pub mod audio;
pub mod backup;
pub mod chunk;
pub mod config;
pub mod engine;
pub mod fuse;
pub mod governor;
pub mod jobs;
pub mod logging;
pub mod net;
pub mod store;

{% if embeddings %}#[cfg(feature = "embed")]
pub mod embed;
{% endif %}{% if pdf %}#[cfg(feature = "pdf")]
pub mod pdf;
{% endif %}{% if postgres %}#[cfg(feature = "postgres")]
pub mod pgstore;
{% endif %}{% if watch_folders %}{% if mobile %}#[cfg(all(feature = "watch", not(any(target_os = "android", target_os = "ios"))))]{% else %}#[cfg(feature = "watch")]{% endif %}
pub mod watch;
{% endif %}{% if ai %}#[cfg(feature = "ai")]
pub mod cache;
#[cfg(feature = "ai")]
pub mod llm;
{% endif %}{% if mcp_server %}#[cfg(feature = "mcp")]
pub mod mcp;
{% endif %}{% if local_llm %}#[cfg(feature = "llama")]
pub mod local;
{% endif %}{% if tts %}#[cfg(feature = "tts")]
pub mod tts;
{% endif %}{% if stt %}#[cfg(feature = "stt")]
pub mod stt;
{% endif %}{% if email %}#[cfg(feature = "mail")]
pub mod mail;
{% endif %}{% if telemetry %}#[cfg(feature = "telemetry")]
pub mod telemetry;
{% endif %}{% if plugins %}#[cfg(feature = "plugins")]
pub mod plugin;
{% endif %}{% if rclone %}#[cfg(feature = "rclone")]
pub mod rclone;
{% endif %}{% if tailscale %}#[cfg(feature = "tailscale")]
pub mod tailscale;
{% endif %}// kitchen:modules (`kitchen generate module` adds modules above this line)

pub use chunk::{chunks_from_pages, chunks_from_text, Chunk, Chunking};
pub use logging::LogConfig;
pub use config::{Config, IngestConfig, KeywordEngine, SearchConfig, SearchMode, WatchConfig};
pub use engine::{Backend, Engine, EngineOptions, IngestReport, Status};
{% if ai %}#[cfg(feature = "ai")]
pub use engine::Answer;
{% endif %}
pub use store::{FileStore, Hit, Store};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("embedding: {0}")]
    Embed(String),
    #[error("store: {0}")]
    Store(String),
    #[error("{0}")]
    Source(String),
    #[error("AI: {0}")]
    Llm(String),
    #[error("{0} is not in this build")]
    NotBuilt(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
{% if embeddings or tts or stt %}
/// The process's one ONNX Runtime environment, shared by the embedder, the voice and Whisper
/// (ONNX Runtime allows only one per process).
#[cfg(any(feature = "embed", feature = "tts", feature = "stt"))]
pub fn onnx_env() -> Result<ort::environment::Environment> {
    static ENV: std::sync::Mutex<Option<ort::environment::Environment>> = std::sync::Mutex::new(None);
    let mut env = ENV.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(e) = env.as_ref() {
        return Ok(e.clone());
    }
    let e = ort::init().build().map_err(|e| Error::Source(format!("ONNX Runtime: {e}")))?;
    *env = Some(e.clone());
    Ok(e)
}
{% endif %}

/// Turns text into unit-length vectors. `LocalEmbedder` (feature `embed`) is the real one;
/// tests use a small deterministic stand-in.
pub trait Embedder: Send + Sync {
    /// Model name and dimension, stored with the index so a model change re-embeds.
    fn name(&self) -> String;
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

/// Scale `v` to unit length (a zero vector stays zero).
pub fn normalise(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Dot product, which is the cosine for unit vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// File-name match with `*` (any run) and `?` (one character), case-insensitive.
pub fn watch_glob(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

#[cfg(test)]
mod glob_tests {
    #[test]
    fn globs() {
        assert!(super::watch_glob(".*", ".DS_Store"));
        assert!(super::watch_glob("~$*", "~$report.docx"));
        assert!(super::watch_glob("*.TMP", "a.tmp"));
        assert!(!super::watch_glob("*.tmp", "notes.md"));
    }
}
