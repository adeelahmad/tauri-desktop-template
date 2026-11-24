//! The HTTP API end to end: each test starts its own server on a random port over an
//! engine in its own temporary folder, so the tests run in parallel.

#![cfg(feature = "http-api")]

use desktop_core::http_api::{HttpApiConfig, Server, ServerOptions, TokenFile};
use desktop_core::{Config, Engine, FileStore, SearchConfig};
use serde_json::{json, Value};
use std::sync::Arc;

/// A started API, its token, and the folder it keeps its data in.
struct Api {
    server: Server,
    token: String,
    _dir: tempfile::TempDir,
}

/// The backend methods the routes call, over a real engine (the app's `service.rs` does
/// the same and more).
fn engine_backend(engine: Arc<Engine>) -> impl Fn(&str, Value) -> Result<Value, String> + Send + Sync + 'static {
    move |method, p| {
        let e = &engine;
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let to = |v: Result<Value, desktop_core::Error>| v.map_err(|e| e.to_string());
        match method {
            "status" => to(e.status().map(|s| json!(s))),
            "config" => Ok(json!(e.config())),
            "configure" => {
                let c: Config = serde_json::from_value(p["config"].clone()).map_err(|e| e.to_string())?;
                e.configure(c).map_err(|e| e.to_string())?;
                Ok(json!(e.config()))
            }
            "ingest_text" => to(e.ingest_text(&text("source"), &text("text")).map(|n| json!(n))),
            "remove_source" => to(e.remove_source(&text("source")).map(|n| json!(n))),
            "search" => {
                let mut cfg: SearchConfig = e.config().search;
                cfg.top_k = p["k"].as_u64().unwrap_or(10) as usize;
                to(e.search_with(&text("query"), &cfg).map(|h| json!(h)))
            }
            other => Err(format!("unknown method {other:?}")),
        }
    }
}

fn start_with(config: HttpApiConfig, backend: Option<Arc<dyn desktop_core::http_api::Dispatch>>) -> Api {
    let dir = tempfile::tempdir().unwrap();
    let token = TokenFile::new(dir.path()).load_or_create().unwrap();
    let backend = backend.unwrap_or_else(|| {
        let store = FileStore::open(&dir.path().join("store")).unwrap();
        let engine = Engine::with_parts(Box::new(store), None, dir.path(), &mut |_| {}).unwrap();
        Arc::new(engine_backend(Arc::new(engine)))
    });
    let mut opts = ServerOptions::new(HttpApiConfig { port: 0, ..config }, token.clone());
    opts.production = true;
    let server = Server::start(backend, opts).unwrap();
    Api { server, token, _dir: dir }
}

fn start() -> Api {
    start_with(HttpApiConfig::default(), None)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).proxy(None).build().into()
}

impl Api {
    /// `(status, headers, JSON body)` for a request with the token.
    fn send(&self, method: &str, path: &str, body: Option<Value>) -> (u16, ureq::http::HeaderMap, Value) {
        self.send_as(method, path, body, Some(&self.token), &[])
    }

