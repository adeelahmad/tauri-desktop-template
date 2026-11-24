//! The one object the app talks to: a store plus (optionally) an embedder.

use crate::chunk::{chunks_from_text_with, Chunking};
use crate::config::{Config, SearchConfig};
use crate::store::{FileStore, Hit, Store};
use crate::{Chunk, Embedder, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

{% if ai %}/// What `Engine::ask` returns: the model's reply and the passages it was given.
#[cfg(feature = "ai")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub reply: crate::llm::Reply,
    pub sources: Vec<Hit>,
    /// Served from the answer cache (`llm.cache`) instead of asking the model.
    #[serde(default)]
    pub cached: bool,
}

{% endif %}#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    journal: crate::jobs::Journal,
    /// Set by `cancel_ingest`: workers stop after their current file.
    cancel: AtomicBool,
{% if plugins %}    /// The plugins built in (they run only when `plugins.enabled`).
    #[cfg(feature = "plugins")]
    plugins: crate::plugin::Registry,
{% endif %}{% if ai %}    /// Answers to `ask` (`llm.cache`), emptied whenever the index changes.
    #[cfg(feature = "ai")]
    answers: crate::cache::SemanticCache,
{% endif %}}

/// What `ingest_paths` did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestReport {
    pub files: usize,
    pub chunks: usize,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
    /// Stopped by `cancel_ingest` (the rest stays in the journal for the next start).
    #[serde(default)]
    pub cancelled: bool,
}

