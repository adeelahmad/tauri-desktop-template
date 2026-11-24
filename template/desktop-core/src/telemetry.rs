//! Opt-in telemetry (feature `telemetry`): crash reports to Sentry, product analytics
//! to PostHog or Aptabase, and traces to any OpenTelemetry collector (OTLP/HTTP JSON).
//! Every part is off until the user switches it on in Settings, and none of it needs an
//! SDK: each service's HTTP API is called with the crate's one HTTP client.
//!
//! What can be sent is fixed here, not by callers' data: method names, durations, counts,
//! ok/error, the app version, OS and architecture, and an anonymous install id (random,
//! kept in `<data>/telemetry-id`, never tied to a person). Document text, queries, file
//! names and error messages are never sent. Crash reports carry the panic message and a
//! backtrace with the home folder replaced by `~`.
//!
//! Crashes are always written to `<data>/crashes/` (newest 20), so a crash can be
//! inspected locally; they are sent at the next start only when crash reports are on.
//! Keys default to the build's environment (`SENTRY_DSN`, `POSTHOG_KEY`, `POSTHOG_HOST`,
//! `APTABASE_KEY`, `OTEL_EXPORTER_OTLP_ENDPOINT`), so they live in CI secrets, not code.
//!
//! Main items: [`init`], [`configure`], [`event`], [`record`], [`span`], [`flush`],
//! [`status`], [`install_panic_hook`] and [`TelemetryConfig`].
//!
//! # Example
//!
//! ```
//! use desktop_core::telemetry::{sentry_endpoint, TelemetryConfig};
//!
//! let (url, key) = sentry_endpoint("https://abc123@o1.ingest.sentry.io/42").unwrap();
//! assert_eq!(url, "https://o1.ingest.sentry.io/api/42/envelope/");
//! assert_eq!(key, "abc123");
//! assert!(!TelemetryConfig::default().any()); // nothing is sent until the user opts in
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Telemetry settings (`telemetry` in settings). Everything is off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TelemetryConfig {
    /// Send crash reports (written locally either way).
    pub crash_reports: bool,
    /// Where crash reports go (a Sentry DSN).
    pub sentry_dsn: String,
    /// Send anonymous usage events.
    pub analytics: bool,
    /// "posthog" or "aptabase".
    pub analytics_provider: String,
    /// PostHog: `https://us.i.posthog.com` (default) or your instance; Aptabase: only for self-hosting.
    pub analytics_host: String,
    /// The PostHog project key or the Aptabase app key.
    pub analytics_key: String,
    /// Send traces of backend calls over OTLP/HTTP.
    pub traces: bool,
    /// e.g. `https://otel.example.com` or `http://localhost:4318` (`/v1/traces` is added).
    pub otlp_endpoint: String,
    /// Extra headers, `key=value` pairs separated by commas (e.g. an API key).
    pub otlp_headers: String,
    /// Sent as the environment / deployment name.
    pub environment: String,
}

fn build_env(key: Option<&'static str>) -> String {
    key.unwrap_or_default().to_string()
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        let aptabase = !build_env(option_env!("APTABASE_KEY")).is_empty() && build_env(option_env!("POSTHOG_KEY")).is_empty();
        Self {
            crash_reports: false,
            sentry_dsn: build_env(option_env!("SENTRY_DSN")),
            analytics: false,
            analytics_provider: if aptabase { "aptabase" } else { "posthog" }.into(),
            analytics_host: build_env(option_env!("POSTHOG_HOST")),
            analytics_key: if aptabase { build_env(option_env!("APTABASE_KEY")) } else { build_env(option_env!("POSTHOG_KEY")) },
            traces: false,
            otlp_endpoint: build_env(option_env!("OTEL_EXPORTER_OTLP_ENDPOINT")),
            otlp_headers: String::new(),
            environment: if cfg!(debug_assertions) { "development" } else { "production" }.into(),
        }
    }
}

