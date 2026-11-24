//! A local REST API over the app's backend, so scripts, agents and a browser UI can drive
//! the app over HTTP (feature `http_api`, off by default).
//!
//! The design follows [ndelvalle/rustapi](https://github.com/ndelvalle/rustapi) (MIT): an
//! [Axum](https://docs.rs/axum) router over one shared [`Context`], one uniform JSON error
//! shape ([`errors`]), validated JSON bodies that answer with per-field errors
//! ([`extract`]), pagination helpers ([`pagination`]), and a request ID on every log line,
//! a request timeout, a body-size limit and a graceful shutdown ([`middleware`]). The
//! routes ([`routes`]) are thin wrappers: each turns its request into a backend method call
//! (`search`, `config`, `configure`, ...) through [`Dispatch`], the same calls the window
//! makes, so the API needs no logic of its own. This crate's `lib.rs` is shared by the app
//! and by the tests, which start the API on a random port.
//!
//! Safe by default:
//! - it binds to `127.0.0.1` (configurable), on [`HttpApiConfig::port`] (0 picks a free one);
//! - every request needs `Authorization: Bearer <token>`; the token is made on first run and
//!   kept owner-only (0600) in the data folder next to the app's keys ([`TokenFile`]), and a
//!   production build refuses to start without one;
//! - no CORS origins are allowed until you list them.
//!
//! Errors always look like this (`fields` only for validation errors):
//!
//! ```text
//! {"error": {"code": "validation", "message": "check the highlighted fields",
//!            "requestId": "4f1c2a9e0b7d3e61", "fields": {"query": ["must not be empty"]}}}
//! ```
//!
//! # Example
//!
//! Serve a backend (here a closure) on a random port, call it, and stop:
//!
//! ```
//! use desktop_core::http_api::{HttpApiConfig, Server, ServerOptions};
//! use serde_json::{json, Value};
//! use std::sync::Arc;
//!
//! let backend = Arc::new(|method: &str, _params: Value| match method {
//!     "status" => Ok(json!({"chunks": 0})),
//!     other => Err(format!("unknown method {other:?}")),
//! });
//! let config = HttpApiConfig { enabled: true, ..HttpApiConfig::default() }; // port 0: any free port
//! let server = Server::start(backend, ServerOptions::new(config, "secret-token-0123456789"))?;
//! assert!(server.url().starts_with("http://127.0.0.1:"));
//!
//! let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).proxy(None).build().into();
//! let reply = agent.get(format!("{}/health", server.url())).call()?;
//! assert_eq!(reply.status(), 401); // no token
//! let mut reply = agent.get(format!("{}/health", server.url())).header("Authorization", "Bearer secret-token-0123456789").call()?;
//! assert_eq!(reply.status(), 200);
//! let body: Value = serde_json::from_str(&reply.body_mut().read_to_string()?)?;
//! assert_eq!(body["status"], "ok");
//! server.shutdown();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod errors;
pub mod extract;
pub mod middleware;
pub mod pagination;
pub mod routes;

pub use errors::ApiError;
pub use extract::{FieldErrors, Validate, ValidJson, ValidQuery};
pub use pagination::{Page, Paginated};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// HTTP API settings (`httpApi` in settings). Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpApiConfig {
    /// Serve the API (off by default).
    pub enabled: bool,
    /// The address to listen on: `127.0.0.1` (this computer only) unless you change it.
    pub host: String,
    /// The port; 0 picks a free one at each start.
    pub port: u16,
    /// Browser origins allowed to call the API (`https://example.com`); empty = none.
    pub cors_origins: Vec<String>,
    /// The largest request body, in KB.
    pub max_body_kb: u64,
    /// A request that takes longer is answered with a timeout error.
    pub timeout_secs: u64,
}

impl Default for HttpApiConfig {
    fn default() -> Self {
        Self { enabled: false, host: "127.0.0.1".into(), port: 0, cors_origins: Vec::new(), max_body_kb: 1024, timeout_secs: 30 }
    }
}