impl Engine {
    /// Open the store and load the model. The first call downloads what is missing (the
    /// model, and Postgres with pgvector), so run it off the UI thread.
    pub fn open(opts: &EngineOptions, progress: &mut dyn FnMut(&str)) -> Result<Self> {
        crate::net::configure(opts.config.net.clone());
        progress("Opening the store");
        let store = open_store(opts)?;
        let embedder = if opts.embeddings { load_embedder(&opts.data_dir.join("models"), opts.config.resources.onnx_provider, progress)? } else { None };
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
        let engine = Self {
            store,
            embedder,
            data_dir: data_dir.to_path_buf(),
            config: RwLock::new(Config::default()),
            journal: crate::jobs::Journal::new(data_dir),
            cancel: AtomicBool::new(false),
{% if plugins %}            #[cfg(feature = "plugins")]
            plugins: crate::plugin::builtin(),
{% endif %}{% if ai %}            #[cfg(feature = "ai")]
            answers: crate::cache::SemanticCache::new(data_dir.join("answer-cache.jsonl")),
{% endif %}        };
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

{% if plugins %}    /// Use these plugins instead of the built-in ones (tests, or an app that builds its list).
    #[cfg(feature = "plugins")]
    pub fn with_plugins(mut self, plugins: crate::plugin::Registry) -> Self {
        self.plugins = plugins;
        self
    }

    /// The plugins in this build.
    #[cfg(feature = "plugins")]
    pub fn plugins(&self) -> &crate::plugin::Registry {
        &self.plugins
    }

    /// Answer `plugin.<name>.<method>` with the current plugin settings.
    #[cfg(feature = "plugins")]
    pub fn plugin_call(&self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        self.plugins.call(&self.config().plugins, method, params).map_err(Error::Source)
    }

{% endif %}    /// Run the plugins' ingest hooks (feature `plugins`; nothing without it).
    fn plugin_hooks(&self, source: &str, chunks: &mut Vec<Chunk>) -> Result<()> {
{% if plugins %}        #[cfg(feature = "plugins")]
        self.plugins.on_ingest(&self.config().plugins, source, chunks).map_err(Error::Source)?;
{% endif %}        let _ = (source, chunks);
        Ok(())
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
        crate::net::configure(config.net.clone());
        *self.config.write().map_err(|_| Error::Store("lock poisoned".into()))? = config;
        Ok(())
    }

    pub fn config(&self) -> Config {
        self.config.read().map(|c| c.clone()).unwrap_or_default()
    }

    /// The index changed: cached answers no longer hold.
    fn changed(&self) {
{% if ai %}        #[cfg(feature = "ai")]
        self.answers.clear();
{% endif %}    }

    /// The embedding batch for one writer, under the memory budget.
    fn embed_batch(&self) -> usize {
        let cfg = self.config();
        let free = if cfg.resources.memory_budget_mb > 0 { crate::governor::free_memory_mb() } else { None };
        cfg.resources.plan(1, cfg.ingest.embed_batch, free).embed_batch
    }

    fn add(&self, chunks: &[Chunk]) -> Result<usize> {
        self.changed();
        let batch = self.embed_batch();
        // Re-embedding keeps where a chunk came from but records the new model.
        let model = self.model_name();
        let chunks: Vec<Chunk> = chunks
            .iter()
            .cloned()
            .map(|mut c| {
                if let Some(p) = c.prov.as_mut() {
                    p.model.clone_from(&model);
                }
                c
            })
            .collect();
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
        let mut chunks = chunks_from_text_with(source, text, chunking);
        self.stamp(&mut chunks, None, "direct");
        self.plugin_hooks(source, &mut chunks)?;
        self.ingest_chunks(source, &chunks)
    }

    fn model_name(&self) -> String {
        self.embedder.as_ref().map(|e| e.name()).unwrap_or_default()
    }

    /// Fill in each chunk's provenance: the file's hash, size and time (when there is a
    /// file), how the text was extracted (kept when the reader set it), its position, the
    /// model and when it was indexed.
    fn stamp(&self, chunks: &mut [Chunk], file: Option<(&[u8], &std::fs::Metadata)>, extractor: &str) {
        use sha2::Digest;
        let (sha256, size, modified) = match file {
            Some((bytes, meta)) => (format!("{:x}", sha2::Sha256::digest(bytes)), meta.len(), meta.modified().map(crate::logging::rfc3339).unwrap_or_default()),
            None => Default::default(),
        };
        let (model, ingested) = (self.model_name(), crate::logging::rfc3339(std::time::SystemTime::now()));
        for (i, c) in chunks.iter_mut().enumerate() {
            let method = c.prov.take().map(|p| p.extractor).filter(|e| !e.is_empty()).unwrap_or_else(|| extractor.into());
            c.prov = Some(crate::chunk::Provenance {
                sha256: sha256.clone(),
                size,
                modified: modified.clone(),
                extractor: method,
                index: i as u32,
                model: model.clone(),
                ingested: ingested.clone(),
                indexer: env!("CARGO_PKG_VERSION").into(),
            });
        }
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
        let failed = |e: std::io::Error| Error::Source(format!("{}: {e}", path.display()));
        let meta = std::fs::metadata(path).map_err(failed)?;
        let bytes = std::fs::read(path).map_err(failed)?;
        if is_pdf {
            #[cfg(feature = "pdf")]
            {
                let mut chunks = crate::pdf::chunks_from_pdf_with(path, chunking)?;
                self.stamp(&mut chunks, Some((&bytes, &meta)), "pdf-text");
                self.plugin_hooks(&source, &mut chunks)?;
                return Ok((source, chunks));
            }
            #[cfg(not(feature = "pdf"))]
            return Err(Error::NotBuilt("PDF reading"));
        }
        let text = String::from_utf8(bytes).map_err(|_| Error::Source(format!("{}: not UTF-8 text", path.display())))?;
        let mut chunks = chunks_from_text_with(&source, &text, chunking);
        self.stamp(&mut chunks, Some((text.as_bytes(), &meta)), "text");
        self.plugin_hooks(&source, &mut chunks)?;
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

    /// Ingest files with `ingest.workers` threads (capped by `resources.cpuPercent`, at
    /// background priority when `resources.backgroundPriority`): reading, chunking and
    /// embedding run in parallel, store writes one file at a time. The list is journaled
    /// first, so an interrupted run can be resumed (`resume_interrupted`). `progress` gets
    /// `(done, total, path)`.
    pub fn ingest_paths(&self, paths: &[PathBuf], progress: &(dyn Fn(usize, usize, &Path) + Sync)) -> IngestReport {
        let job = match self.journal.begin(paths) {
            Ok(j) => Some(j),
            Err(e) => {
                log::warn!("ingest journal unavailable, continuing without it: {e}");
                None
            }
        };
        let all: Vec<usize> = (0..paths.len()).collect();
        self.run_ingest(paths, &all, job, progress)
    }

    /// Ingests that a crash or quit cut short.
    pub fn interrupted(&self) -> Vec<crate::jobs::Pending> {
        self.journal.pending()
    }

    /// Continue every interrupted ingest (oldest first).
    pub fn resume_interrupted(&self, progress: &(dyn Fn(usize, usize, &Path) + Sync)) -> IngestReport {
        let mut total = IngestReport::default();
        for p in self.journal.pending() {
            let Some(paths) = self.journal.paths(&p.id) else { continue };
            let left: Vec<usize> = paths.iter().enumerate().filter(|(_, x)| p.remaining.contains(x)).map(|(i, _)| i).collect();
            log::info!("resuming an interrupted ingest: {} of {} files left", left.len(), paths.len());
            let job = self.journal.reopen(&p.id).ok();
            let r = self.run_ingest(&paths, &left, job, progress);
            total.files += r.files;
            total.chunks += r.chunks;
            total.skipped.extend(r.skipped);
            total.errors.extend(r.errors);
            if r.cancelled {
                total.cancelled = true;
                break;
            }
        }
        total
    }

    /// Stop the running ingest after the files in progress (what is left is kept for later).
    pub fn cancel_ingest(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn run_ingest(&self, paths: &[PathBuf], todo: &[usize], job: Option<crate::jobs::Job>, progress: &(dyn Fn(usize, usize, &Path) + Sync)) -> IngestReport {
        self.cancel.store(false, Ordering::SeqCst);
        let resources = self.config().resources;
        let workers = resources.workers(self.config_workers()).min(todo.len()).max(1);
        // Under a memory budget: fewer workers and smaller batches when memory is short,
        // and files too large for a worker's share are skipped (instead of running out).
        let free = if resources.memory_budget_mb > 0 { crate::governor::free_memory_mb() } else { None };
        let plan = resources.plan(workers, self.config().ingest.embed_batch, free);
        let (workers, batch) = (plan.workers, plan.embed_batch);
        self.changed();
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let report = Mutex::new(IngestReport::default());
        let write = Mutex::new(());
        let run = || loop {
            if self.cancel.load(Ordering::SeqCst) {
                if let Ok(mut r) = report.lock() {
                    r.cancelled = true;
                }
                break;
            }
            let n = next.fetch_add(1, Ordering::SeqCst);
            let Some(&i) = todo.get(n) else { break };
            let Some(path) = paths.get(i) else { break };
            let result = (|| -> Result<Option<usize>> {
                let too_large = || plan.too_large(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)).map(|r| format!("{}: {r}", path.display()));
                if let Some(reason) = self.skip_reason(path).or_else(too_large) {
                    report.lock().map_err(|_| Error::Store("lock poisoned".into()))?.skipped.push(reason);
                    return Ok(None);
                }
                let (source, chunks) = self.read_file(path)?;
                let vectors = self.embed_all(&chunks, batch)?;
                let _one_writer = write.lock().map_err(|_| Error::Store("lock poisoned".into()))?;
                self.store.remove_source(&source)?;
                let mut added = 0;
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
            if let Some(j) = &job {
                j.done(i);
            }
            progress(done.fetch_add(1, Ordering::SeqCst) + 1, todo.len(), path);
        };
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    if resources.background_priority {
                        crate::governor::background_thread();
                    }
                    run()
                });
            }
        });
        let report = report.into_inner().unwrap_or_default();
        // Answers cached while the run was going are stale too.
        self.changed();
        if let Some(j) = job {
            if !report.cancelled {
                j.finish();
            }
        }
        report
    }

    fn config_workers(&self) -> usize {
        self.config().workers()
    }

    fn embed_all(&self, chunks: &[Chunk], batch: usize) -> Result<Option<Vec<Vec<f32>>>> {
        let Some(e) = &self.embedder else { return Ok(None) };
        let batch = batch.max(1);
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
        let hits = self.store.search(query, qv.as_deref(), &cfg)?;
{% if plugins %}        #[cfg(feature = "plugins")]
        let hits = self.plugins.search(&self.config().plugins, query, cfg.top_k, hits);
{% endif %}        Ok(hits)
    }