impl TelemetryConfig {
    /// Unknown providers become PostHog; text fields are trimmed.
    pub fn validated(mut self) -> Self {
        if !matches!(self.analytics_provider.as_str(), "posthog" | "aptabase") {
            self.analytics_provider = "posthog".into();
        }
        for s in [&mut self.sentry_dsn, &mut self.analytics_host, &mut self.analytics_key, &mut self.otlp_endpoint, &mut self.environment] {
            *s = s.trim().to_string();
        }
        self.environment.truncate(64);
        self
    }

    /// Whether anything at all may leave the machine.
    pub fn any(&self) -> bool {
        self.crash_reports || self.analytics || self.traces
    }
}

// ---- Small helpers (no dependencies) ----

/// Random-enough ids from the standard library's randomly seeded hasher.
fn random_hex(bytes: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut out = String::new();
    while out.len() < bytes * 2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out.truncate(bytes * 2);
    out
}

fn unix_nanos(t: SystemTime) -> u128 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
}

pub use crate::logging::{rfc3339, scrub};

/// A Sentry DSN (`https://KEY@HOST/[PATH/]PROJECT`) as (envelope URL, public key).
pub fn sentry_endpoint(dsn: &str) -> Option<(String, String)> {
    let rest = dsn.strip_prefix("https://")?;
    let (key, rest) = rest.split_once('@')?;
    let key = key.split(':').next()?;
    let (host_path, project) = rest.trim_end_matches('/').rsplit_once('/')?;
    if key.is_empty() || project.is_empty() || !project.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((format!("https://{host_path}/api/{project}/envelope/"), key.to_string()))
}

fn aptabase_host(key: &str, host: &str) -> Option<String> {
    match key.split('-').nth(1)? {
        "EU" => Some("https://eu.aptabase.com".into()),
        "US" => Some("https://us.aptabase.com".into()),
        "DEV" => Some("http://localhost:3000".into()),
        _ if !host.is_empty() => Some(host.trim_end_matches('/').into()),
        _ => None,
    }
}

// ---- The recorder ----

struct Shared {
    app: String,
    version: String,
    install_id: String,
    session_id: String,
    data_dir: PathBuf,
    config: RwLock<TelemetryConfig>,
    last_error: Mutex<Option<String>>,
    sent: std::sync::atomic::AtomicU64,
}

enum Msg {
    Event(Value),
    Span(Value),
    Flush(Sender<()>),
}

struct Telemetry {
    shared: std::sync::Arc<Shared>,
    tx: Mutex<Sender<Msg>>,
}

static GLOBAL: OnceLock<Telemetry> = OnceLock::new();

fn install_id(data_dir: &Path) -> String {
    let file = data_dir.join("telemetry-id");
    if let Ok(id) = std::fs::read_to_string(&file) {
        let id = id.trim();
        if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return id.to_string();
        }
    }
    let id = random_hex(16);
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&file, &id);
    id
}

/// Start the recorder once per process (later calls only update the config). Sends
/// pending crash reports when they are allowed.
pub fn init(data_dir: &Path, app: &str, version: &str, config: TelemetryConfig) {
    if GLOBAL.get().is_some() {
        configure(config);
        return;
    }
    let shared = std::sync::Arc::new(Shared {
        app: app.into(),
        version: version.into(),
        install_id: install_id(data_dir),
        session_id: random_hex(16),
        data_dir: data_dir.to_path_buf(),
        config: RwLock::new(config.validated()),
        last_error: Mutex::new(None),
        sent: Default::default(),
    });
    let (tx, rx) = mpsc::channel();
    let worker = shared.clone();
    let spawned = std::thread::Builder::new().name("telemetry".into()).spawn(move || run(worker, rx));
    if spawned.is_ok() && GLOBAL.set(Telemetry { shared: shared.clone(), tx: Mutex::new(tx) }).is_ok() {
        install_panic_hook(data_dir, app, version);
        let s = shared.clone();
        let _ = std::thread::Builder::new().name("telemetry-crashes".into()).spawn(move || send_crashes(&s));
    }
}

/// Replace the config (from Settings).
pub fn configure(config: TelemetryConfig) {
    if let Some(t) = GLOBAL.get() {
        if let Ok(mut c) = t.shared.config.write() {
            *c = config.validated();
        }
    }
}

