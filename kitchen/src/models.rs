//! Models built into the installer ("bundled models"): the catalog of models an app can
//! ship, its `models.lock`, and `kitchen models list|add|remove|fetch|verify`.
//!
//! An app generated with `models = bundle` (or `bundle_with_fallback`) lists the models it
//! ships in `src-tauri/resources/models/models.lock`, one file per line with its URL, size
//! and SHA-256. `kitchen models fetch` (or `make models`) downloads them next to the lock
//! and checks every checksum; Tauri then copies that folder into the installer. At run time
//! the app loads them from there (`desktop_core::models`) and downloads nothing.
//!
//! ```
//! use kitchen::models::{self, CATALOG};
//!
//! // The answers pick the models: Whisper base and the voice here.
//! let answers = |k: &str| match k {
//!     "models" => Some("bundle".to_string()),
//!     "bundle_stt_model" | "bundle_tts_voice" => Some("true".to_string()),
//!     _ => None,
//! };
//! let chosen = models::chosen(answers);
//! assert_eq!(chosen.iter().map(|m| m.name).collect::<Vec<_>>(), ["whisper-base", "tiny-voca"]);
//! let lock = models::lock_text(&chosen);
//! assert!(lock.lines().any(|l| l.starts_with("stt whisper-base encoder.onnx ")));
//! assert!(models::total(&chosen) < models::WARN_BYTES);
//! assert!(CATALOG.iter().all(|m| !m.licence.is_empty()));
//! ```

use crate::doctor::{Check, Status};
use crate::lifecycle::{Plan, Step};
use crate::{Error, Options, Report, Result};
use std::path::{Path, PathBuf};

/// One model an app can bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogModel {
    /// Its name in the lock and its folder (`whisper-base`).
    pub name: &'static str,
    /// What it is for: `embed`, `stt`, `tts` or `llm`.
    pub kind: &'static str,
    /// The generation answer that bundles it (`bundle_stt_model`); empty when only
    /// `kitchen models add` adds it.
    pub answer: &'static str,
    /// Also needs this text answer to have this value (`embedding_model`).
    pub when: Option<(&'static str, &'static str)>,
    /// The model's licence (SPDX). Bundling redistributes the model under it.
    pub licence: &'static str,
    /// Where the model is published (its model card).
    pub home: &'static str,
    /// `(path in the model folder, bytes, sha256, url)`.
    pub files: &'static [(&'static str, u64, &'static str, &'static str)],
}

impl CatalogModel {
    /// The total size of its files.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.1).sum()
    }

    /// Its lines in `models.lock`.
    pub fn lock_lines(&self) -> String {
        self.files
            .iter()
            .map(|(path, size, sha, url)| {
                format!("{} {} {path} {size} {sha} {url}\n", self.kind, self.name)
            })
            .collect()
    }
}