{% if ai %}    /// Answer `question` from the indexed documents with the configured LLM (RAG).
    /// `on_token` gets the answer as it streams; the sources come back with it.
    ///
    /// With `llm.cache` on, an answer to the same question (or, with embeddings, one at
    /// least `llm.cache.similarity` alike) on the same index comes from the cache and
    /// streams as one piece. `resources` caps the local model's GPU layers and refuses a
    /// model larger than the memory budget.
    #[cfg(feature = "ai")]
    pub fn ask(&self, question: &str, keys: crate::llm::KeyLookup<'_>, on_token: &mut dyn FnMut(&str)) -> Result<Answer> {
        let mut cfg = self.config();
        let cache = cfg.llm.cache.clone();
        let (state, qv) = if cache.enabled {
            let qv = match &self.embedder {
                Some(e) => Some(e.embed_query(question)?),
                None => None,
            };
            (self.index_state(&cfg)?, qv)
        } else {
            Default::default()
        };
        if let Some(mut hit) = self.answers.get::<Answer>(&cache, question, qv.as_deref(), &state) {
            hit.cached = true;
            on_token(&hit.reply.text);
            return Ok(hit);
        }
        cfg.llm.local.gpu_layers = cfg.resources.gpu_layers(cfg.llm.local.gpu_layers);
        if cfg.llm.target.provider == crate::llm::Provider::Local && cfg.resources.memory_budget_mb > 0 {
            let size = cfg.llm.local.model_path.as_deref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
            if let Some(why) = cfg.resources.model_too_large(size, crate::governor::free_memory_mb()) {
                return Err(Error::Llm(why));
            }
        }
        let sources = self.search_with(question, &cfg.search)?;
        let messages = crate::llm::rag_messages(cfg.llm.system_prompt.as_deref(), question, &sources);
        let usage = self.data_dir.join("llm-usage.json");
        let reply = crate::llm::Client::new(cfg.llm, keys, Some(&usage)).chat(&messages, on_token)?;
        let answer = Answer { reply, sources, cached: false };
        self.answers.put(&cache, question, qv.as_deref(), &state, &answer);
        Ok(answer)
    }

    /// What a cached answer depends on: the chunk count, the embedding model, and the
    /// settings that shape an answer (search and AI, without the cache's own).
    #[cfg(feature = "ai")]
    fn index_state(&self, cfg: &Config) -> Result<String> {
        use sha2::Digest;
        let llm = crate::llm::LlmConfig { cache: Default::default(), ..cfg.llm.clone() };
        let shape = serde_json::to_vec(&(&cfg.search, &llm))?;
        Ok(format!("{}:{}:{:x}", self.store.count()?, self.model_name(), sha2::Sha256::digest(shape)))
    }

