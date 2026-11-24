//! The local REST API (feature `http-api`): started after setup when `httpApi.enabled`,
//! restarted when its settings change, stopped on exit. It serves the same backend as the
//! window (`service::Service`, in this process or the worker), with the token from
//! `<data>/api-token` (made on first use, owner-only). See `desktop_core::http_api`.
//!
//! # Example
//!
//! What Settings → HTTP API shows (`http_api_info`):
//!
//! ```text
//! {"enabled": true, "running": true, "url": "http://127.0.0.1:51234", "token": "9f2c...", "error": null}
//! ```

use crate::service::Service;
use desktop_core::http_api::{HttpApiConfig, Server, ServerOptions, TokenFile};
use desktop_core::Config;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// The running server, the settings it runs with, and the last start error.
struct Running {
    server: Option<Server>,
    config: Option<HttpApiConfig>,
    error: Option<String>,
}

static API: Mutex<Running> = Mutex::new(Running { server: None, config: None, error: None });

/// Start, restart or stop the API to match `config` (nothing happens when it already matches).
pub fn apply(service: Arc<dyn Service>, config: &HttpApiConfig) {
    let mut api = API.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if api.config.as_ref() == Some(config) && (api.server.is_some() || !config.enabled) {
        return;
    }
    if let Some(old) = api.server.take() {
        old.shutdown();
    }
    api.config = Some(config.clone());
    api.error = None;
    if !config.enabled {
        return;
    }
    match start(service, config) {
        Ok(server) => api.server = Some(server),
        Err(e) => {
            log::warn!("HTTP API not started: {e}");
            api.error = Some(e);
        }
    }
}

fn start(service: Arc<dyn Service>, config: &HttpApiConfig) -> Result<Server, String> {
    let token = TokenFile::new(&crate::paths::app_dirs().1).load_or_create().map_err(|e| e.to_string())?;
    let backend = move |method: &str, params: Value| {
        let reply = service.call(method, params, &mut |_| {})?;
        // Settings changed over HTTP are saved, as when Settings saves them.
        if method == "configure" {
            let config: Config = serde_json::from_value(reply.clone()).map_err(|e| e.to_string())?;
            crate::save_config(config)?;
        }
        Ok(reply)
    };
    let mut opts = ServerOptions::new(config.clone(), token);
    opts.features = crate::features();
    Server::start(Arc::new(backend), opts).map_err(|e| e.to_string())
}

/// Stop the API (on exit).
pub fn stop() {
    let mut api = API.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(server) = api.server.take() {
        server.shutdown();
    }
}

/// The URL and token for Settings → HTTP API.
#[tauri::command]
pub fn http_api_info() -> Value {
    let api = API.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let token = TokenFile::new(&crate::paths::app_dirs().1).load_or_create().ok();
    json!({
        "enabled": api.config.as_ref().is_some_and(|c| c.enabled),
        "running": api.server.is_some(),
        "url": api.server.as_ref().map(Server::url),
        "token": token,
        "error": api.error,
    })
}
