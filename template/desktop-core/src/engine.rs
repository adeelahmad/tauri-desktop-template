//! The one object the app talks to: a store plus (optionally) an embedder.

use crate::chunk::{chunks_from_text_with, Chunking};
use crate::config::{Config, SearchConfig};
use crate::store::{FileStore, Hit, Store};
use crate::{Chunk, Embedder, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

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
    /// Search, ingest and watch settings.
    #[serde(default)]
    pub config: Config,
    /// Names the default drop folder (`<Documents>/<app_name> Inbox`).
    #[serde(default)]
    pub app_name: Option<String>,
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
    config: RwLock<Config>,
}

/// What `ingest_paths` did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestReport {
    pub files: usize,
    pub chunks: usize,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

impl Engine {
    /// Open the store and load the model. The first call downloads what is missing (the
    /// model, and Postgres with pgvector), so run it off the UI thread.
    pub fn open(opts: &EngineOptions, progress: &mut dyn FnMut(&str)) -> Result<Self> {
        progress("Opening the store");
        let store = open_store(opts)?;
        let embedder = if opts.embeddings { load_embedder(&opts.data_dir.join("models"), progress)? } else { None };
        let engine = Self::with_parts(store, embedder, &opts.data_dir, progress)?;
        engine.configure(opts.config.clone())?;
        Ok(engine)
    }