fn send(msg: Msg) {
    if let Some(t) = GLOBAL.get() {
        if let Ok(tx) = t.tx.lock() {
            let _ = tx.send(msg);
        }
    }
}

fn enabled(f: impl Fn(&TelemetryConfig) -> bool) -> bool {
    GLOBAL.get().and_then(|t| t.shared.config.read().ok().map(|c| f(&c))).unwrap_or(false)
}

/// An analytics event. `props` must hold only counts, durations, flags and fixed names.
pub fn event(name: &str, props: Value) {
    if enabled(|c| c.analytics && !c.analytics_key.is_empty()) {
        send(Msg::Event(json!({"name": name, "props": props, "time": rfc3339(SystemTime::now())})));
    }
}

/// A trace span for one backend call.
pub fn span(name: &str, start: SystemTime, took: Duration, ok: bool, attrs: &[(&str, Value)]) {
    if enabled(|c| c.traces && !c.otlp_endpoint.is_empty()) {
        let attributes: Vec<Value> = attrs.iter().map(|(k, v)| json!({"key": k, "value": otlp_value(v)})).collect();
        send(Msg::Span(json!({
            "traceId": random_hex(16),
            "spanId": random_hex(8),
            "name": name,
            "kind": 1,
            "startTimeUnixNano": unix_nanos(start).to_string(),
            "endTimeUnixNano": unix_nanos(start + took).to_string(),
            "status": {"code": if ok { 1 } else { 2 }},
            "attributes": attributes,
        })));
    }
}

/// Time one backend call and record it as a span and (for the given names) an event.
pub fn record(method: &str, start: Instant, ok: bool) {
    let took = start.elapsed();
    let wall = SystemTime::now() - took;
    span(method, wall, took, ok, &[("app.ok", json!(ok))]);
    const EVENTS: [&str; 10] = ["setup", "ingest_text", "ingest_file", "ingest_folder", "search", "ask", "speak", "transcribe", "clear", "remove_source"];
    if EVENTS.contains(&method) {
        event(method, json!({"ok": ok, "ms": took.as_millis() as u64}));
    }
}

/// Send what is queued now, waiting at most `timeout` (call before exit).
pub fn flush(timeout: Duration) {
    let (tx, rx) = mpsc::channel();
    send(Msg::Flush(tx));
    let _ = rx.recv_timeout(timeout);
}

/// What is on, the install id, how much was sent and the last problem, for Settings.
pub fn status() -> Value {
    match GLOBAL.get() {
        None => json!({"running": false}),
        Some(t) => {
            let c = t.shared.config.read().map(|c| c.clone()).unwrap_or_default();
            json!({
                "running": true,
                "installId": t.shared.install_id,
                "crashReports": c.crash_reports && sentry_endpoint(&c.sentry_dsn).is_some(),
                "analytics": c.analytics && !c.analytics_key.is_empty(),
                "traces": c.traces && !c.otlp_endpoint.is_empty(),
                "sent": t.shared.sent.load(std::sync::atomic::Ordering::Relaxed),
                "pendingCrashes": crash_files(&t.shared.data_dir).len(),
                "lastError": t.shared.last_error.lock().ok().and_then(|e| e.clone()),
            })
        }
    }
}

fn otlp_value(v: &Value) -> Value {
    match v {
        Value::Bool(b) => json!({"boolValue": b}),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({"intValue": n.to_string()}),
        Value::Number(n) => json!({"doubleValue": n.as_f64()}),
        other => json!({"stringValue": other.as_str().map(String::from).unwrap_or_else(|| other.to_string())}),
    }
}

fn os() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH)
}

/// The PostHog `/batch/` body.
fn posthog_body(s: &Shared, key: &str, events: &[Value]) -> Value {
    let (os, arch) = os();
    let batch: Vec<Value> = events
        .iter()
        .map(|e| {
            let mut props = json!({"$session_id": s.session_id, "$process_person_profile": false, "$os": os, "arch": arch, "app": s.app, "app_version": s.version});
            if let (Some(p), Some(extra)) = (props.as_object_mut(), e["props"].as_object()) {
                p.extend(extra.clone());
            }
            json!({"event": e["name"], "distinct_id": s.install_id, "timestamp": e["time"], "properties": props})
        })
        .collect();
    json!({"api_key": key, "batch": batch})
}

