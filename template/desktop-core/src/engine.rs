//! The one object the app talks to: a store plus (optionally) an embedder.

use crate::store::{FileStore, Hit, Store};
use crate::{chunks_from_text, Chunk, Embedder, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Embed this many chunks per model call.
const BATCH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    File,
    Postgres,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOptions {
    /// Private folder for the store, the database and the model cache.
    pub data_dir: PathBuf,
    #[serde(default)]
    pub backend: Backend,
    /// Use the local model when it is built in (feature `embed`).
    #[serde(default = "yes")]
    pub embeddings: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub backend: String,
    pub embedder: Option<String>,
    pub chunks: usize,
    pub data_dir: String,
}

pub struct Engine {
    store: Box<dyn Store>,
    embedder: Option<Box<dyn Embedder>>,
    data_dir: PathBuf,
}

impl Engine {
    /// Open the store and load the model. The first call downloads what is missing (the
    /// model, and Postgres with pgvector), so run it off the UI thread.
    pub fn open(opts: &EngineOptions, progress: &mut dyn FnMut(&str)) -> Result<Self> {
        progress("Opening the store");
        let store = open_store(opts)?;
        let embedder = if opts.embeddings { load_embedder(&opts.data_dir.join("models"), progress)? } else { None };
        Self::with_parts(store, embedder, &opts.data_dir, progress)
    }

    /// Build from parts (tests use their own embedder). Chunks stored with another model
    /// are embedded again with this one.
    pub fn with_parts(
        store: Box<dyn Store>,
        embedder: Option<Box<dyn Embedder>>,
        data_dir: &Path,
        progress: &mut dyn FnMut(&str),
    ) -> Result<Self> {
        let engine = Self { store, embedder, data_dir: data_dir.to_path_buf() };
        let want = engine.embedder.as_ref().map(|e| e.name());
        if engine.store.embedder()? != want {
            let chunks = engine.store.all()?;
            if !chunks.is_empty() {
                progress(&format!("Re-indexing {} chunks for the new model", chunks.len()));
            }
            engine.store.clear()?;
            engine.store.set_embedder(want.as_deref())?;
            engine.add(&chunks)?;
        }
        Ok(engine)
    }

    pub fn status(&self) -> Result<Status> {
        Ok(Status {
            backend: self.store.kind().into(),
            embedder: self.embedder.as_ref().map(|e| e.name()),
            chunks: self.store.count()?,
            data_dir: self.data_dir.to_string_lossy().into_owned(),
        })
    }

    fn add(&self, chunks: &[Chunk]) -> Result<usize> {
        let mut added = 0;
        for batch in chunks.chunks(BATCH) {
            let vectors = match &self.embedder {
                Some(e) => Some(e.embed_documents(&batch.iter().map(|c| c.text.clone()).collect::<Vec<_>>())?),
                None => None,
            };
            added += self.store.add(batch, vectors.as_deref())?;
        }
        Ok(added)
    }

    /// Replace everything stored under `source` with these chunks.
    pub fn ingest_chunks(&self, source: &str, chunks: &[Chunk]) -> Result<usize> {
        self.store.remove_source(source)?;
        self.add(chunks)
    }

    pub fn ingest_text(&self, source: &str, text: &str) -> Result<usize> {
        self.ingest_chunks(source, &chunks_from_text(source, text))
    }

    /// A PDF (feature `pdf`) or any UTF-8 text file.
    pub fn ingest_file(&self, path: &Path) -> Result<usize> {
        let source = path.to_string_lossy().to_string();
        let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        if is_pdf {
            #[cfg(feature = "pdf")]
            return self.ingest_chunks(&source, &crate::pdf::chunks_from_pdf(path)?);
            #[cfg(not(feature = "pdf"))]
            return Err(Error::NotBuilt("PDF reading"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
        self.ingest_text(&source, &text)
    }

    pub fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>> {
        let qv = match &self.embedder {
            Some(e) => Some(e.embed_query(query)?),
            None => None,
        };
        self.store.search(query, qv.as_deref(), k)
    }

    pub fn remove_source(&self, source: &str) -> Result<usize> {
        self.store.remove_source(source)
    }

    pub fn clear(&self) -> Result<()> {
        self.store.clear()
    }
}

fn open_store(opts: &EngineOptions) -> Result<Box<dyn Store>> {
    match opts.backend {
        Backend::File => Ok(Box::new(FileStore::open(&opts.data_dir.join("store"))?)),
        #[cfg(feature = "postgres")]
        Backend::Postgres => Ok(Box::new(crate::pgstore::PostgresStore::open(&opts.data_dir.join("postgres"))?)),
        #[cfg(not(feature = "postgres"))]
        Backend::Postgres => Err(Error::NotBuilt("Postgres")),
    }
}

#[cfg(feature = "embed")]
fn load_embedder(cache: &Path, progress: &mut dyn FnMut(&str)) -> Result<Option<Box<dyn Embedder>>> {
    progress("Loading the search model (the first time downloads it)");
    Ok(Some(Box::new(crate::embed::LocalEmbedder::new(cache)?)))
}

#[cfg(not(feature = "embed"))]
fn load_embedder(_cache: &Path, _progress: &mut dyn FnMut(&str)) -> Result<Option<Box<dyn Embedder>>> {
    Ok(None)
}
