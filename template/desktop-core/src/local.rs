//! Local LLM inference with llama.cpp built in (feature `llama`): GGUF models, no server,
//! nothing leaves the machine. llama.cpp is compiled from source and linked statically
//! (Metal on Apple Silicon, CPU elsewhere), so the app ships as one binary.
//!
//! The model stays loaded between questions (`LocalConfig::keep_loaded`) and is unloaded
//! when another file is picked.

use crate::llm::{LocalConfig, Message, Reply, Role};
use crate::{Error, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn llm_err(e: impl std::fmt::Display) -> Error {
    Error::Llm(format!("local model: {e}"))
}

/// llama.cpp may be initialised once per process.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<std::result::Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            LlamaBackend::init().map(|mut b| {
                b.void_logs();
                b
            })
            .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(llm_err)
}

struct Loaded {
    path: PathBuf,
    gpu_layers: u32,
    model: LlamaModel,
}

fn cache() -> &'static Mutex<Option<Loaded>> {
    static CACHE: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Free the loaded model (it loads again on the next question).
pub fn unload() {
    if let Ok(mut c) = cache().lock() {
        *c = None;
    }
}

fn role(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Run a chat on the GGUF model at `cfg.model_path`; `on_token` gets text as it is made.
pub fn chat(cfg: &LocalConfig, messages: &[Message], max_tokens: u32, temperature: f32, on_token: &mut dyn FnMut(&str)) -> Result<Reply> {
    let path = cfg.model_path.as_deref().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| llm_err("pick a .gguf model file in Settings → AI"))?;
    if !path.is_file() {
        return Err(llm_err(format!("{} is missing", path.display())));
    }
    let backend = backend()?;
    let mut guard = cache().lock().map_err(llm_err)?;
    if !guard.as_ref().is_some_and(|l| l.path == path && l.gpu_layers == cfg.gpu_layers) {
        *guard = None; // free the old one first: two models may not fit in memory
        let params = LlamaModelParams::default().with_n_gpu_layers(cfg.gpu_layers);
        let model = LlamaModel::load_from_file(backend, path, &params).map_err(llm_err)?;
        *guard = Some(Loaded { path: path.to_path_buf(), gpu_layers: cfg.gpu_layers, model });
    }
    let model = &guard.as_ref().ok_or_else(|| llm_err("not loaded"))?.model;
    let reply = generate(backend, model, cfg, messages, max_tokens, temperature, on_token);
    if !cfg.keep_loaded {
        *guard = None;
    }
    let mut reply = reply?;
    reply.model = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(reply)
}

fn prompt(model: &LlamaModel, messages: &[Message]) -> Result<String> {
    let chat: Vec<LlamaChatMessage> = messages.iter().map(|m| LlamaChatMessage::new(role(m.role).into(), m.content.clone())).collect::<std::result::Result<_, _>>().map_err(llm_err)?;
    // The model's own template, else ChatML (what most instruction models expect).
    let template = match model.chat_template(None) {
        Ok(t) => t,
        Err(_) => LlamaChatTemplate::new("chatml").map_err(llm_err)?,
    };
    model.apply_chat_template(&template, &chat, true).map_err(llm_err)
}

