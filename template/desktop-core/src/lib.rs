//! Local documents and search for the desktop app.
//!
//! Text (and PDFs) is split into chunks, embedded with a local ONNX model, kept in a
//! store (a JSON file, or embedded Postgres + pgvector) and searched. Every optional
//! part is a cargo feature: `embed`, `postgres`, `pdf`. Without `embed`, search falls
//! back to keywords, so the app works (and tests run) with no model download.
//!
//! # How it fits together
//!
//! [`Engine`] is the one object an app holds. It owns a [`Store`] (a JSON file,
//! [`FileStore`], or Postgres with feature `postgres`), an optional [`Embedder`], the live
//! [`Config`] and the ingest journal ([`jobs`]). Text goes in through `Engine::ingest_*`
//! ([`chunk`] splits it, the embedder turns chunks into vectors, the store keeps them) and
//! comes back through [`Engine::search`] ([`store`] ranks, [`fuse`] merges vector and keyword
//! lists in hybrid mode). Everything that leaves the computer goes through [`net`].
//!
//! Main modules: [`engine`] (the facade), [`config`] (every setting, validated),
//! [`chunk`], [`store`], [`fuse`], [`jobs`] (crash-safe ingest), [`backup`], [`logging`],
//! [`net`], [`models`] (models built into the installer, and where each loads from),
//! [`governor`] (CPU share and priority), [`audio`], [`dotenv`] (`.env` files in
//! development builds), [`untrusted`] (size caps and panic guards for outside input). Feature
//! modules: `embed`, `pdf` (with `sandbox`, its child-process limits), `pgstore`, `watch`, `llm` and `meter` (cost
//! meter; feature `ai`), `local` (feature `llama`), `mcp`, `tts`, `stt`, `mail`, `oauth`, `telemetry`, `webview`,
//! `http_api` (a local REST API, feature `http-api`), `appearance` (feature `app-shell`),
//! `notices` (licence notices, feature `supply-chain`), `release` (update channels, feature
//! `release-channels`), `bindings` (typed IPC: TypeScript types for the Tauri commands; feature
//! `ts`, tests only), `eval` (search and RAG evals, feature `evals`), `i18n` (translations: Fluent messages, plurals,
//! numbers, dates and the OS language; feature `i18n`).
//!
//! # Example
//!
//! Index some text and search it, with no model (keyword search):
//!
//! ```
//! use desktop_core::{Engine, FileStore};
//!
//! # let dir = std::env::temp_dir().join(format!("desktop-core-lib-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let store = FileStore::open(&dir)?;
//! let engine = Engine::with_parts(Box::new(store), None, &dir, &mut |_| {})?;
//! engine.ingest_text("notes.md", "Tauri apps ship one small binary.\n\nRust keeps them fast.")?;
//! let hits = engine.search("binary", 3)?;
//! assert_eq!(hits[0].source, "notes.md");
//! # let _ = std::fs::remove_dir_all(&dir);
//! # Ok::<(), desktop_core::Error>(())
//! ```

#![warn(missing_docs)]

pub mod audio;
pub mod backup;
#[cfg(feature = "ts")]
pub mod bindings;
pub mod chunk;
pub mod config;
pub mod dotenv;
pub mod engine;
pub mod fuse;
pub mod governor;
pub mod jobs;
pub mod logging;
pub mod models;
pub mod net;
pub mod store;
pub mod untrusted;