impl HttpApiConfig {
    /// The same settings with a valid host, sizes in range and only well-formed origins.
    pub fn validated(mut self) -> Self {
        if self.host.trim().parse::<IpAddr>().is_err() {
            self.host = Self::default().host;
        }
        self.host = self.host.trim().to_string();
        self.max_body_kb = self.max_body_kb.clamp(1, 64 * 1024);
        self.timeout_secs = self.timeout_secs.clamp(1, 3600);
        self.cors_origins = self.cors_origins.iter().map(|o| o.trim().trim_end_matches('/').to_string()).filter(|o| valid_origin(o)).collect();
        self
    }

    /// Where the server listens.
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.host.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)), self.port)
    }
}

/// `http(s)://host[:port]` with nothing after it (no `*`, no path).
fn valid_origin(o: &str) -> bool {
    let rest = o.strip_prefix("https://").or_else(|| o.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']')))
}

/// Why the API could not start.
#[derive(Debug, thiserror::Error)]
pub enum HttpApiError {
    /// A production build never serves the API without a token.
    #[error("the HTTP API needs a token in a production build")]
    NoToken,
    /// Binding the port or starting the runtime failed.
    #[error("HTTP API: {0}")]
    Io(#[from] std::io::Error),
}

/// The backend the routes call: a method name and JSON params in, JSON out (the app's
/// `service::Service`). Any `Fn(&str, Value) -> Result<Value, String>` is one.
pub trait Dispatch: Send + Sync + 'static {
    /// Run `method` with `params`; the error text is shown to the caller.
    fn call(&self, method: &str, params: Value) -> Result<Value, String>;
}

impl<F> Dispatch for F
where
    F: Fn(&str, Value) -> Result<Value, String> + Send + Sync + 'static,
{
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self(method, params)
    }
}

/// What every handler shares: the backend, the token, the settings and the feature list.
#[derive(Clone)]
pub struct Context {
    backend: Arc<dyn Dispatch>,
    token: Arc<str>,
    config: Arc<HttpApiConfig>,
    features: Arc<Value>,
}

impl Context {
    /// A context over `backend`, guarded by `token` (empty = no auth, debug builds only).
    pub fn new(backend: Arc<dyn Dispatch>, token: &str, config: HttpApiConfig, features: Value) -> Self {
        Self { backend, token: token.into(), config: Arc::new(config.validated()), features: Arc::new(features) }
    }

    /// Run a backend method off the async threads; its error becomes an [`ApiError`].
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        let backend = self.backend.clone();
        let method = method.to_string();
        tokio::task::spawn_blocking(move || backend.call(&method, params))
            .await
            .map_err(|e| ApiError::internal(format!("the backend call stopped: {e}")))?
            .map_err(ApiError::backend)
    }

    /// The API's settings.
    pub fn config(&self) -> &HttpApiConfig {
        &self.config
    }

    /// The bearer token requests must carry.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The parts built into this app, as `GET /v1/features` returns them.
    pub fn features(&self) -> &Value {
        &self.features
    }
}

/// The parts built into `desktop-core` (the app passes its own, fuller list).
pub fn core_features() -> Value {
    serde_json::json!({
        "httpApi": true,
        "embed": cfg!(feature = "embed"),
        "postgres": cfg!(feature = "postgres"),
        "pdf": cfg!(feature = "pdf"),
        "ai": cfg!(feature = "ai"),
    })
}

/// How [`Server::start`] runs.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Host, port, CORS, limits.
    pub config: HttpApiConfig,
    /// The bearer token (see [`TokenFile`]).
    pub token: String,
    /// What `GET /v1/features` returns.
    pub features: Value,
    /// Refuse to start without a token (true in release builds).
    pub production: bool,
}

impl ServerOptions {
    /// Options with `config` and `token`, this crate's feature list, and `production` set
    /// in release builds.
    pub fn new(config: HttpApiConfig, token: impl Into<String>) -> Self {
        Self { config, token: token.into(), features: core_features(), production: !cfg!(debug_assertions) }
    }
}

