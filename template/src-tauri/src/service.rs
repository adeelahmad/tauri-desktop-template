//! The backend as requests and replies, so it can run in this process or in a separate
//! worker process (feature `ipc-worker`) without the commands knowing which.
//!
//! Methods: `ping`, `setup` (EngineOptions), `status`, `ingest_text` {source, text},
//! `ingest_file` {path}, `search` {query, k}, `remove_source` {source}, `clear`.

use desktop_core::{Engine, EngineOptions};
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
                serde_json::to_value(status).map_err(err)
            }
            "status" => match self.engine.lock().map_err(err)?.as_ref() {
                Some(e) => to(e.status()),
                None => Ok(Value::Null),
            },
            "ingest_text" => to(self.engine()?.ingest_text(&arg::<String>(&params, "source")?, &arg::<String>(&params, "text")?)),
            "ingest_file" => to(self.engine()?.ingest_file(Path::new(&arg::<String>(&params, "path")?))),
            "search" => {
                let k = arg::<Option<usize>>(&params, "k")?.unwrap_or(10);
                to(self.engine()?.search(&arg::<String>(&params, "query")?, k))
            }
            "remove_source" => to(self.engine()?.remove_source(&arg::<String>(&params, "source")?)),
            "clear" => {
                let e = self.engine()?;
                e.clear().map_err(err)?;
                to(e.status())
            }
            other => Err(format!("unknown method {other:?}")),
        }
    }
}
