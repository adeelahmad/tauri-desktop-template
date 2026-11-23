//! Every tunable of search, ingest and folder watching, in one serde struct.
//!
//! It lives in the app's `settings.json` (under `config`), travels to the worker process
//! with `EngineOptions`, and can be changed at runtime with `Engine::configure`. Every
//! optional behaviour is off by default: search is plain vector (or keyword) search,
//! ingest uses one worker, and no folder is watched. Unknown or missing keys fall back to
//! the defaults, and `validated` clamps values into safe ranges.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub search: SearchConfig,
    pub ingest: IngestConfig,
    pub watch: WatchConfig,
    /// Log level and file rotation.
    pub log: crate::logging::LogConfig,
{% if ai %}    #[cfg(feature = "ai")]
    pub llm: crate::llm::LlmConfig,
{% endif %}{% if mcp_server %}    #[cfg(feature = "mcp")]
    pub mcp: crate::mcp::McpConfig,
{% endif %}{% if tts %}    #[cfg(feature = "tts")]
    pub tts: crate::tts::TtsConfig,
{% endif %}{% if stt %}    #[cfg(feature = "stt")]
    pub stt: crate::stt::SttConfig,
{% endif %}{% if telemetry %}    #[cfg(feature = "telemetry")]
    pub telemetry: crate::telemetry::TelemetryConfig,
{% endif %}}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// Vector search when a model is loaded, keyword search otherwise.
    #[default]
    Auto,
    Vector,
    Keyword,
    /// Vector and keyword results fused with weighted reciprocal rank fusion.
    Hybrid,
}

/// How the Postgres store ranks keywords.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeywordEngine {
    /// BM25 (VectorChord-BM25) when it is built in and installed, else full text.
    #[default]
    Auto,
    Bm25,
    /// Postgres full-text search (`ts_rank_cd`).
    Fulltext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchConfig {
    pub mode: SearchMode,
    /// Results returned.
    pub top_k: usize,
    /// Candidates taken from each list before fusing (hybrid).
    pub candidates: usize,
    /// Drop vector hits below this cosine similarity (-1..1).
    pub min_vector_score: Option<f32>,
    /// Drop keyword hits below this score (BM25, full-text rank or term share, by store).
    pub min_keyword_score: Option<f32>,
    /// Drop final results below this score (the fused score in hybrid mode).
    pub min_score: Option<f32>,
    /// The `k` of reciprocal rank fusion: higher flattens the rank curve.
    pub rrf_k: f32,
    pub vector_weight: f32,
    pub keyword_weight: f32,
    pub keyword_engine: KeywordEngine,
    /// pg_tokenizer model for BM25 (`bert_base_uncased`, `llmlingua2`, `gemma2b`, ...).
    pub bm25_tokenizer: String,
    /// Postgres text-search configuration for full-text ranking.
    pub fulltext_language: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            mode: SearchMode::Auto,
            top_k: 10,
            candidates: 50,
            min_vector_score: None,
            min_keyword_score: None,
            min_score: None,
            rrf_k: 60.0,
            vector_weight: 1.0,
            keyword_weight: 1.0,
            keyword_engine: KeywordEngine::Auto,
            bm25_tokenizer: "bert_base_uncased".into(),
            fulltext_language: "english".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IngestConfig {
    /// Files read, chunked and embedded at once. 0 means one per CPU core.
    pub workers: usize,
    /// Chunks aim for at most this many characters.
    pub chunk_chars: usize,
    /// Characters repeated from the end of one chunk at the start of the next.
    pub chunk_overlap: usize,
    /// Chunks per embedding call.
    pub embed_batch: usize,
    /// Larger files are skipped.
    pub max_file_mb: u64,
    /// File types ingested from folders (lower case, no dot).
    pub extensions: Vec<String>,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            workers: 1,
            chunk_chars: 800,
            chunk_overlap: 0,
            embed_batch: 64,
            max_file_mb: 64,
            extensions: ["txt", "md", "markdown", "pdf", "csv", "json", "html", "htm", "rst", "org"].map(String::from).to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WatchConfig {
    pub enabled: bool,
    /// Watch the built-in drop folder.
    pub drop_folder: bool,
    /// Where the drop folder is (default: `<Documents>/<App> Inbox`).
    pub drop_folder_path: Option<PathBuf>,
    /// More folders to watch.
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    /// Wait this long after the last change to a file before ingesting it.
    pub debounce_ms: u64,
    /// Ingest what is already in a folder when watching starts.
    pub ingest_existing: bool,
    /// Remove a file's chunks when it is deleted.
    pub remove_deleted: bool,
    /// File names to skip (`*` and `?` wildcards).
    pub ignore: Vec<String>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            drop_folder: true,
            drop_folder_path: None,
            paths: Vec::new(),
            recursive: true,
            debounce_ms: 2000,
            ingest_existing: true,
            remove_deleted: true,
            ignore: [".*", "~$*", "*.tmp", "*.part", "*.crdownload", "Thumbs.db"].map(String::from).to_vec(),
        }
    }
}

