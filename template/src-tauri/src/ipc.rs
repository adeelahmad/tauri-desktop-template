//! Process separation: the backend runs as its own process (`<app> --worker`) and the UI
//! process talks to it over a local socket (a Unix domain socket on macOS and Linux, a
//! named pipe on Windows). The worker can outlive the window, so a background agent can
//! keep it running at login, and a crash in the backend never takes the UI down.
//!
//! Protocol, one request per connection, newline-delimited JSON:
//!   client → `{"method": "...", "params": {...}}`
//!   worker → zero or more `{"progress": "..."}`, then `{"result": ...}` or `{"error": "..."}`
//! `shutdown` stops the worker (used when the app exits without a background agent, and
//! after an update so the new binary takes over).
//!
//! The daemon (feature `daemon`, `<app> --daemon`) serves the same socket through
//! [`bind`] and [`serve_on`], and says so in its `ping` reply (`"daemon": true`): then the
//! window leaves it running when it closes, and after an update it waits for the service
//! manager to start the new binary instead of starting a worker of its own.
//!
//! # Example
//!
//! One exchange, as the lines on the socket:
//!
//! ```
//! use serde_json::{json, Value};
//!
//! let request = json!({"method": "ingest_folder", "params": {"path": "/docs", "recursive": true}});
//! let replies = ["{\"progress\": \"Ingesting 12 files with 4 workers\"}", "{\"result\": {\"files\": 12, \"chunks\": 80}}"];
//! let last: Value = serde_json::from_str(replies[1])?;
//! assert_eq!(request["method"], "ingest_folder");
//! assert_eq!(last["result"]["files"], 12);
//! # Ok::<(), serde_json::Error>(())
//! ```

use crate::service::{Local, Service};
use interprocess::local_socket::{prelude::*, GenericFilePath, GenericNamespaced, Listener, ListenerOptions, Name, Stream};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ID: &str = env!("APP_IDENTIFIER");

/// Per-user socket name: a named pipe or abstract socket where the OS has one, else a
/// file in the user's temporary folder (private to the user on macOS).
pub fn socket_name() -> std::io::Result<Name<'static>> {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    // One worker per profile.
    let base = match crate::paths::profile() {
        crate::paths::DEFAULT => format!("{ID}.{user}.worker.sock"),
        p => format!("{ID}.{user}.{p}.worker.sock"),
    };
    if GenericNamespaced::is_supported() {
        base.to_ns_name::<GenericNamespaced>()
    } else {
        std::env::temp_dir().join(base).to_fs_name::<GenericFilePath>().map(|n| n.into_owned())
    }
}

fn send(stream: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}

fn handle(conn: Stream, local: &Local) -> std::io::Result<bool> {
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let conn = reader.get_mut();
    if method == "shutdown" {
        send(conn, &json!({"result": true}))?;
        return Ok(true);
    }
    let mut out_err = None;
    let mut result = {
        let mut progress = |msg: &str| {
            if out_err.is_none() {
                out_err = send(conn, &json!({"progress": msg})).err();
            }
        };
        local.call(&method, params, &mut progress)
    };
    if method == "ping" && DAEMON.load(Ordering::SeqCst) {
        if let Ok(Value::Object(m)) = &mut result {
            m.insert("daemon".into(), Value::Bool(true));
        }
    }
    let reply = match result {
        Ok(v) => json!({"result": v}),
        Err(e) => json!({"error": e}),
    };
    send(conn, &reply)?;
    Ok(false)
}

/// Whether this process serves as the daemon (set once by `--daemon`).
static DAEMON: AtomicBool = AtomicBool::new(false);
/// Whether the backend this window talks to is a daemon (from its `ping`).
static BACKEND_IS_DAEMON: AtomicBool = AtomicBool::new(false);

/// Mark this process as the daemon: `ping` replies say so.
#[cfg(feature = "daemon")]
pub fn set_daemon() {
    DAEMON.store(true, Ordering::SeqCst);
}

/// Whether the backend this window uses is a daemon (it keeps running when the window closes).
#[cfg(feature = "daemon")]
pub fn backend_is_daemon() -> bool {
    BACKEND_IS_DAEMON.load(Ordering::SeqCst)
}