/// Every model Kitchen knows how to bundle, with pinned revisions and checksums.
pub const CATALOG: &[CatalogModel] = &[
    CatalogModel {
        name: "embed-nomic-embed-text-v1.5",
        kind: "embed",
        answer: "bundle_embedding_model",
        when: Some(("embedding_model", "nomic-embed-text-v1.5")),
        licence: "Apache-2.0",
        home: "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5",
        files: &[
            ("models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab/onnx/model.onnx", 547310275, "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965", "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/onnx/model.onnx"),
            ("models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab/tokenizer.json", 711396, "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66", "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/tokenizer.json"),
            ("models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab/config.json", 2538, "9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b", "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/config.json"),
            ("models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab/special_tokens_map.json", 695, "5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a", "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/special_tokens_map.json"),
            ("models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab/tokenizer_config.json", 1191, "d7e0000bcc80134debd2222220427e6bf5fa20a669f40a0d0d1409cc18e0a9bc", "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/tokenizer_config.json"),
        ],
    },
    CatalogModel {
        name: "embed-bge-small-en-v1.5",
        kind: "embed",
        answer: "bundle_embedding_model",
        when: Some(("embedding_model", "bge-small-en-v1.5")),
        licence: "MIT",
        home: "https://huggingface.co/BAAI/bge-small-en-v1.5",
        files: &[
            ("models--Xenova--bge-small-en-v1.5/snapshots/ea104dacec62c0de699686887e3f920caeb4f3e3/onnx/model.onnx", 133093490, "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35", "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/onnx/model.onnx"),
            ("models--Xenova--bge-small-en-v1.5/snapshots/ea104dacec62c0de699686887e3f920caeb4f3e3/tokenizer.json", 711396, "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66", "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/tokenizer.json"),
            ("models--Xenova--bge-small-en-v1.5/snapshots/ea104dacec62c0de699686887e3f920caeb4f3e3/config.json", 683, "fa73f90bf92c8cace1fbcb709626306f2bdbc9ea3e5b5f94b440df9b6aa56350", "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/config.json"),
            ("models--Xenova--bge-small-en-v1.5/snapshots/ea104dacec62c0de699686887e3f920caeb4f3e3/special_tokens_map.json", 125, "b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3", "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/special_tokens_map.json"),
            ("models--Xenova--bge-small-en-v1.5/snapshots/ea104dacec62c0de699686887e3f920caeb4f3e3/tokenizer_config.json", 366, "9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3", "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/tokenizer_config.json"),
        ],
    },
    CatalogModel {
        name: "embed-all-MiniLM-L6-v2",
        kind: "embed",
        answer: "bundle_embedding_model",
        when: Some(("embedding_model", "all-MiniLM-L6-v2")),
        licence: "Apache-2.0",
        home: "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2",
        files: &[
            ("models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/d13954661f83248295ba75c1ed411eef3b7b936e/model.onnx", 90387630, "bbd7b466f6d58e646fdc2bd5fd67b2f5e93c0b687011bd4548c420f7bd46f0c5", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/d13954661f83248295ba75c1ed411eef3b7b936e/model.onnx"),
            ("models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/d13954661f83248295ba75c1ed411eef3b7b936e/tokenizer.json", 711649, "851ca67100d372ca3ae031a6abd168f53489eebfd7d89523f35c5c9b4d372c3c", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/d13954661f83248295ba75c1ed411eef3b7b936e/tokenizer.json"),
            ("models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/d13954661f83248295ba75c1ed411eef3b7b936e/config.json", 650, "1b4d8e2a3988377ed8b519a31d8d31025a25f1c5f8606998e8014111438efcd7", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/d13954661f83248295ba75c1ed411eef3b7b936e/config.json"),
            ("models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/d13954661f83248295ba75c1ed411eef3b7b936e/special_tokens_map.json", 695, "5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/d13954661f83248295ba75c1ed411eef3b7b936e/special_tokens_map.json"),
            ("models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/d13954661f83248295ba75c1ed411eef3b7b936e/tokenizer_config.json", 1412, "abda01c8c14c5151ae498aceb30db406d6b91242c394fd88d3b6fd5a63a101e6", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/d13954661f83248295ba75c1ed411eef3b7b936e/tokenizer_config.json"),
        ],
    },
    CatalogModel {
        name: "embed-multilingual-e5-small",
        kind: "embed",
        answer: "bundle_embedding_model",
        when: Some(("embedding_model", "multilingual-e5-small")),
        licence: "MIT",
        home: "https://huggingface.co/intfloat/multilingual-e5-small",
        files: &[
            ("models--intfloat--multilingual-e5-small/snapshots/614241f622f53c4eeff9890bdc4f31cfecc418b3/onnx/model.onnx", 470268510, "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665", "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/onnx/model.onnx"),
            ("models--intfloat--multilingual-e5-small/snapshots/614241f622f53c4eeff9890bdc4f31cfecc418b3/tokenizer.json", 17082730, "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39", "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/tokenizer.json"),
            ("models--intfloat--multilingual-e5-small/snapshots/614241f622f53c4eeff9890bdc4f31cfecc418b3/config.json", 655, "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959", "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/config.json"),
            ("models--intfloat--multilingual-e5-small/snapshots/614241f622f53c4eeff9890bdc4f31cfecc418b3/special_tokens_map.json", 167, "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7", "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/special_tokens_map.json"),
            ("models--intfloat--multilingual-e5-small/snapshots/614241f622f53c4eeff9890bdc4f31cfecc418b3/tokenizer_config.json", 443, "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b", "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/tokenizer_config.json"),
        ],
    },
    CatalogModel {
        name: "whisper-tiny",
        kind: "stt",
        answer: "",
        when: None,
        licence: "MIT",
        home: "https://huggingface.co/openai/whisper-tiny",
        files: &[
            ("encoder.onnx", 10124990, "2af4a414ca47aa30f61246017e5fe82b0a8d229281d1255ba666a2a7f6b84d19", "https://huggingface.co/onnx-community/whisper-tiny/resolve/ff4177021cc41f7db950912b73ea4fdf7d01d8e7/onnx/encoder_model_quantized.onnx"),
            ("decoder.onnx", 30461071, "2a160a2a5ccf901f83aa4d4ceec580a269bd2cec582871e0816e98c610594f12", "https://huggingface.co/onnx-community/whisper-tiny/resolve/ff4177021cc41f7db950912b73ea4fdf7d01d8e7/onnx/decoder_model_quantized.onnx"),
            ("vocab.json", 1036584, "50d6a919f0a0601d56a04eb583c780d18553aa388254ba3158eb6a00f13e2c1a", "https://huggingface.co/onnx-community/whisper-tiny/resolve/ff4177021cc41f7db950912b73ea4fdf7d01d8e7/vocab.json"),
            ("added_tokens.json", 34604, "9715fd2243b6f06a5858b5e32950d2853f73dd5bc201aafcf76f5082a2d8acd1", "https://huggingface.co/onnx-community/whisper-tiny/resolve/ff4177021cc41f7db950912b73ea4fdf7d01d8e7/added_tokens.json"),
        ],
    },
    CatalogModel {
        name: "whisper-base",
        kind: "stt",
        answer: "bundle_stt_model",
        when: None,
        licence: "MIT",
        home: "https://huggingface.co/openai/whisper-base",
        files: &[
            ("encoder.onnx", 23201314, "5862993336bf33acd23736071aae2b32261d3b1b2f37780194460d4ef974dd46", "https://huggingface.co/onnx-community/whisper-base/resolve/27a07b4b52b3a4c5886134bb4320e93e1a4bbc7c/onnx/encoder_model_quantized.onnx"),
            ("decoder.onnx", 53310178, "c859636f4463dd5a9ea752546c6c7badeb48ab3e22b340cafef08a15552fcb2e", "https://huggingface.co/onnx-community/whisper-base/resolve/27a07b4b52b3a4c5886134bb4320e93e1a4bbc7c/onnx/decoder_model_quantized.onnx"),
            ("vocab.json", 1036584, "50d6a919f0a0601d56a04eb583c780d18553aa388254ba3158eb6a00f13e2c1a", "https://huggingface.co/onnx-community/whisper-base/resolve/27a07b4b52b3a4c5886134bb4320e93e1a4bbc7c/vocab.json"),
            ("added_tokens.json", 34604, "9715fd2243b6f06a5858b5e32950d2853f73dd5bc201aafcf76f5082a2d8acd1", "https://huggingface.co/onnx-community/whisper-base/resolve/27a07b4b52b3a4c5886134bb4320e93e1a4bbc7c/added_tokens.json"),
        ],
    },
    CatalogModel {
        name: "whisper-small",
        kind: "stt",
        answer: "",
        when: None,
        licence: "MIT",
        home: "https://huggingface.co/openai/whisper-small",
        files: &[
            ("encoder.onnx", 92326160, "a43a83f3c5361cd591cfa7c36f14b43cf7cb22f47a415cc14a8d557be800fa92", "https://huggingface.co/onnx-community/whisper-small/resolve/36050c46d777d46dc4b5f43f6d90574fc38f8732/onnx/encoder_model_quantized.onnx"),
            ("decoder.onnx", 155989346, "0486afa8eb464bf2210d07aca0cb3f5d91d3f2f01cbe47f9f1e6ff9122abaaf6", "https://huggingface.co/onnx-community/whisper-small/resolve/36050c46d777d46dc4b5f43f6d90574fc38f8732/onnx/decoder_model_quantized.onnx"),
            ("vocab.json", 1036584, "50d6a919f0a0601d56a04eb583c780d18553aa388254ba3158eb6a00f13e2c1a", "https://huggingface.co/onnx-community/whisper-small/resolve/36050c46d777d46dc4b5f43f6d90574fc38f8732/vocab.json"),
            ("added_tokens.json", 34604, "9715fd2243b6f06a5858b5e32950d2853f73dd5bc201aafcf76f5082a2d8acd1", "https://huggingface.co/onnx-community/whisper-small/resolve/36050c46d777d46dc4b5f43f6d90574fc38f8732/added_tokens.json"),
        ],
    },
    CatalogModel {
        name: "tiny-voca",
        kind: "tts",
        answer: "bundle_tts_voice",
        when: None,
        licence: "Apache-2.0",
        home: "https://huggingface.co/n4ze3m/tiny-voca",
        files: &[
            ("model.onnx", 21386372, "fe6d918791a15b82e37465981d7f27a070e46b46635e5a3482fc8dff87a1ae46", "https://huggingface.co/n4ze3m/tiny-voca/resolve/5fe06641d0afecb3c04bc9faca2cf64d9b2b1600/onnx/model.onnx"),
            ("lexicon.tsv", 3454357, "3fc930345c6b3cabc533981c4b212fa309eec83913fcdbd42919338270d64b06", "https://huggingface.co/n4ze3m/tiny-voca/resolve/5fe06641d0afecb3c04bc9faca2cf64d9b2b1600/phonemizer/lexicon.tsv"),
        ],
    },
    CatalogModel {
        name: "qwen3-0.6b-q4",
        kind: "llm",
        answer: "bundle_llm",
        when: None,
        licence: "Apache-2.0",
        home: "https://huggingface.co/Qwen/Qwen3-0.6B",
        files: &[
            ("Qwen3-0.6B-Q4_K_M.gguf", 396705472, "ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a", "https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/50968a4468ef4233ed78cd7c3de230dd1d61a56b/Qwen3-0.6B-Q4_K_M.gguf"),
        ],
    },
];

/// Where the lock lives in an app.
pub const LOCK: &str = "src-tauri/resources/models/models.lock";
/// An installer above this many bytes of models gets a warning (about 200 MB).
pub const WARN_BYTES: u64 = 200 * 1000 * 1000;
/// GitHub refuses release assets above 2 GiB.
pub const RELEASE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
/// The lock's first lines.
pub const LOCK_HEADER: &str = "# Models built into the installer (docs/GUIDE.md, \"Bundling models\").\n# kind model file bytes sha256 url\n";

/// The catalog entry called `name`.
pub fn find(name: &str) -> Option<&'static CatalogModel> {
    CATALOG.iter().find(|m| m.name == name)
}

/// Whether the answers bundle models at all (`models` is `bundle` or `bundle_with_fallback`).
pub fn bundling(get: impl Fn(&str) -> Option<String>) -> bool {
    get("models").is_some_and(|m| m == "bundle" || m == "bundle_with_fallback")
}

/// The models the answers bundle (`get(name)` is an answer as text).
pub fn chosen(get: impl Fn(&str) -> Option<String>) -> Vec<&'static CatalogModel> {
    if !bundling(&get) {
        return Vec::new();
    }
    CATALOG
        .iter()
        .filter(|m| !m.answer.is_empty() && get(m.answer).as_deref() == Some("true"))
        .filter(|m| m.when.is_none_or(|(k, v)| get(k).as_deref() == Some(v)))
        .collect()
}

/// The size of `models` together.
pub fn total(models: &[&CatalogModel]) -> u64 {
    models.iter().map(|m| m.bytes()).sum()
}

/// `bytes` in MB, rounded.
pub fn mb(bytes: u64) -> String {
    format!("{} MB", (bytes + 500_000) / 1_000_000)
}

/// What to know about an installer that bundles `bytes` of models (`llm`: one is a GGUF):
/// the size, and warnings past [`WARN_BYTES`] and for large models.
pub fn size_notes(bytes: u64, llm: bool) -> (String, Vec<String>) {
    let size = format!(
        "bundled models add {} to the installer (and to every full update download)",
        mb(bytes)
    );
    let mut warn = Vec::new();
    if bytes > WARN_BYTES {
        warn.push(format!("the installer grows past 200 MB ({} of models): bundle fewer models (`kitchen models remove <name>`), or let them download on first use", mb(bytes)));
    }
    if llm || bytes > RELEASE_LIMIT / 2 {
        warn.push("a large model inside the installer: GitHub refuses release assets over 2 GB, and macOS notarization uploads the whole installer to Apple (slow, and large uploads can time out); keep the installer well under 2 GB".into());
    }
    (size, warn)
}

/// The lock for `models`.
pub fn lock_text(models: &[&CatalogModel]) -> String {
    let mut s = String::from(LOCK_HEADER);
    for m in models {
        s.push_str(&m.lock_lines());
    }
    s
}

/// The template's `models.lock` (Liquid): what [`lock_text`] gives for the answers.
pub fn lock_liquid() -> String {
    let mut s = String::from(LOCK_HEADER);
    for m in CATALOG.iter().filter(|m| !m.answer.is_empty()) {
        let (open, close) = match m.when {
            Some((k, v)) => (
                format!("{{% if {} %}}{{% if {k} == \"{v}\" %}}", m.answer),
                "{% endif %}{% endif %}",
            ),
            None => (format!("{{% if {} %}}", m.answer), "{% endif %}"),
        };
        s.push_str(&open);
        s.push_str(&m.lock_lines());
        s.push_str(close);
    }
    s.push('\n');
    s
}

/// One line of an app's lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockLine {
    /// `embed`, `stt`, `tts` or `llm`.
    pub kind: String,
    /// The model.
    pub model: String,
    /// The file inside the model folder.
    pub path: String,
    /// Its size.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: String,
    /// Where it downloads from.
    pub url: String,
}

/// Parse a lock's text (comments and blank lines skipped).
pub fn parse_lock(text: &str) -> Result<Vec<LockLine>> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let size = f.get(3).and_then(|s| s.parse().ok());
        match (&f[..], size) {
            ([kind, model, path, _, sha, url], Some(size)) => out.push(LockLine {
                kind: kind.to_string(),
                model: model.to_string(),
                path: path.to_string(),
                size,
                sha256: sha.to_string(),
                url: url.to_string(),
            }),
            _ => {
                return Err(Error::Usage(format!(
                    "{LOCK} line {}: expected `kind model file bytes sha256 url`",
                    n + 1
                )))
            }
        }
    }
    Ok(out)
}

