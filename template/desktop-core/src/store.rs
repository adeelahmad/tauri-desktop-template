//! Where chunks live. `FileStore` is one JSON file and searches in memory (cosine and
//! BM25), which is fast up to tens of thousands of chunks; `pgstore::PostgresStore`
//! (feature `postgres`) keeps them in embedded Postgres with pgvector.
//!
//! Implement [`Store`] to add a backend; [`bm25_scores`], [`terms`] and [`plan`] are shared
//! helpers, and [`crate::fuse`] merges lists in hybrid mode.
//!
//! # Example
//!
//! ```
//! use desktop_core::{chunks_from_text, FileStore, SearchConfig, Store};
//!
//! # let dir = std::env::temp_dir().join(format!("desktop-core-store-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let store = FileStore::open(&dir)?;
//! store.add(&chunks_from_text("fruit.md", "Apples are red."), None)?;
//! store.add(&chunks_from_text("veg.md", "Carrots are orange."), None)?;
//! let hits = store.search("apples", None, &SearchConfig::default())?;
//! assert_eq!(hits[0].source, "fruit.md");
//! assert_eq!(store.count()?, 2);
//! # let _ = std::fs::remove_dir_all(&dir);
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::config::{SearchConfig, SearchMode};
use crate::fuse::{self, Candidate};
use crate::{cosine, Chunk, Result};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// One search result: a chunk, where it came from and how it scored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Hit {
    /// The source path or name of the chunk.
    pub source: String,
    /// 1-based page for PDFs.
    pub page: Option<u32>,
    /// The chunk's text.
    pub text: String,
    /// The final score: cosine, keyword score, or the fused RRF score in hybrid mode.
    pub score: f32,
    /// Hybrid mode: the raw scores from each list, when the chunk was in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f32>,
    /// Hybrid mode: the raw keyword score, when the chunk was in that list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_score: Option<f32>,
    /// File hash, extraction method, model and time of indexing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prov: Option<crate::chunk::Provenance>,
}

/// What a search runs once the mode meets what is available (no vector: keywords).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Nearest vectors only.
    Vector,
    /// Keyword ranking only.
    Keyword,
    /// Both lists, fused with reciprocal rank fusion.
    Hybrid,
}

/// The plan for `mode` when a query vector is (or is not) available.
pub fn plan(mode: SearchMode, has_vector: bool) -> Plan {
    match (mode, has_vector) {
        (_, false) | (SearchMode::Keyword, _) => Plan::Keyword,
        (SearchMode::Hybrid, true) => Plan::Hybrid,
        _ => Plan::Vector,
    }
}

/// A place chunks are kept and searched. Implement it to add a backend (see
/// [`FileStore`] for a small one).
pub trait Store: Send + Sync {
    /// A short name for status displays (`file`, `postgres`).
    fn kind(&self) -> &'static str;
    /// Add chunks, with one vector each when embeddings are on.
    fn add(&self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<usize>;
    /// Best chunks for `cfg` (mode, top k, thresholds, fusion weights).
    fn search(&self, query: &str, query_vector: Option<&[f32]>, cfg: &SearchConfig) -> Result<Vec<Hit>>;
    /// Chunks stored.
    fn count(&self) -> Result<usize>;
    /// Every stored chunk, for re-embedding with a new model.
    fn all(&self) -> Result<Vec<Chunk>>;
    /// Remove every chunk of `source`; returns how many went.
    fn remove_source(&self, source: &str) -> Result<usize>;
    /// Remove every chunk.
    fn clear(&self) -> Result<()>;
    /// Name of the embedder the stored vectors came from (`None`: no vectors).
    fn embedder(&self) -> Result<Option<String>>;
    /// Record the embedder the stored vectors come from.
    fn set_embedder(&self, name: Option<&str>) -> Result<()>;
    /// Something the user should know about this store while it works (for example, an
    /// embedded Postgres that still runs an old major version); `None` when all is well.
    fn notice(&self) -> Option<String> {
        None
    }
    /// New settings (the Postgres store sets up BM25 here when it is turned on).
    fn configure(&self, _config: &crate::config::Config) -> Result<()> {
        Ok(())
    }
}

/// Lower-cased words of 2+ letters or digits.
pub fn terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 1).map(str::to_lowercase).collect()
}

/// Okapi BM25 over the given texts (k1 = 1.2, b = 0.75), scored for `query`.
pub fn bm25_scores<'a>(query: &str, texts: impl Iterator<Item = &'a str>) -> Vec<f32> {
    let docs: Vec<Vec<String>> = texts.map(terms).collect();
    let n = docs.len() as f32;
    if n == 0.0 {
        return Vec::new();
    }
    let avg = docs.iter().map(Vec::len).sum::<usize>() as f32 / n;
    let q: HashSet<String> = terms(query).into_iter().collect();
    let mut df: HashMap<&str, f32> = HashMap::new();
    for d in &docs {
        let seen: HashSet<&str> = d.iter().map(String::as_str).filter(|t| q.contains(*t)).collect();
        for t in seen {
            *df.entry(t).or_default() += 1.0;
        }
    }
    docs.iter()
        .map(|d| {
            let len = d.len() as f32;
            q.iter()
                .map(|t| {
                    let f = d.iter().filter(|w| *w == t).count() as f32;
                    if f == 0.0 {
                        return 0.0;
                    }
                    let df = df.get(t.as_str()).copied().unwrap_or(0.0);
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * len / avg.max(1.0)))
                })
                .sum()
        })
        .collect()
}