{% if embeddings %}#[cfg(feature = "embed")]
pub mod embed;
{% endif %}{% if pdf %}#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "pdf")]
pub mod sandbox;
{% endif %}{% if postgres %}#[cfg(feature = "postgres")]
pub mod pgstore;
#[cfg(feature = "postgres")]
pub mod pgupgrade;
{% endif %}{% if watch_folders %}{% if mobile %}#[cfg(all(feature = "watch", not(any(target_os = "android", target_os = "ios"))))]{% else %}#[cfg(feature = "watch")]{% endif %}
pub mod watch;
{% endif %}{% if ai %}#[cfg(feature = "ai")]
pub mod cache;
#[cfg(feature = "ai")]
pub mod llm;
#[cfg(feature = "ai")]
pub mod meter;
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
{% endif %}{% if oauth %}#[cfg(feature = "oauth")]
pub mod oauth;
{% endif %}{% if telemetry %}#[cfg(feature = "telemetry")]
pub mod telemetry;
{% endif %}{% if background_jobs %}#[cfg(feature = "jobs-queue")]
pub mod job_types;
#[cfg(feature = "jobs-queue")]
pub mod queue;
{% endif %}{% if plugins %}#[cfg(feature = "plugins")]
pub mod plugin;
{% endif %}{% if rclone %}#[cfg(feature = "rclone")]
pub mod rclone;
{% endif %}{% if tailscale %}#[cfg(feature = "tailscale")]
pub mod tailscale;
{% endif %}{% if app_shell %}#[cfg(feature = "app-shell")]
pub mod appearance;
{% endif %}{% if ipc_worker %}{% if daemon %}#[cfg(feature = "daemon")]
pub mod daemon;
{% endif %}{% endif %}{% if http_api %}#[cfg(feature = "http-api")]
pub mod http_api;
{% endif %}{% if webview %}#[cfg(feature = "webview")]
pub mod webview;
{% endif %}{% if tray_menu %}#[cfg(feature = "tray-menu")]
pub mod actions;
{% endif %}{% if supply_chain %}#[cfg(feature = "supply-chain")]
pub mod notices;
{% endif %}{% if release_channels %}#[cfg(feature = "release-channels")]
pub mod release;
{% endif %}{% if evals %}#[cfg(feature = "evals")]
pub mod eval;
{% endif %}{% if i18n %}#[cfg(feature = "i18n")]
pub mod i18n;
{% endif %}// kitchen:modules (`kitchen generate module` adds modules above this line)

pub use chunk::{chunks_from_pages, chunks_from_text, Chunk, Chunking};
pub use logging::LogConfig;
pub use config::{Config, IngestConfig, KeywordEngine, SearchConfig, SearchMode, WatchConfig};
pub use engine::{Backend, Engine, EngineOptions, IngestReport, Status};
{% if ai %}#[cfg(feature = "ai")]
pub use engine::Answer;
{% endif %}
pub use store::{FileStore, Hit, Store};

/// Everything that can go wrong in this crate. The message is written for the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading or writing a file failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A file or a message was not the JSON expected.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The embedding model could not load or run.
    #[error("embedding: {0}")]
    Embed(String),
    /// The store (file or Postgres) refused or failed.
    #[error("store: {0}")]
    Store(String),
    /// A source (file, folder, audio, URL) could not be used; the text says why.
    #[error("{0}")]
    Source(String),
    /// The language model call failed or AI is switched off.
    #[error("AI: {0}")]
    Llm(String),
    /// The named part is not compiled into this build (its cargo feature is off).
    #[error("{0} is not in this build")]
    NotBuilt(&'static str),
}

/// This crate's `Result`, with [`Error`] as the default error.
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
    /// Vectors for chunks being stored, one per text, in order.
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// The vector for a search query (some models prefix queries differently).
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
///
/// Linear-time backtracking over the last `*` (the usual wildcard algorithm), so a hostile
/// file name or pattern such as `*a*a*a*a*b` cannot make it take exponential time.
pub fn watch_glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ni));
                pi += 1;
            }
            Some('?') => (pi, ni) = (pi + 1, ni + 1),
            Some(c) if c.eq_ignore_ascii_case(&n[ni]) => (pi, ni) = (pi + 1, ni + 1),
            _ => match star {
                // Let the last `*` take one more character and try again from there.
                Some((sp, sn)) => {
                    star = Some((sp, sn + 1));
                    (pi, ni) = (sp + 1, sn + 1);
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod glob_tests {
    #[test]
    fn globs() {
        assert!(super::watch_glob(".*", ".DS_Store"));
        assert!(super::watch_glob("~$*", "~$report.docx"));
        assert!(super::watch_glob("*.TMP", "a.tmp"));
        assert!(!super::watch_glob("*.tmp", "notes.md"));
        assert!(super::watch_glob("a?c*", "abcdef"));
        assert!(super::watch_glob("*", ""));
        assert!(!super::watch_glob("?", ""));
        assert!(super::watch_glob("**x", "abx"));
        assert!(!super::watch_glob("a*b", "acbd"));
    }

    #[test]
    fn hostile_patterns_are_not_exponential() {
        // The recursive matcher took exponential time on these (a hang on one file name).
        let started = std::time::Instant::now();
        let name = "a".repeat(200);
        assert!(!super::watch_glob(&format!("{}b", "*a".repeat(30)), &name));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