fn lock_path(root: &Path) -> PathBuf {
    root.join(LOCK)
}

fn read(root: &Path) -> Result<Vec<LockLine>> {
    match std::fs::read_to_string(lock_path(root)) {
        Ok(t) => parse_lock(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::Io(format!("read {LOCK}"), e)),
    }
}

fn write(root: &Path, lines: &[LockLine]) -> Result<()> {
    let mut s = String::from(LOCK_HEADER);
    for l in lines {
        s.push_str(&format!(
            "{} {} {} {} {} {}\n",
            l.kind, l.model, l.path, l.size, l.sha256, l.url
        ));
    }
    let p = lock_path(root);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::Io(format!("create {}", d.display()), e))?;
    }
    std::fs::write(&p, s).map_err(|e| Error::Io(format!("write {LOCK}"), e))
}

fn models_of(lines: &[LockLine]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for l in lines {
        if !out.contains(&l.model.as_str()) {
            out.push(&l.model);
        }
    }
    out
}

fn size_report(r: &mut Report, lines: &[LockLine]) {
    let bytes: u64 = lines.iter().map(|l| l.size).sum();
    let (size, warn) = size_notes(bytes, lines.iter().any(|l| l.kind == "llm"));
    r.lines.push(size);
    r.lines
        .extend(warn.into_iter().map(|w| format!("warning: {w}")));
}