/// Listen on this profile's socket; `Ok(None)` when a worker or daemon already listens.
pub fn bind() -> Result<Option<Listener>, String> {
    let name = socket_name().map_err(|e| e.to_string())?;
    if Stream::connect(name.clone()).is_ok() {
        return Ok(None);
    }
    ListenerOptions::new().name(name).try_overwrite(true).create_sync().map(Some).map_err(|e| format!("cannot listen: {e}"))
}

/// Serve requests with `local` (one thread per connection) until the process exits.
/// `on_shutdown` runs when a client sends `shutdown`.
pub fn serve_on(listener: Listener, local: Arc<Local>, on_shutdown: fn()) {
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let local = local.clone();
        std::thread::spawn(move || {
            if handle(conn, &local).unwrap_or(false) {
                on_shutdown();
            }
        });
    }
}

fn worker_exit() {
    #[cfg(feature = "postgres")]
    desktop_core::pgstore::shutdown();
    std::process::exit(0);
}

/// Run the worker until `shutdown`. Exits 0 at once if another worker already listens.
pub fn serve() -> i32 {
    match bind() {
        Ok(Some(listener)) => {
            let local = Arc::new(Local::default());
{% if background_jobs %}{% if background_agent %}            #[cfg(all(feature = "background-agent", feature = "jobs-queue"))]
            crate::jobs_autostart(&local);
{% endif %}{% endif %}            serve_on(listener, local, worker_exit);
            0
        }
        Ok(None) => {
            eprintln!("worker: already running");
            0
        }
        Err(e) => {
            eprintln!("worker: {e}");
            1
        }
    }
}

/// The UI side: forwards every call to the worker, starting it when none is running.
#[derive(Default)]
pub struct Remote {
    checked: AtomicBool,
}

fn spawn_worker() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .arg("--worker")
        .env("APP_PROFILE", crate::paths::profile())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start the worker: {e}"))?;
    Ok(())
}

fn connect(start: bool) -> Result<Stream, String> {
    let name = socket_name().map_err(|e| e.to_string())?;
    if let Ok(s) = Stream::connect(name.clone()) {
        return Ok(s);
    }
    if !start {
        return Err("worker not running".into());
    }
    spawn_worker()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match Stream::connect(name.clone()) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() > deadline => return Err(format!("the worker did not start: {e}")),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// One request over a fresh connection.
pub fn request(method: &str, params: Value, start: bool, progress: &mut dyn FnMut(&str)) -> Result<Value, String> {
    let mut stream = connect(start)?;
    send(&mut stream, &json!({"method": method, "params": params})).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("the worker closed the connection".into());
        }
        let msg: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(p) = msg.get("progress").and_then(Value::as_str) {
            progress(p);
        } else if let Some(e) = msg.get("error") {
            return Err(e.as_str().map(String::from).unwrap_or_else(|| e.to_string()));
        } else {
            return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Stop a running worker (no-op when none runs).
pub fn shutdown_worker() {
    let _ = request("shutdown", Value::Null, false, &mut |_| {});
}

impl Service for Remote {
    fn call(&self, method: &str, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, String> {
        // A worker left over from an older version (e.g. kept alive by the background
        // agent across an update) is replaced once.
        if !self.checked.swap(true, Ordering::SeqCst) {
            let ping = request("ping", Value::Null, true, &mut |_| {})?;
            let daemon = ping.get("daemon").and_then(Value::as_bool) == Some(true);
            BACKEND_IS_DAEMON.store(daemon, Ordering::SeqCst);
            if ping.get("version").and_then(Value::as_str) != Some(env!("CARGO_PKG_VERSION")) {
                shutdown_worker();
                if daemon {
                    // The service manager starts the new binary in a few seconds; use that
                    // rather than a worker of our own (which would keep the daemon out).
                    let deadline = Instant::now() + Duration::from_secs(20);
                    std::thread::sleep(Duration::from_millis(500));
                    while connect(false).is_err() && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        }
        request(method, params, true, progress)
    }
}