    fn send_as(&self, method: &str, path: &str, body: Option<Value>, token: Option<&str>, headers: &[(&str, &str)]) -> (u16, ureq::http::HeaderMap, Value) {
        let url = format!("{}{path}", self.server.url());
        let mut req = ureq::http::Request::builder().method(method).uri(url);
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = match body {
            Some(b) => agent().run(req.header("Content-Type", "application/json").body(b.to_string()).unwrap()),
            None => agent().run(req.body(()).unwrap()),
        };
        let mut res = res.unwrap();
        let text = res.body_mut().read_to_string().unwrap_or_default();
        (res.status().as_u16(), res.headers().clone(), serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

/// The one error shape: code, message, request ID (the same as the header).
fn assert_error(status: u16, headers: &ureq::http::HeaderMap, body: &Value, want_status: u16, code: &str) {
    assert_eq!(status, want_status, "{body}");
    let e = &body["error"];
    assert_eq!(e["code"], code, "{body}");
    assert!(e["message"].as_str().is_some_and(|m| !m.is_empty()), "{body}");
    assert_eq!(e["requestId"].as_str(), headers.get("x-request-id").and_then(|v| v.to_str().ok()), "{body}");
}

#[test]
fn requests_without_the_right_token_are_refused() {
    let api = start();
    for token in [None, Some("wrong"), Some("")] {
        let (s, h, b) = api.send_as("GET", "/v1/status", None, token, &[]);
        assert_error(s, &h, &b, 401, "unauthorized");
        assert_eq!(h.get("www-authenticate").unwrap(), "Bearer");
    }
    let (s, h, b) = api.send_as("GET", "/health", None, None, &[]);
    assert_error(s, &h, &b, 401, "unauthorized");
    assert_eq!(api.send("GET", "/v1/status", None).0, 200);
}

#[test]
fn every_error_has_the_same_shape() {
    let api = start();
    let (s, h, b) = api.send("GET", "/nope", None);
    assert_error(s, &h, &b, 404, "not_found");
    let (s, h, b) = api.send("DELETE", "/v1/status", None);
    assert_error(s, &h, &b, 405, "method_not_allowed");
    let (s, h, b) = api.send("POST", "/v1/search", Some(json!({"query": "", "perPage": 500})));
    assert_error(s, &h, &b, 422, "validation");
    assert_eq!(b["error"]["fields"]["query"][0], "must not be empty");
    assert!(b["error"]["fields"]["perPage"].is_array());
    let (s, h, b) = api.send("POST", "/v1/search", Some(json!({"perPage": 5})));
    assert_error(s, &h, &b, 422, "invalid_json");
    let (s, h, b) = api.send_as("POST", "/v1/search", None, Some(&api.token), &[("content-type", "application/json")]);
    assert_error(s, &h, &b, 400, "invalid_json");
    let (s, h, b) = api.send("DELETE", "/v1/documents", None);
    assert_error(s, &h, &b, 400, "invalid_query");
    // A client's own request ID is kept.
    let (_, h, b) = api.send_as("GET", "/nope", None, Some(&api.token), &[("x-request-id", "trace-42")]);
    assert_eq!(b["error"]["requestId"], "trace-42");
    assert_eq!(h.get("x-request-id").unwrap(), "trace-42");
}

#[test]
fn health_checks_the_store() {
    let api = start();
    let (s, _, b) = api.send("GET", "/health", None);
    assert_eq!(s, 200);
    assert_eq!(b["status"], "ok");
    assert_eq!(b["store"]["backend"], "file");

    let starting = start_with(HttpApiConfig::default(), Some(Arc::new(|_: &str, _: Value| Err("Still starting; try again in a moment.".to_string()))));
    assert_eq!(starting.send("GET", "/health", None).2["status"], "starting");
    let broken = start_with(HttpApiConfig::default(), Some(Arc::new(|_: &str, _: Value| Err("store: disk full".to_string()))));
    let (s, h, b) = broken.send("GET", "/health", None);
    assert_error(s, &h, &b, 503, "unhealthy");
}

#[test]
fn documents_search_and_settings_go_through_the_backend() {
    let api = start();
    let (s, _, b) = api.send("POST", "/v1/documents", Some(json!({"source": "notes.md", "text": "The heart pumps blood.\n\nRust compiles to native code."})));
    assert_eq!(s, 201, "{b}");
    assert!(b["chunks"].as_u64().unwrap() >= 1);

    let (s, _, b) = api.send("POST", "/v1/search", Some(json!({"query": "heart", "perPage": 1})));
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"][0]["source"], "notes.md");
    assert_eq!(b["page"], 1);
    assert_eq!(b["perPage"], 1);

    // Settings are validated like everywhere else in the app: topK is clamped.
    let (_, _, mut cfg) = api.send("GET", "/v1/settings", None);
    cfg["search"]["topK"] = json!(99_999);
    let (s, _, b) = api.send("PUT", "/v1/settings", Some(cfg));
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["search"]["topK"], 1000);
    assert_eq!(api.send("GET", "/v1/settings", None).2["search"]["topK"], 1000);

    let (s, _, b) = api.send("DELETE", "/v1/documents?source=notes.md", None);
    assert_eq!(s, 200);
    assert!(b["removed"].as_u64().unwrap() >= 1);
    assert_eq!(api.send("GET", "/v1/features", None).2["httpApi"], true);
}

#[test]
fn limits_cors_and_timeouts() {
    let config = HttpApiConfig { max_body_kb: 1, timeout_secs: 1, cors_origins: vec!["http://localhost:5173".into()], ..HttpApiConfig::default() };
    let slow = |m: &str, _: Value| {
        if m == "status" {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        Ok(json!([]))
    };
    let api = start_with(config, Some(Arc::new(slow)));
    let (s, h, b) = api.send("POST", "/v1/search", Some(json!({"query": "x".repeat(2000)})));
    assert_error(s, &h, &b, 413, "payload_too_large");
    let (s, h, b) = api.send("GET", "/v1/status", None);
    assert_error(s, &h, &b, 408, "timeout");

    // Only listed origins get CORS headers; their preflight needs no token.
    let pre = [("origin", "http://localhost:5173"), ("access-control-request-method", "POST")];
    let (s, h, _) = api.send_as("OPTIONS", "/v1/search", None, None, &pre);
    assert_eq!(s, 204);
    assert_eq!(h.get("access-control-allow-origin").unwrap(), "http://localhost:5173");
    let other = [("origin", "https://evil.example"), ("access-control-request-method", "POST")];
    let (s, h, _) = api.send_as("OPTIONS", "/v1/search", None, None, &other);
    assert_eq!(s, 401);
    assert!(h.get("access-control-allow-origin").is_none());
}
{% if background_jobs %}
#[cfg(feature = "jobs-queue")]
#[test]
fn jobs_routes_call_the_queue_methods() {
    let backend = |m: &str, p: Value| Ok(json!({"method": m, "params": p}));
    let api = start_with(HttpApiConfig::default(), Some(Arc::new(backend)));
    assert_eq!(api.send("GET", "/v1/jobs", None).2["method"], "jobs_list");
    let (s, _, b) = api.send("POST", "/v1/jobs", Some(json!({"kind": "digest", "delaySecs": 5})));
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["params"], json!({"kind": "digest", "payload": {}, "delaySecs": 5}));
    let (s, h, b) = api.send("POST", "/v1/jobs", Some(json!({"kind": ""})));
    assert_error(s, &h, &b, 422, "validation");
    let (_, _, b) = api.send("POST", "/v1/jobs/j-1/retry", None);
    assert_eq!(b, json!({"method": "jobs_retry", "params": {"id": "j-1"}}));
    assert_eq!(api.send("POST", "/v1/jobs/j-1/cancel", None).2["method"], "jobs_cancel");
}
{% endif %}
