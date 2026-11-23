//! Local ONNX text embeddings through fastembed-rs: no API key, nothing leaves the machine.
//!
//! The model is chosen at build time (`APP_EMBED_MODEL` in `.cargo/config.toml`) and can be
//! overridden at run time with the same environment variable. Each model gets its task
//! prefixes; Nomic v1.5 is cut to `APP_EMBED_DIM` (default 256) Matryoshka dimensions.

use crate::{normalise, Embedder, Error, Result};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::path::Path;
use std::sync::Mutex;

/// Long chunks would pad a whole batch; embed the first ~200 tokens.
pub const MAX_CHARS: usize = 800;
const BATCH: usize = 16;

pub const MODELS: &[&str] = &["nomic-embed-text-v1.5", "bge-small-en-v1.5", "all-MiniLM-L6-v2", "multilingual-e5-small"];

struct Spec {
    model: EmbeddingModel,
    doc_prefix: &'static str,
    query_prefix: &'static str,
    /// Matryoshka truncation (Nomic only).
    dim: Option<usize>,
}

fn spec(name: &str) -> Result<Spec> {
    let s = |model, doc_prefix, query_prefix| Spec { model, doc_prefix, query_prefix, dim: None };
    Ok(match name {
        "nomic-embed-text-v1.5" => {
            let dim = std::env::var("APP_EMBED_DIM").ok().and_then(|d| d.parse().ok()).unwrap_or(256);
            Spec { dim: Some(dim), ..s(EmbeddingModel::NomicEmbedTextV15, "search_document: ", "search_query: ") }
        }
        "bge-small-en-v1.5" => s(EmbeddingModel::BGESmallENV15, "", "Represent this sentence for searching relevant passages: "),
        "all-MiniLM-L6-v2" => s(EmbeddingModel::AllMiniLML6V2, "", ""),
        "multilingual-e5-small" => s(EmbeddingModel::MultilingualE5Small, "passage: ", "query: "),
        other => return Err(Error::Embed(format!("unknown model {other:?}; use one of {MODELS:?}"))),
    })
}

/// The configured model name.
pub fn model_name() -> String {
    std::env::var("APP_EMBED_MODEL")
        .ok()
        .filter(|m| !m.is_empty())
        .or_else(|| option_env!("APP_EMBED_MODEL").map(String::from))
        .unwrap_or_else(|| MODELS[0].into())
}

pub struct LocalEmbedder {
    name: String,
    spec: Spec,
    model: Mutex<TextEmbedding>,
}

impl LocalEmbedder {
    /// Load the configured model, downloading it into `cache` the first time.
    pub fn new(cache: &Path) -> Result<Self> {
        let name = model_name();
        let spec = spec(&name)?;
        // fastembed downloads through its own client, so ask the network policy first.
        let code = TextEmbedding::get_model_info(&spec.model).map(|i| i.model_code.replace('/', "--")).unwrap_or_default();
        if !cache.join(format!("models--{code}")).is_dir() {
            crate::net::check("https://huggingface.co/").map_err(|e| Error::Embed(format!("the embedding model is not downloaded yet: {e}")))?;
        }
        let opts = TextInitOptions::new(spec.model.clone()).with_cache_dir(cache.to_path_buf()).with_show_download_progress(false).with_environment(crate::onnx_env()?);
        let model = TextEmbedding::try_new(opts).map_err(|e| Error::Embed(e.to_string()))?;
        Ok(Self { name, spec, model: Mutex::new(model) })
    }

    fn encode(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let texts: Vec<String> = texts.into_iter().map(|t| t.chars().take(MAX_CHARS).collect()).collect();
        let raw = self
            .model
            .lock()
            .map_err(|_| Error::Embed("model lock poisoned".into()))?
            .embed(texts, Some(BATCH))
            .map_err(|e| Error::Embed(e.to_string()))?;
        Ok(raw.into_iter().map(|v| self.post(v)).collect())
    }

    /// Nomic: layer norm, truncate, re-normalise. Others: normalise.
    fn post(&self, mut v: Vec<f32>) -> Vec<f32> {
        if let Some(dim) = self.spec.dim {
            let n = v.len() as f32;
            let mean = v.iter().sum::<f32>() / n;
            let std = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n).sqrt();
            v.iter_mut().for_each(|x| *x = (*x - mean) / (std + 1e-5));
            v.truncate(dim);
        }
        normalise(&mut v);
        v
    }
}

impl Embedder for LocalEmbedder {
    fn name(&self) -> String {
        match self.spec.dim {
            Some(d) => format!("{}@{d}", self.name),
            None => self.name.clone(),
        }
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.encode(texts.iter().map(|t| format!("{}{t}", self.spec.doc_prefix)).collect())
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.encode(vec![format!("{}{text}", self.spec.query_prefix)])?.remove(0))
    }
}