/// `kitchen models list`: the catalog, with the ones in this app's lock marked.
pub fn list(root: &Path) -> Result<Report> {
    let lines = read(root)?;
    let inside = models_of(&lines);
    let mut r = Report::new(format!(
        "{} of {} models bundled ({LOCK})",
        inside.len(),
        CATALOG.len()
    ));
    for m in CATALOG {
        let mark = if inside.contains(&m.name) {
            "bundled"
        } else {
            "-"
        };
        r.lines.push(format!(
            "{:<30} {:<8} {:<5} {:>7}  {}",
            m.name,
            mark,
            m.kind,
            mb(m.bytes()),
            m.licence
        ));
    }
    for other in inside.iter().filter(|n| find(n).is_none()) {
        r.lines.push(format!(
            "{other:<30} bundled  (not in the catalog: your own entry)"
        ));
    }
    if !lines.is_empty() {
        size_report(&mut r, &lines);
    }
    r.next
        .push("kitchen models add <name> / remove <name> / fetch / verify".into());
    Ok(r)
}

/// `kitchen models add <name>`: add a catalog model to the lock.
pub fn add(root: &Path, mode: Option<&str>, name: &str, opts: &Options) -> Result<Report> {
    let m = find(name).ok_or_else(|| {
        Error::Usage(format!(
            "no model {name:?}; `kitchen models list` lists them"
        ))
    })?;
    let mut lines = read(root)?;
    if lines.iter().any(|l| l.model == name) {
        return Ok(Report::new(format!("{name} is already bundled.")));
    }
    lines.extend(parse_lock(&m.lock_lines())?);
    let mut r = Report::new(format!(
        "{} {name} ({}, {}) to {LOCK}",
        if opts.dry_run { "Would add" } else { "Added" },
        mb(m.bytes()),
        m.licence
    ));
    size_report(&mut r, &lines);
    if !opts.dry_run {
        write(root, &lines)?;
        r.next
            .push("kitchen models fetch   (download it and check its checksums)".into());
    }
    if !matches!(mode, Some("bundle" | "bundle_with_fallback")) {
        r.next.push("this app still downloads its models: `kitchen set models=bundle_with_fallback` makes it use the bundled ones".into());
    }
    Ok(r)
}

