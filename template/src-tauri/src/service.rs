//! The backend as requests and replies, so it can run in this process or in a separate
//! worker process (feature `ipc-worker`) without the commands knowing which.
//!
//! Methods: `ping`, `setup` (EngineOptions), `status`, `ingest_text` {source, text},
//! `ingest_file` {path}, `ingest_folder` {path, recursive}, `search` {query, k, search?},
//! `remove_source` {source}, `clear`, `configure` {config}, `watch_status`, and with
//! feature `ai`: `ask` {question} (the answer streams as progress lines), `ai_keys`,
//! `set_ai_key` {provider, key}.
//!
//! Folder watching (feature `watch`) starts with `setup` or `configure` when
//! `config.watch.enabled`; `watch_status` returns the folders and recent events, which
//! the UI polls (the same in this process and in the worker).

use desktop_core::{Config, Engine, EngineOptions, SearchConfig};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub trait Service: Send + Sync {
    /// Run `method`; `progress` gets human-readable lines while it works.
    fn call(&self, method: &str, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, String>;
}

/// The backend in this process.
#[derive(Default)]
pub struct Local {
    engine: Mutex<Option<Arc<Engine>>>,
    app_name: Mutex<Option<String>>,
    #[cfg(feature = "watch")]
    watcher: Mutex<Option<desktop_core::watch::Watcher>>,
    /// Recent watch events, newest last (at most 100).
    events: Arc<Mutex<std::collections::VecDeque<Value>>>,
}

fn arg<T: DeserializeOwned>(params: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(params.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{key}: {e}"))
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn to<T: serde::Serialize>(v: Result<T, desktop_core::Error>) -> Result<Value, String> {
    v.map_err(err).and_then(|v| serde_json::to_value(v).map_err(err))
}

impl Local {
    fn engine(&self) -> Result<Arc<Engine>, String> {
        self.engine.lock().map_err(err)?.clone().ok_or_else(|| "Still starting; try again in a moment.".into())
    }

    /// Start, restart or stop the folder watcher to match the engine's config.
    #[cfg(feature = "watch")]
    fn rewatch(&self) -> Result<(), String> {
        let engine = self.engine()?;
        let mut slot = self.watcher.lock().map_err(err)?;
        *slot = None;
        let cfg = engine.config().watch;
        if !cfg.enabled {
            return Ok(());
        }
        let name = self.app_name.lock().map_err(err)?.clone().unwrap_or_else(|| "App".into());
        let drop = desktop_core::watch::default_drop_folder(&name, engine.data_dir());
        let events = self.events.clone();
        let data_dir = engine.data_dir().to_path_buf();
        let watcher = desktop_core::watch::start(engine, &cfg, &drop, &data_dir, move |ev| {
            if let (Ok(mut q), Ok(v)) = (events.lock(), serde_json::to_value(&ev)) {
                q.push_back(v);
                while q.len() > 100 {
                    q.pop_front();
                }
            }
        })
        .map_err(err)?;
        *slot = Some(watcher);
        Ok(())
    }

    #[cfg(not(feature = "watch"))]
    fn rewatch(&self) -> Result<(), String> {
        Ok(())
    }

    fn watch_status(&self) -> Result<Value, String> {
        #[cfg(feature = "watch")]
        let folders = self.watcher.lock().map_err(err)?.as_ref().map(|w| w.folders.clone()).unwrap_or_default();
        #[cfg(not(feature = "watch"))]
        let folders: Vec<std::path::PathBuf> = Vec::new();
        let events: Vec<Value> = self.events.lock().map_err(err)?.iter().cloned().collect();
        Ok(json!({"built": cfg!(feature = "watch"), "folders": folders, "events": events}))
    }
}

impl Service for Local {
    fn call(&self, method: &str, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, String> {
        match method {
            "ping" => Ok(json!({"pid": std::process::id(), "version": env!("CARGO_PKG_VERSION")})),
            "setup" => {
                let opts: EngineOptions = serde_json::from_value(params).map_err(err)?;
                let engine = Engine::open(&opts, progress).map_err(err)?;
                let status = engine.status().map_err(err)?;
                *self.engine.lock().map_err(err)? = Some(Arc::new(engine));
                *self.app_name.lock().map_err(err)? = opts.app_name.clone();
                if let Err(e) = self.rewatch() {
                    progress(&format!("Folder watching is off: {e}"));
                }
                serde_json::to_value(status).map_err(err)
            }
            "status" => match self.engine.lock().map_err(err)?.as_ref() {
                Some(e) => to(e.status()),
                None => Ok(Value::Null),
            },
            "ingest_text" => to(self.engine()?.ingest_text(&arg::<String>(&params, "source")?, &arg::<String>(&params, "text")?)),
            "ingest_file" => to(self.engine()?.ingest_file(Path::new(&arg::<String>(&params, "path")?))),
            "ingest_folder" => {
                let e = self.engine()?;
                let recursive = arg::<Option<bool>>(&params, "recursive")?.unwrap_or(true);
                let files = e.files_in(Path::new(&arg::<String>(&params, "path")?), recursive);
                progress(&format!("Ingesting {} files with {} workers", files.len(), e.config().workers()));
                to(Ok(e.ingest_paths(&files, &|_, _, _| {})))
            }
            "search" => {
                let e = self.engine()?;
                let query = arg::<String>(&params, "query")?;
                let mut cfg = arg::<Option<SearchConfig>>(&params, "search")?.unwrap_or_else(|| e.config().search);
                if let Some(k) = arg::<Option<usize>>(&params, "k")? {
                    cfg.top_k = k;
                }
                to(e.search_with(&query, &cfg))
            }
            "configure" => {
                let e = self.engine()?;
                e.configure(arg::<Config>(&params, "config")?).map_err(err)?;
                self.rewatch()?;
                to(Ok(e.config()))
            }
            "config" => to(Ok(self.engine()?.config())),
            "watch_status" => self.watch_status(),
            "remove_source" => to(self.engine()?.remove_source(&arg::<String>(&params, "source")?)),
            "clear" => {
                let e = self.engine()?;
                e.clear().map_err(err)?;
                to(e.status())
            }
            #[cfg(feature = "ai")]
            "ask" => {
                let e = self.engine()?;
                let keys = desktop_core::llm::KeyFile::new(e.data_dir().join("keys.json"));
                let lookup = |p| keys.get(p);
                to(e.ask(&arg::<String>(&params, "question")?, &lookup, &mut |t| progress(t)))
            }
            #[cfg(feature = "ai")]
            "ai_keys" => to(Ok(desktop_core::llm::KeyFile::new(self.engine()?.data_dir().join("keys.json")).present())),
            #[cfg(feature = "ai")]
            "set_ai_key" => {
                let keys = desktop_core::llm::KeyFile::new(self.engine()?.data_dir().join("keys.json"));
                keys.set(arg(&params, "provider")?, arg::<Option<String>>(&params, "key")?.as_deref()).map_err(err)?;
                to(Ok(keys.present()))
            }
            other => Err(format!("unknown method {other:?}")),
        }
    }
}