    /// Build from parts (tests use their own embedder). Chunks stored with another model
    /// are embedded again with this one.
    pub fn with_parts(
        store: Box<dyn Store>,
        embedder: Option<Box<dyn Embedder>>,
        data_dir: &Path,
        progress: &mut dyn FnMut(&str),
    ) -> Result<Self> {
        let engine = Self { store, embedder, data_dir: data_dir.to_path_buf(), config: RwLock::new(Config::default()) };
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

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn status(&self) -> Result<Status> {
        Ok(Status {
            backend: self.store.kind().into(),
            embedder: self.embedder.as_ref().map(|e| e.name()),
            chunks: self.store.count()?,
            data_dir: self.data_dir.to_string_lossy().into_owned(),
        })
    }

    /// Apply new settings (validated first). Takes effect for the next ingest or search.
    pub fn configure(&self, config: Config) -> Result<()> {
        let config = config.validated();
        self.store.configure(&config)?;
        *self.config.write().map_err(|_| Error::Store("lock poisoned".into()))? = config;
        Ok(())
    }

    pub fn config(&self) -> Config {
        self.config.read().map(|c| c.clone()).unwrap_or_default()
    }

    fn add(&self, chunks: &[Chunk]) -> Result<usize> {
        let batch = self.config().ingest.embed_batch.max(1);
        let mut added = 0;
        for batch in chunks.chunks(batch) {
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
        let chunking = Chunking::from(&self.config().ingest);
        self.ingest_chunks(source, &chunks_from_text_with(source, text, chunking))
    }

    /// Why `path` would be skipped by folder ingest and watching, if it would.
    pub fn skip_reason(&self, path: &Path) -> Option<String> {
        let cfg = self.config();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if cfg.watch.ignore.iter().any(|p| crate::watch_glob(p, &name)) {
            return Some(format!("{name}: ignored name"));
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !cfg.ingest.extensions.contains(&ext) {
            return Some(format!("{name}: .{ext} is not an ingested type"));
        }
        match std::fs::metadata(path) {
            Ok(m) if !m.is_file() => Some(format!("{name}: not a file")),
            Ok(m) if m.len() > cfg.ingest.max_file_mb * 1024 * 1024 => Some(format!("{name}: larger than {} MB", cfg.ingest.max_file_mb)),
            Ok(_) => None,
            Err(e) => Some(format!("{name}: {e}")),
        }
    }

    /// A PDF (feature `pdf`) or any UTF-8 text file.
    pub fn ingest_file(&self, path: &Path) -> Result<usize> {
        let (source, chunks) = self.read_file(path)?;
        self.ingest_chunks(&source, &chunks)
    }

    /// Read and chunk a file without touching the store (the parallel part of ingest).
    fn read_file(&self, path: &Path) -> Result<(String, Vec<Chunk>)> {
        let source = path.to_string_lossy().to_string();
        let chunking = Chunking::from(&self.config().ingest);
        let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        if is_pdf {
            #[cfg(feature = "pdf")]
            return Ok((source, crate::pdf::chunks_from_pdf_with(path, chunking)?));
            #[cfg(not(feature = "pdf"))]
            return Err(Error::NotBuilt("PDF reading"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
        let chunks = chunks_from_text_with(&source, &text, chunking);
        Ok((source, chunks))
    }

    /// Every ingestable file in `dir` (and below it, when `recursive`).
    pub fn files_in(&self, dir: &Path, recursive: bool) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let p = entry.path();
                // Symlinked folders are not followed, so a link cannot pull in the whole disk.
                match entry.file_type() {
                    Ok(t) if t.is_dir() && recursive && !crate::watch_glob(".*", &entry.file_name().to_string_lossy()) => stack.push(p),
                    Ok(t) if t.is_file() && self.skip_reason(&p).is_none() => out.push(p),
                    _ => {}
                }
            }
        }
        out.sort();
        out
    }

    /// Ingest files with `ingest.workers` threads: reading, chunking and embedding run in
    /// parallel, store writes one file at a time. `progress` gets `(done, total, path)`.
    pub fn ingest_paths(&self, paths: &[PathBuf], progress: &(dyn Fn(usize, usize, &Path) + Sync)) -> IngestReport {
        let workers = self.config_workers().min(paths.len()).max(1);
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let report = Mutex::new(IngestReport::default());
        let write = Mutex::new(());
        let run = || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            let Some(path) = paths.get(i) else { break };
            let result = (|| -> Result<Option<usize>> {
                if let Some(reason) = self.skip_reason(path) {
                    report.lock().map_err(|_| Error::Store("lock poisoned".into()))?.skipped.push(reason);
                    return Ok(None);
                }
                let (source, chunks) = self.read_file(path)?;
                let vectors = self.embed_all(&chunks)?;
                let _one_writer = write.lock().map_err(|_| Error::Store("lock poisoned".into()))?;
                self.store.remove_source(&source)?;
                let mut added = 0;
                let batch = self.config().ingest.embed_batch.max(1);
                for (n, part) in chunks.chunks(batch).enumerate() {
                    let v = vectors.as_ref().map(|v| &v[n * batch..n * batch + part.len()]);
                    added += self.store.add(part, v)?;
                }
                Ok(Some(added))
            })();
            if let Ok(mut r) = report.lock() {
                match result {
                    Ok(Some(n)) => {
                        r.files += 1;
                        r.chunks += n;
                    }
                    Ok(None) => {}
                    Err(e) => r.errors.push(format!("{}: {e}", path.display())),
                }
            }
            progress(done.fetch_add(1, Ordering::SeqCst) + 1, paths.len(), path);
        };
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(run);
            }
        });
        report.into_inner().unwrap_or_default()
    }

    fn config_workers(&self) -> usize {
        self.config().workers()
    }

    fn embed_all(&self, chunks: &[Chunk]) -> Result<Option<Vec<Vec<f32>>>> {
        let Some(e) = &self.embedder else { return Ok(None) };
        let batch = self.config().ingest.embed_batch.max(1);
        let mut out = Vec::with_capacity(chunks.len());
        for part in chunks.chunks(batch) {
            out.extend(e.embed_documents(&part.iter().map(|c| c.text.clone()).collect::<Vec<_>>())?);
        }
        Ok(Some(out))
    }

    /// Search with the configured settings, returning `k` results.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>> {
        let cfg = SearchConfig { top_k: k, ..self.config().search };
        self.search_with(query, &cfg)
    }

    /// Search with explicit settings (for a UI that tweaks them per query).
    pub fn search_with(&self, query: &str, cfg: &SearchConfig) -> Result<Vec<Hit>> {
        let cfg = Config { search: cfg.clone(), ..Config::default() }.validated().search;
        let qv = match &self.embedder {
            Some(e) if cfg.mode != crate::config::SearchMode::Keyword => Some(e.embed_query(query)?),
            _ => None,
        };
        self.store.search(query, qv.as_deref(), &cfg)
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