{% endif %}{% if local_llm %}    /// Download a GGUF model (`url`, or the configured `llm.local.modelUrl`) into
    /// `<data>/models`, resuming a partial download, and select it. Returns the file.
    #[cfg(feature = "llama")]
    pub fn download_model(&self, url: Option<&str>, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
        let mut cfg = self.config();
        let url = url.map(str::to_owned).or_else(|| cfg.llm.local.model_url.clone()).ok_or_else(|| Error::Llm("no model URL".into()))?;
        let dest = crate::local::model_file(&self.data_dir.join("models"), &url);
        if !dest.is_file() {
            crate::net::download_to(&url, &dest, progress).map_err(Error::Llm)?;
        }
        cfg.llm.local.model_path = Some(dest.clone());
        self.configure(cfg)?;
        Ok(dest)
    }

{% endif %}    pub fn remove_source(&self, source: &str) -> Result<usize> {
        self.changed();
        self.store.remove_source(source)
    }

    pub fn clear(&self) -> Result<()> {
        self.changed();
        self.store.clear()
    }

    /// Where backups go: the configured folder, or `<data>/backups`.
    pub fn backup_dir(&self) -> PathBuf {
        self.config().backup.folder.unwrap_or_else(|| self.data_dir.join("backups"))
    }

    /// Write a backup of every chunk now, then delete the oldest beyond `backup.keep`.
    pub fn backup(&self) -> Result<crate::backup::BackupInfo> {
        let dir = self.backup_dir();
        let key = if self.config().backup.encrypt { Some(self.backup_key()?) } else { None };
        let info = crate::backup::write(&dir, &self.store.all()?, &self.model_name(), key.as_ref())
            .map_err(|e| Error::Store(format!("backup to {}: {e}", dir.display())))?;
        let pruned = crate::backup::prune(&dir, self.config().backup.keep);
        log::info!("backup written: {} chunks, {} bytes ({pruned} old backups removed)", info.chunks, info.bytes);
        Ok(info)
    }

    /// This computer's backup key (made the first time it is needed).
    pub fn backup_key(&self) -> Result<crate::backup::BackupKey> {
        crate::backup::BackupKey::load_or_create(&self.data_dir).map_err(|e| Error::Store(format!("backup key: {e}")))
    }

    /// Backups in the backup folder, newest first.
    pub fn backups(&self) -> Vec<crate::backup::BackupInfo> {
        crate::backup::list(&self.backup_dir())
    }

    /// Back up if the schedule says one is due (`None` when not).
    pub fn backup_if_due(&self) -> Option<Result<crate::backup::BackupInfo>> {
        crate::backup::due(&self.backup_dir(), self.config().backup.interval_hours).then(|| self.backup())
    }

    /// Replace the index with a backup. The file is verified first, and the current index
    /// is backed up before it is replaced (unless it is empty). Chunks are embedded again
    /// with the current model. An encrypted backup opens with this computer's key or the
    /// `recovery_key` given; a recovery key that works becomes this computer's key when it
    /// has none yet. Returns the chunks restored.
    pub fn restore(&self, path: &Path, recovery_key: Option<&str>) -> Result<usize> {
        let given = match recovery_key.filter(|k| !k.trim().is_empty()) {
            Some(text) => Some(crate::backup::BackupKey::from_text(text).ok_or_else(|| Error::Store("that is not a recovery key (64 hex digits)".into()))?),
            None => None,
        };
        let key_file = self.data_dir.join("backup.key");
        let local = if key_file.exists() { Some(self.backup_key()?) } else { None };
        let keys: Vec<_> = given.iter().chain(local.iter()).cloned().collect();
        let (manifest, chunks) = crate::backup::read(path, &keys).map_err(|e| Error::Store(format!("{}: {e}", path.display())))?;
        if let (Some(k), None) = (&given, &local) {
            if manifest.encrypted {
                k.save(&self.data_dir)?;
            }
        }
        if self.store.count()? > 0 {
            self.backup()?;
        }
        self.store.clear()?;
        let restored = self.add(&chunks)?;
        log::info!("restored {restored} chunks from a backup made {} by version {}", manifest.created, manifest.app_version);
        Ok(restored)
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
fn load_embedder(cache: &Path, provider: crate::governor::OnnxProvider, progress: &mut dyn FnMut(&str)) -> Result<Option<Box<dyn Embedder>>> {
    progress("Loading the search model (the first time downloads it)");
    Ok(Some(Box::new(crate::embed::LocalEmbedder::with_provider(cache, provider)?)))
}

#[cfg(not(feature = "embed"))]
fn load_embedder(_cache: &Path, _provider: crate::governor::OnnxProvider, _progress: &mut dyn FnMut(&str)) -> Result<Option<Box<dyn Embedder>>> {
    Ok(None)
}