/// The file store's format: 1 added chunk provenance.
const FILE_FORMAT: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
struct FileData {
    /// Format version (see `FILE_FORMAT`); files from a newer app are refused, not damaged.
    #[serde(default)]
    version: u32,
    embedder: Option<String>,
    records: Vec<Record>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
struct Record {
    #[serde(flatten)]
    chunk: Chunk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

/// Chunks and vectors in one JSON file (`chunks.json`), searched in memory.
pub struct FileStore {
    path: PathBuf,
    data: Mutex<FileData>,
}

impl FileStore {
    /// Open (or start) `dir/chunks.json`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("chunks.json");
        let mut data: FileData = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileData::default(),
            Err(e) => return Err(e.into()),
        };
        if data.version > FILE_FORMAT {
            return Err(crate::Error::Store(format!("{} was written by a newer version of this app (format {}, this one reads up to {FILE_FORMAT}); update the app", path.display(), data.version)));
        }
        // Older formats only lack fields that have defaults, so they upgrade on the next write.
        data.version = FILE_FORMAT;
        Ok(Self { path, data: Mutex::new(data) })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, FileData>> {
        self.data.lock().map_err(|_| crate::Error::Store("lock poisoned".into()))
    }

    /// Write to a temporary file, then rename, so a crash never leaves half a file.
    fn persist(&self, data: &FileData) -> Result<()> {
        // Synced before the rename, so even a power cut leaves the old or the new file.
        crate::jobs::write_atomic(&self.path, &serde_json::to_vec(data)?)?;
        Ok(())
    }
}

impl Store for FileStore {
    fn kind(&self) -> &'static str {
        "file"
    }

    fn add(&self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<usize> {
        let mut data = self.lock()?;
        for (i, c) in chunks.iter().enumerate() {
            data.records.push(Record { chunk: c.clone(), vector: vectors.map(|v| v[i].clone()) });
        }
        self.persist(&data)?;
        Ok(chunks.len())
    }

    fn search(&self, query: &str, query_vector: Option<&[f32]>, cfg: &SearchConfig) -> Result<Vec<Hit>> {
        let data = self.lock()?;
        let hit = |r: &Record, score: f32| Hit {
            source: r.chunk.source.clone(),
            page: r.chunk.page,
            text: r.chunk.text.clone(),
            score,
            vector_score: None,
            keyword_score: None,
            prov: r.chunk.prov.clone(),
        };
        let ranked = |mut list: Vec<(usize, f32)>, n: usize| {
            list.sort_by(|a, b| b.1.total_cmp(&a.1));
            list.truncate(n);
            list
        };
        let plan = plan(cfg.mode, query_vector.is_some());
        let n = if plan == Plan::Hybrid { cfg.candidates } else { cfg.top_k };
        let vector = || {
            let qv = query_vector.unwrap_or_default();
            let scores = data.records.iter().enumerate().filter_map(|(i, r)| r.vector.as_ref().map(|v| (i, cosine(qv, v)))).collect();
            ranked(scores, n)
        };
        let keyword = || {
            let scores = bm25_scores(query, data.records.iter().map(|r| r.chunk.text.as_str()));
            ranked(scores.into_iter().enumerate().filter(|(_, s)| *s > 0.0).collect(), n)
        };
        Ok(match plan {
            Plan::Vector => fuse::single(vector().into_iter().map(|(i, s)| hit(&data.records[i], s)).collect(), cfg.min_vector_score, cfg),
            Plan::Keyword => fuse::single(keyword().into_iter().map(|(i, s)| hit(&data.records[i], s)).collect(), cfg.min_keyword_score, cfg),
            Plan::Hybrid => {
                let cands = |list: Vec<(usize, f32)>| list.into_iter().map(|(i, s)| Candidate { key: i as u64, hit: hit(&data.records[i], s) }).collect();
                fuse::rrf(cands(vector()), cands(keyword()), cfg)
            }
        })
    }

    fn count(&self) -> Result<usize> {
        Ok(self.lock()?.records.len())
    }

    fn all(&self) -> Result<Vec<Chunk>> {
        Ok(self.lock()?.records.iter().map(|r| r.chunk.clone()).collect())
    }

    fn remove_source(&self, source: &str) -> Result<usize> {
        let mut data = self.lock()?;
        let before = data.records.len();
        data.records.retain(|r| r.chunk.source != source);
        let removed = before - data.records.len();
        self.persist(&data)?;
        Ok(removed)
    }

    fn clear(&self) -> Result<()> {
        let mut data = self.lock()?;
        data.records.clear();
        self.persist(&data)
    }

    fn embedder(&self) -> Result<Option<String>> {
        Ok(self.lock()?.embedder.clone())
    }

    fn set_embedder(&self, name: Option<&str>) -> Result<()> {
        let mut data = self.lock()?;
        data.embedder = name.map(String::from);
        self.persist(&data)
    }
}
