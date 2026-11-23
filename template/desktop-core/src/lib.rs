//! Local documents and search for the desktop app.
//!
//! Text (and PDFs) is split into chunks, embedded with a local ONNX model, kept in a
//! store (a JSON file, or embedded Postgres + pgvector) and searched. Every optional
//! part is a cargo feature: `embed`, `postgres`, `pdf`. Without `embed`, search falls
//! back to keywords, so the app works (and tests run) with no model download.

pub mod chunk;
pub mod engine;
pub mod store;

{% if embeddings %}#[cfg(feature = "embed")]
pub mod embed;
{% endif %}{% if pdf %}#[cfg(feature = "pdf")]
pub mod pdf;
{% endif %}{% if postgres %}#[cfg(feature = "postgres")]
pub mod pgstore;
{% endif %}
pub use chunk::{chunks_from_pages, chunks_from_text, Chunk};
pub use engine::{Backend, Engine, EngineOptions, Status};
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
    #[error("{0} is not in this build")]
    NotBuilt(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

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