fn generate(backend: &LlamaBackend, model: &LlamaModel, cfg: &LocalConfig, messages: &[Message], max_tokens: u32, temperature: f32, on_token: &mut dyn FnMut(&str)) -> Result<Reply> {
    let text = prompt(model, messages)?;
    let vocab = model.vocab();
    let tokens = vocab.tokenize(text.as_bytes(), true, true);
    let n_ctx = cfg.context.clamp(512, 131_072);
    if tokens.len() as u32 + max_tokens > n_ctx {
        return Err(llm_err(format!("the question and sources ({} tokens) do not fit the context ({n_ctx}); lower the result count or raise the context", tokens.len())));
    }
    let threads = if cfg.threads == 0 { std::thread::available_parallelism().map_or(4, |n| n.get()) as i32 } else { cfg.threads as i32 };
    let params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(n_ctx)).with_n_batch(n_ctx.min(2048)).with_n_threads(threads).with_n_threads_batch(threads);
    let mut ctx = model.new_context(backend, params).map_err(llm_err)?;

    let mut batch = LlamaBatch::new(n_ctx.min(2048) as usize, 1);
    let last = tokens.len().saturating_sub(1);
    for chunk_start in (0..tokens.len()).step_by(n_ctx.min(2048) as usize) {
        batch.clear();
        for (i, t) in tokens.iter().enumerate().skip(chunk_start).take(n_ctx.min(2048) as usize) {
            batch.add(*t, i as i32, &[0], i == last).map_err(llm_err)?;
        }
        ctx.decode(&mut batch).map_err(llm_err)?;
    }

    let mut sampler = if temperature <= 0.0 {
        LlamaSampler::greedy()
    } else {
        LlamaSampler::chain_simple([LlamaSampler::top_k(40), LlamaSampler::top_p(0.95, 1), LlamaSampler::temp(temperature), LlamaSampler::dist(cfg.seed)])
    };
    let mut reply = Reply { input_tokens: tokens.len() as u64, ..Reply::default() };
    let mut pending: Vec<u8> = Vec::new();
    for pos in (tokens.len() as i32..).take(max_tokens as usize) {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if vocab.is_eog(token) {
            break;
        }
        reply.output_tokens += 1;
        // A UTF-8 character can span tokens: emit only whole characters.
        pending.extend(vocab.token_to_piece(token, false, None));
        let valid = match std::str::from_utf8(&pending) {
            Ok(s) => s.len(),
            Err(e) => e.valid_up_to(),
        };
        if valid > 0 {
            let s = String::from_utf8_lossy(&pending[..valid]).into_owned();
            on_token(&s);
            reply.text.push_str(&s);
            pending.drain(..valid);
        }
        batch.clear();
        batch.add(token, pos, &[0], true).map_err(llm_err)?;
        ctx.decode(&mut batch).map_err(llm_err)?;
    }
    if !pending.is_empty() {
        let s = String::from_utf8_lossy(&pending).into_owned();
        on_token(&s);
        reply.text.push_str(&s);
    }
    Ok(reply)
}

/// A file name for a model URL inside `dir` (for downloads).
pub fn model_file(dir: &Path, url: &str) -> PathBuf {
    let name = url.rsplit('/').next().and_then(|n| n.split('?').next()).filter(|n| !n.is_empty()).unwrap_or("model.gguf");
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
    dir.join(safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_file_names_are_safe() {
        let d = Path::new("/m");
        assert_eq!(model_file(d, "https://h/x/Qwen3-0.6B-Q4_K_M.gguf?download=1"), d.join("Qwen3-0.6B-Q4_K_M.gguf"));
        assert_eq!(model_file(d, "https://h/../../evil"), d.join("evil"));
        assert_eq!(model_file(d, "https://h/a b.gguf"), d.join("a_b.gguf"));
    }

    /// Runs when `APP_TEST_GGUF` names a small model (CI uses stories260K, about 1 MB).
    #[test]
    fn generates_with_a_real_model() {
        let Some(path) = std::env::var_os("APP_TEST_GGUF").map(PathBuf::from) else {
            eprintln!("skipped: set APP_TEST_GGUF to a .gguf file");
            return;
        };
        let cfg = LocalConfig { model_path: Some(path), context: 512, ..LocalConfig::default() };
        let mut streamed = String::new();
        let msgs = [Message::new(Role::User, "Once upon a time")];
        let r = chat(&cfg, &msgs, 16, 0.0, &mut |t| streamed.push_str(t)).unwrap();
        assert!(r.output_tokens > 0, "{r:?}");
        assert_eq!(streamed, r.text);
        assert!(r.input_tokens > 0);
    }
}