/// The Aptabase `/api/v0/events` body.
fn aptabase_body(s: &Shared, events: &[Value]) -> Value {
    let (os, _) = os();
    Value::Array(
        events
            .iter()
            .map(|e| {
                json!({
                    "timestamp": e["time"], "sessionId": s.session_id, "eventName": e["name"], "props": e["props"],
                    "systemProps": {"isDebug": cfg!(debug_assertions), "osName": os, "osVersion": "", "locale": "", "appVersion": s.version, "sdkVersion": concat!("desktop-core@", env!("CARGO_PKG_VERSION"))},
                })
            })
            .collect(),
    )
}

/// The OTLP/HTTP JSON body for spans.
fn otlp_body(s: &Shared, env: &str, spans: Vec<Value>) -> Value {
    let (os, arch) = os();
    let attr = |k: &str, v: &str| json!({"key": k, "value": {"stringValue": v}});
    json!({"resourceSpans": [{
        "resource": {"attributes": [attr("service.name", &s.app), attr("service.version", &s.version), attr("service.instance.id", &s.install_id), attr("deployment.environment.name", env), attr("os.type", os), attr("host.arch", arch)]},
        "scopeSpans": [{"scope": {"name": "desktop-core", "version": env!("CARGO_PKG_VERSION")}, "spans": spans}],
    }]})
}

fn headers(list: &str) -> Vec<(String, String)> {
    list.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).filter(|(k, _)| !k.is_empty()).collect()
}

fn run(s: std::sync::Arc<Shared>, rx: mpsc::Receiver<Msg>) {
    const MAX_QUEUE: usize = 1000;
    let (mut events, mut spans) = (Vec::new(), Vec::new());
    let mut last = Instant::now();
    loop {
        let mut waiting = None;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Msg::Event(e)) => events.push(e),
            Ok(Msg::Span(sp)) => spans.push(sp),
            Ok(Msg::Flush(done)) => waiting = Some(done),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let due = waiting.is_some() || events.len() + spans.len() >= 50 || last.elapsed() >= Duration::from_secs(15);
        if due && !(events.is_empty() && spans.is_empty()) {
            last = Instant::now();
            let cfg = s.config.read().map(|c| c.clone()).unwrap_or_default();
            let mut problem = None;
            if !events.is_empty() {
                match send_events(&s, &cfg, &events) {
                    Ok(()) => {
                        s.sent.fetch_add(events.len() as u64, std::sync::atomic::Ordering::Relaxed);
                        events.clear();
                    }
                    Err(e) => problem = Some(e),
                }
            }
            if !spans.is_empty() {
                match send_spans(&s, &cfg, &spans) {
                    Ok(()) => {
                        s.sent.fetch_add(spans.len() as u64, std::sync::atomic::Ordering::Relaxed);
                        spans.clear();
                    }
                    Err(e) => problem = Some(e),
                }
            }
            // Keep the newest when offline for long.
            for q in [&mut events, &mut spans] {
                if q.len() > MAX_QUEUE {
                    q.drain(..q.len() - MAX_QUEUE);
                }
            }
            if let Ok(mut l) = s.last_error.lock() {
                *l = problem;
            }
        }
        if let Some(done) = waiting {
            let _ = done.send(());
        }
    }
}

fn send_events(s: &Shared, cfg: &TelemetryConfig, events: &[Value]) -> Result<(), String> {
    if !cfg.analytics || cfg.analytics_key.is_empty() {
        return Ok(()); // switched off since: drop them
    }
    if cfg.analytics_provider == "aptabase" {
        let host = aptabase_host(&cfg.analytics_key, &cfg.analytics_host).ok_or("Aptabase: a self-hosted key needs the host")?;
        crate::net::post_json(&format!("{host}/api/v0/events"), &[("App-Key".into(), cfg.analytics_key.clone())], &aptabase_body(s, events))
    } else {
        let host = if cfg.analytics_host.is_empty() { "https://us.i.posthog.com" } else { cfg.analytics_host.trim_end_matches('/') };
        crate::net::post_json(&format!("{host}/batch/"), &[], &posthog_body(s, &cfg.analytics_key, events))
    }
}

