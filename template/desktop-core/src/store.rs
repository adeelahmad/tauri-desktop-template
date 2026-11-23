//! Where chunks live. `FileStore` is one JSON file and searches in memory, which is fast
//! up to tens of thousands of chunks; `pgstore::PostgresStore` (feature `postgres`)
//! keeps them in embedded Postgres with pgvector.

use crate::{cosine, Chunk, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub source: String,
    pub page: Option<u32>,
    pub text: String,
    pub score: f32,
}

pub trait Store: Send + Sync {
    fn kind(&self) -> &'static str;
    /// Add chunks, with one vector each when embeddings are on.
    fn add(&self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<usize>;
    /// Best `k` chunks: by vector when `query_vector` is given, else by keywords.
    fn search(&self, query: &str, query_vector: Option<&[f32]>, k: usize) -> Result<Vec<Hit>>;
    fn count(&self) -> Result<usize>;
    /// Every stored chunk, for re-embedding with a new model.
    fn all(&self) -> Result<Vec<Chunk>>;
    fn remove_source(&self, source: &str) -> Result<usize>;
    fn clear(&self) -> Result<()>;
    /// Name of the embedder the stored vectors came from (`None`: no vectors).
    fn embedder(&self) -> Result<Option<String>>;
    fn set_embedder(&self, name: Option<&str>) -> Result<()>;
}

/// Lower-cased words of 2+ letters or digits.
pub fn terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 1).map(str::to_lowercase).collect()
}

/// Share of the query's distinct terms found in `text`, in 0..=1.
pub fn keyword_score(query_terms: &HashSet<String>, text: &str) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let words: HashSet<String> = terms(text).into_iter().collect();
    query_terms.iter().filter(|t| words.contains(*t)).count() as f32 / query_terms.len() as f32
}

#[derive(Default, Serialize, Deserialize)]
struct FileData {
    embedder: Option<String>,
    records: Vec<Record>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    #[serde(flatten)]
    chunk: Chunk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

pub struct FileStore {
    path: PathBuf,
    data: Mutex<FileData>,
}

impl FileStore {
    /// Open (or start) `dir/chunks.json`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("chunks.json");
        let data = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileData::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, data: Mutex::new(data) })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, FileData>> {
        self.data.lock().map_err(|_| crate::Error::Store("lock poisoned".into()))
    }

    /// Write to a temporary file, then rename, so a crash never leaves half a file.
    fn persist(&self, data: &FileData) -> Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(data)?)?;
        std::fs::rename(&tmp, &self.path)?;
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

    fn search(&self, query: &str, query_vector: Option<&[f32]>, k: usize) -> Result<Vec<Hit>> {
        let data = self.lock()?;
        let q: HashSet<String> = terms(query).into_iter().collect();
        let mut hits: Vec<Hit> = data
            .records
            .iter()
            .map(|r| {
                let score = match (query_vector, &r.vector) {
                    (Some(qv), Some(v)) => cosine(qv, v),
                    _ => keyword_score(&q, &r.chunk.text),
                };
                Hit { source: r.chunk.source.clone(), page: r.chunk.page, text: r.chunk.text.clone(), score }
            })
            .filter(|h| query_vector.is_some() || h.score > 0.0)
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(k);
        Ok(hits)
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