/// `kitchen models remove <name>`: drop a model from the lock and delete its fetched files.
pub fn remove(root: &Path, name: &str, opts: &Options) -> Result<Report> {
    let mut lines = read(root)?;
    let before = lines.len();
    lines.retain(|l| l.model != name);
    if lines.len() == before {
        return Err(Error::Usage(format!("{name} is not in {LOCK}")));
    }
    let r = Report::new(format!(
        "{} {name} from {LOCK}",
        if opts.dry_run {
            "Would remove"
        } else {
            "Removed"
        }
    ));
    if !opts.dry_run {
        write(root, &lines)?;
        let dir = lock_path(root).with_file_name(name);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| Error::Io(format!("delete {}", dir.display()), e))?;
        }
    }
    Ok(r)
}

/// `kitchen models fetch` / `verify`: the app's own fetcher (`desktop-core/examples/models.rs`,
/// the same code the app runs), so checksums are checked one way everywhere.
pub fn plan(root: &Path, action: &str) -> Result<Plan> {
    if !matches!(action, "fetch" | "verify") {
        return Err(Error::Usage(format!(
            "kitchen models {action}: use list, add, remove, fetch or verify"
        )));
    }
    if !lock_path(root).is_file() {
        return Err(Error::Usage(format!("no {LOCK}: this app bundles no models (`kitchen set models=bundle_with_fallback`, then `kitchen models add <name>`)")));
    }
    let lines = read(root)?;
    let mut notes = Vec::new();
    let bytes: u64 = lines.iter().map(|l| l.size).sum();
    let (size, warn) = size_notes(bytes, lines.iter().any(|l| l.kind == "llm"));
    notes.push(size);
    notes.extend(warn);
    Ok(Plan {
        steps: vec![Step::new(
            &format!("{action} bundled models"),
            root,
            "cargo",
            &[
                "run",
                "-q",
                "-p",
                "desktop-core",
                "--example",
                "models",
                "--",
                action,
            ],
        )],
        notes,
    })
}