fn send_spans(s: &Shared, cfg: &TelemetryConfig, spans: &[Value]) -> Result<(), String> {
    if !cfg.traces || cfg.otlp_endpoint.is_empty() {
        return Ok(());
    }
    let url = format!("{}/v1/traces", cfg.otlp_endpoint.trim_end_matches('/').trim_end_matches("/v1/traces"));
    crate::net::post_json(&url, &headers(&cfg.otlp_headers), &otlp_body(s, &cfg.environment, spans.to_vec()))
}

// ---- Crashes ----

fn crash_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("crashes")
}

fn crash_files(data_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(crash_dir(data_dir)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
}

/// Write each panic to `<data>/crashes/<time>.json` (then let the default hook print it).
pub fn install_panic_hook(data_dir: &Path, app: &str, version: &str) {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.set(()).is_err() {
        return;
    }
    let (dir, app, version) = (crash_dir(data_dir), app.to_string(), version.to_string());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let (os, arch) = os();
        let report = json!({
            "eventId": random_hex(16),
            "time": rfc3339(SystemTime::now()),
            "app": app,
            "version": version,
            "os": os,
            "arch": arch,
            "thread": std::thread::current().name().unwrap_or("unnamed"),
            "message": scrub(&message),
            "location": scrub(&location),
            "backtrace": scrub(&std::backtrace::Backtrace::force_capture().to_string()),
        });
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("{}-{}.json", unix_nanos(SystemTime::now()), std::process::id()));
        let _ = std::fs::write(file, serde_json::to_vec_pretty(&report).unwrap_or_default());
        // Keep the newest 20.
        let mut old: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        old.sort();
        for f in old.iter().take(old.len().saturating_sub(20)) {
            let _ = std::fs::remove_file(f);
        }
        previous(info);
    }));
}

/// One crash file as a Sentry envelope.
fn sentry_envelope(dsn: &str, env: &str, report: &Value) -> String {
    let id = report["eventId"].as_str().unwrap_or_default().to_string();
    // Sentry lists frames oldest first; a Rust backtrace prints the newest first.
    let mut frames: Vec<Value> = report["backtrace"].as_str().unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).take(200).map(|l| json!({"function": l.trim()})).collect();
    frames.reverse();
    let event = json!({
        "event_id": id,
        "timestamp": report["time"],
        "platform": "native",
        "level": "fatal",
        "release": format!("{}@{}", report["app"].as_str().unwrap_or("app"), report["version"].as_str().unwrap_or("0")),
        "environment": env,
        "contexts": {"os": {"name": report["os"]}, "device": {"arch": report["arch"]}},
        "tags": {"thread": report["thread"]},
        "exception": {"values": [{"type": "panic", "value": report["message"], "stacktrace": {"frames": frames}, "mechanism": {"type": "panic", "handled": false}}]},
        "extra": {"location": report["location"]},
    });
    let event = event.to_string();
    format!("{}\n{}\n{event}\n", json!({"event_id": id, "dsn": dsn, "sent_at": rfc3339(SystemTime::now())}), json!({"type": "event", "length": event.len()}))
}