/// A running API. Dropping it (or [`Server::shutdown`]) stops it gracefully: requests in
/// flight finish, new ones are refused.
pub struct Server {
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Bind and serve on a thread of its own. The port is bound before this returns, so
    /// [`Server::addr`] is final and a taken port is an error here.
    pub fn start(backend: Arc<dyn Dispatch>, opts: ServerOptions) -> Result<Self, HttpApiError> {
        if opts.token.is_empty() {
            if opts.production {
                return Err(HttpApiError::NoToken);
            }
            log::warn!("HTTP API: no token, so requests are not checked (debug builds only)");
        }
        let config = opts.config.validated();
        let listener = std::net::TcpListener::bind(config.addr())?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
        let app = routes::router(Context::new(backend, &opts.token, config, opts.features));
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new().name("http-api".into()).spawn(move || {
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(e) => return log::error!("HTTP API: {e}"),
                };
                let shutdown = async move {
                    let _ = stopped.await;
                };
                if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(shutdown).await {
                    log::error!("HTTP API stopped: {e}");
                }
            });
        })?;
        log::info!("HTTP API listening on http://{addr}");
        Ok(Self { addr, stop: Some(stop), thread: Some(thread) })
    }

    /// The address it listens on (with the real port when 0 was asked for).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://<host>:<port>`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Stop gracefully and wait for the server thread.
    pub fn shutdown(mut self) {
        self.stop_now();
    }

    fn stop_now(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
            log::info!("HTTP API on {} stopped", self.addr);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop_now();
    }
}

/// The API token file in the data folder (`api-token`, owner-only), next to the app's
/// other keys.
pub struct TokenFile(PathBuf);

impl TokenFile {
    /// The token file in `data_dir` (it need not exist yet).
    pub fn new(data_dir: &Path) -> Self {
        Self(data_dir.join("api-token"))
    }

    /// Where it is.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The saved token, or a new random one (256 bits, hex) saved with mode 0600.
    pub fn load_or_create(&self) -> std::io::Result<String> {
        if let Ok(t) = std::fs::read_to_string(&self.0) {
            let t = t.trim();
            if t.len() >= 32 {
                return Ok(t.to_string());
            }
        }
        let token = random_hex(32)?;
        crate::jobs::write_private(&self.0, token.as_bytes())?;
        Ok(token)
    }
}

/// `n` random bytes as lower-case hex.
pub(crate) fn random_hex(n: usize) -> std::io::Result<String> {
    let mut b = vec![0u8; n];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).map_err(|_| std::io::Error::other("no secure random numbers"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_local_and_closed() {
        let c = HttpApiConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(c.cors_origins.is_empty());
    }

    #[test]
    fn validation_fixes_bad_values() {
        let c = HttpApiConfig {
            host: "not an ip".into(),
            max_body_kb: 0,
            timeout_secs: 99_999,
            cors_origins: vec!["*".into(), "https://app.example.com/".into(), "ftp://x".into(), "http://localhost:5173".into()],
            ..HttpApiConfig::default()
        }
        .validated();
        assert_eq!(c.host, "127.0.0.1");
        assert_eq!(c.max_body_kb, 1);
        assert_eq!(c.timeout_secs, 3600);
        assert_eq!(c.cors_origins, ["https://app.example.com", "http://localhost:5173"]);
    }

    #[test]
    fn token_is_made_once_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let f = TokenFile::new(dir.path());
        let a = f.load_or_create().unwrap();
        assert_eq!(a.len(), 64);
        assert_eq!(f.load_or_create().unwrap(), a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(f.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn production_refuses_to_start_without_a_token() {
        let backend = Arc::new(|_: &str, _: Value| Ok(Value::Null));
        let mut opts = ServerOptions::new(HttpApiConfig::default(), "");
        opts.production = true;
        assert!(matches!(Server::start(backend, opts), Err(HttpApiError::NoToken)));
    }
}