/// The `kitchen doctor` check: every file of the lock fetched, with the size it should have.
/// `None` when the app bundles no models.
pub fn doctor_check(root: &Path) -> Option<Check> {
    let lines = match read(root) {
        Ok(l) if l.is_empty() => return None,
        Ok(l) => l,
        Err(e) => {
            return Some(check(
                Status::Fail,
                e.to_string(),
                "fix the lock, or restore it from git",
            ))
        }
    };
    let dir = lock_path(root).with_file_name("");
    let mut missing = Vec::new();
    let mut wrong = Vec::new();
    for l in &lines {
        match std::fs::metadata(dir.join(&l.model).join(&l.path)) {
            Err(_) if !missing.contains(&l.model) => missing.push(l.model.clone()),
            Err(_) => {}
            Ok(m) if m.len() != l.size => wrong.push(format!(
                "{}/{} ({} bytes, the lock says {})",
                l.model,
                l.path,
                m.len(),
                l.size
            )),
            Ok(_) => {}
        }
        if let Some(c) = find(&l.model) {
            if !c.files.iter().any(|f| f.0 == l.path && f.2 == l.sha256) {
                wrong.push(format!(
                    "{}/{} (its checksum differs from the catalog's)",
                    l.model, l.path
                ));
            }
        }
    }
    let bytes: u64 = lines.iter().map(|l| l.size).sum();
    Some(if !wrong.is_empty() {
        check(
            Status::Fail,
            format!("mismatched: {}", wrong.join(", ")),
            "kitchen models fetch   (fetches them again and checks the checksums)",
        )
    } else if !missing.is_empty() {
        check(
            Status::Warn,
            format!("not fetched yet: {}", missing.join(", ")),
            "kitchen models fetch   (or make models) before packaging",
        )
    } else {
        check(
            Status::Ok,
            format!(
                "{} model(s), {}, all fetched (`kitchen models verify` checks the checksums)",
                models_of(&lines).len(),
                mb(bytes)
            ),
            "",
        )
    })
}