fn send_crashes(s: &Shared) {
    let cfg = s.config.read().map(|c| c.clone()).unwrap_or_default();
    let Some((url, key)) = sentry_endpoint(&cfg.sentry_dsn).filter(|_| cfg.crash_reports) else { return };
    let auth = format!("Sentry sentry_version=7, sentry_key={key}, sentry_client=desktop-core/{}", env!("CARGO_PKG_VERSION"));
    for file in crash_files(&s.data_dir) {
        let Some(report) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
            let _ = std::fs::remove_file(&file);
            continue;
        };
        let body = sentry_envelope(&cfg.sentry_dsn, &cfg.environment, &report);
        match crate::net::post(&url, &[("X-Sentry-Auth".into(), auth.clone()), ("Content-Type".into(), "application/x-sentry-envelope".into())], body.as_bytes()) {
            Ok(()) => {
                let _ = std::fs::remove_file(&file);
                s.sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => {
                if let Ok(mut l) = s.last_error.lock() {
                    *l = Some(format!("crash report: {e}"));
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared {
            app: "test-app".into(),
            version: "1.2.3".into(),
            install_id: "a".repeat(32),
            session_id: "b".repeat(32),
            data_dir: std::env::temp_dir(),
            config: RwLock::new(TelemetryConfig::default()),
            last_error: Mutex::new(None),
            sent: Default::default(),
        }
    }

    #[test]
    fn ids_and_dsns() {
        let (a, b) = (random_hex(16), random_hex(16));
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert_eq!(sentry_endpoint("https://abc@o1.ingest.sentry.io/42"), Some(("https://o1.ingest.sentry.io/api/42/envelope/".into(), "abc".into())));
        assert_eq!(sentry_endpoint("https://k@host/sub/7/").map(|e| e.0), Some("https://host/sub/api/7/envelope/".into()));
        assert_eq!(sentry_endpoint("http://k@host/7"), None);
        assert_eq!(sentry_endpoint("https://host/7"), None);
        assert_eq!(aptabase_host("A-EU-123", ""), Some("https://eu.aptabase.com".into()));
        assert_eq!(aptabase_host("A-SH-123", ""), None);
        assert_eq!(headers("x-api-key=1, b = 2,bad"), [("x-api-key".into(), "1".into()), ("b".into(), "2".into())]);
    }

    #[test]
    fn bodies_carry_only_fixed_fields() {
        let s = shared();
        let ev = [json!({"name": "search", "props": {"ok": true, "ms": 12}, "time": "t"})];
        let p = posthog_body(&s, "phc_key", &ev);
        assert_eq!(p["api_key"], "phc_key");
        assert_eq!(p["batch"][0]["distinct_id"], s.install_id);
        assert_eq!(p["batch"][0]["properties"]["ms"], 12);
        assert_eq!(p["batch"][0]["properties"]["$process_person_profile"], false);
        let a = aptabase_body(&s, &ev);
        assert_eq!(a[0]["eventName"], "search");
        assert_eq!(a[0]["systemProps"]["appVersion"], "1.2.3");
        let o = otlp_body(&s, "production", vec![json!({"name": "x"})]);
        assert_eq!(o["resourceSpans"][0]["resource"]["attributes"][0]["value"]["stringValue"], "test-app");
        let report = json!({"eventId": "e1", "time": "t", "app": "a", "version": "1", "os": "linux", "arch": "x86_64", "thread": "main", "message": "boom", "location": "src/x.rs:1", "backtrace": "0: a\n1: b"});
        let env = sentry_envelope("https://k@h/1", "production", &report);
        let lines: Vec<Value> = env.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["event_id"], "e1");
        assert_eq!(lines[1]["type"], "event");
        assert_eq!(lines[2]["exception"]["values"][0]["value"], "boom");
        assert_eq!(lines[1]["length"].as_u64().unwrap() as usize, env.lines().nth(2).unwrap().len());
    }

    #[test]
    fn scrubs_the_home_folder() {
        let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
        if home.len() > 1 {
            assert_eq!(scrub(&format!("{home}/x.rs")), "~/x.rs");
        }
    }

    /// Spans reach a local OTLP collector (a one-shot HTTP server here).
    #[test]
    fn spans_reach_a_local_collector() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = c.read(&mut chunk).unwrap_or(0);
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            c.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        let s = shared();
        let cfg = TelemetryConfig { traces: true, otlp_endpoint: format!("http://127.0.0.1:{port}"), otlp_headers: "x-key=1".into(), ..TelemetryConfig::default() };
        let span = json!({"name": "search", "traceId": random_hex(16), "spanId": random_hex(8)});
        send_spans(&s, &cfg, &[span]).unwrap();
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /v1/traces"), "{req}");
        assert!(req.to_ascii_lowercase().contains("x-key: 1"));
        assert!(req.contains("\"name\":\"search\""));
    }
}