impl Config {
    /// The same config with every number in a safe range.
    pub fn validated(mut self) -> Self {
        self.log = self.log.validated();
        let s = &mut self.search;
        s.top_k = s.top_k.clamp(1, 1000);
        s.candidates = s.candidates.clamp(s.top_k, 5000);
        s.rrf_k = if s.rrf_k.is_finite() { s.rrf_k.clamp(1.0, 1000.0) } else { 60.0 };
        for w in [&mut s.vector_weight, &mut s.keyword_weight] {
            *w = if w.is_finite() { w.clamp(0.0, 100.0) } else { 1.0 };
        }
        for t in [&mut s.min_vector_score, &mut s.min_keyword_score, &mut s.min_score] {
            *t = t.filter(|v| v.is_finite());
        }
        if !valid_ident(&s.bm25_tokenizer) {
            s.bm25_tokenizer = SearchConfig::default().bm25_tokenizer;
        }
        if !valid_ident(&s.fulltext_language) {
            s.fulltext_language = SearchConfig::default().fulltext_language;
        }
        let i = &mut self.ingest;
        i.workers = i.workers.min(64);
        i.chunk_chars = i.chunk_chars.clamp(100, 20_000);
        i.chunk_overlap = i.chunk_overlap.min(i.chunk_chars / 2);
        i.embed_batch = i.embed_batch.clamp(1, 1024);
        i.max_file_mb = i.max_file_mb.clamp(1, 4096);
        i.extensions = i.extensions.iter().map(|e| e.trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
        let w = &mut self.watch;
        w.debounce_ms = w.debounce_ms.clamp(100, 600_000);
{% if ai %}        #[cfg(feature = "ai")]
        {
            self.llm = self.llm.validated();
        }
{% endif %}{% if mcp_server %}        #[cfg(feature = "mcp")]
        {
            self.mcp = self.mcp.validated();
        }
{% endif %}{% if tts %}        #[cfg(feature = "tts")]
        {
            self.tts = self.tts.validated();
        }
{% endif %}{% if stt %}        #[cfg(feature = "stt")]
        {
            self.stt = self.stt.validated();
        }
{% endif %}{% if telemetry %}        #[cfg(feature = "telemetry")]
        {
            self.telemetry = self.telemetry.validated();
        }
{% endif %}        self
    }

    /// Ingest workers to run: `workers`, or one per core when it is 0.
    pub fn workers(&self) -> usize {
        match self.ingest.workers {
            0 => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
            n => n,
        }
    }
}

/// Names that go into SQL as identifiers or literals: letters, digits and `_` only.
pub fn valid_ident(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_turn_everything_optional_off() {
        let c = Config::default();
        assert_eq!(c.search.mode, SearchMode::Auto);
        assert!(!c.watch.enabled);
        assert_eq!(c.ingest.workers, 1);
    }

    #[test]
    fn missing_keys_use_defaults_and_bad_values_are_clamped() {
        let c: Config = serde_json::from_str(r#"{"search": {"topK": 0, "rrfK": -5, "bm25Tokenizer": "x'; drop"}, "ingest": {"chunkOverlap": 99999}}"#).unwrap();
        let c = c.validated();
        assert_eq!(c.search.top_k, 1);
        assert_eq!(c.search.rrf_k, 1.0);
        assert_eq!(c.search.bm25_tokenizer, "bert_base_uncased");
        assert_eq!(c.ingest.chunk_overlap, 400);
        assert_eq!(c.watch, WatchConfig::default());
    }
}