fn check(status: Status, detail: String, fix: &str) -> Check {
    Check {
        name: "Bundled models".into(),
        status,
        detail,
        fix: fix.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed() {
        for m in CATALOG {
            assert!(
                matches!(m.kind, "embed" | "stt" | "tts" | "llm"),
                "{}",
                m.name
            );
            for (path, size, sha, url) in m.files {
                assert!(
                    *size > 0 && sha.len() == 64 && url.starts_with("https://huggingface.co/"),
                    "{path}"
                );
                assert!(!path.contains("..") && !path.starts_with('/'), "{path}");
            }
            let parsed = parse_lock(&m.lock_lines()).unwrap();
            assert_eq!(parsed.len(), m.files.len());
        }
        // One model per answer (and per embedding model choice).
        let mut keys: Vec<_> = CATALOG
            .iter()
            .filter(|m| !m.answer.is_empty())
            .map(|m| (m.answer, m.when))
            .collect();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }

    #[test]
    fn answers_choose_models() {
        let get = |models: &'static str| {
            move |k: &str| match k {
                "models" => Some(models.to_string()),
                "bundle_embedding_model" | "bundle_llm" => Some("true".into()),
                "embedding_model" => Some("bge-small-en-v1.5".into()),
                _ => None,
            }
        };
        let names = |v: Vec<&CatalogModel>| v.iter().map(|m| m.name).collect::<Vec<_>>();
        assert_eq!(
            names(chosen(get("bundle"))),
            ["embed-bge-small-en-v1.5", "qwen3-0.6b-q4"]
        );
        assert!(chosen(get("download")).is_empty());
        let (size, warn) = size_notes(total(&chosen(get("bundle"))), true);
        assert!(size.contains("531 MB"), "{size}");
        assert_eq!(warn.len(), 2);
        assert!(warn[0].contains("200 MB") && warn[1].contains("2 GB"));
        assert!(size_notes(1000, false).1.is_empty());
    }

    #[test]
    fn add_remove_and_doctor() {
        let dir = crate::TempDir::new("models").unwrap();
        let root = dir.path();
        assert!(doctor_check(root).is_none());
        assert!(plan(root, "fetch").is_err());
        let opts = Options::default();
        add(root, Some("bundle"), "tiny-voca", &opts).unwrap();
        assert!(add(root, None, "nope", &opts).is_err());
        assert!(add(root, None, "whisper-tiny", &opts)
            .unwrap()
            .next
            .iter()
            .any(|n| n.contains("kitchen set models")));
        assert_eq!(
            models_of(&read(root).unwrap()),
            ["tiny-voca", "whisper-tiny"]
        );
        let c = doctor_check(root).unwrap();
        assert_eq!(c.status, Status::Warn);
        assert!(
            c.detail
                .ends_with("not fetched yet: tiny-voca, whisper-tiny"),
            "{}",
            c.detail
        );
        // A file of the wrong size is a failure.
        let d = root.join("src-tauri/resources/models/tiny-voca");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("model.onnx"), b"x").unwrap();
        assert_eq!(doctor_check(root).unwrap().status, Status::Fail);
        remove(root, "tiny-voca", &opts).unwrap();
        assert!(!d.exists());
        assert!(remove(root, "tiny-voca", &opts).is_err());
        let p = plan(root, "verify").unwrap();
        assert_eq!(p.steps[0].args.last().map(String::as_str), Some("verify"));
        assert!(list(root)
            .unwrap()
            .lines
            .iter()
            .any(|l| l.starts_with("whisper-tiny") && l.contains("bundled")));
    }

    #[test]
    fn liquid_lock_names_every_answer() {
        let t = lock_liquid();
        for m in CATALOG.iter().filter(|m| !m.answer.is_empty()) {
            assert!(t.contains(&format!("{{% if {} %}}", m.answer)));
        }
        assert!(!t.contains("whisper-tiny"));
    }
}
