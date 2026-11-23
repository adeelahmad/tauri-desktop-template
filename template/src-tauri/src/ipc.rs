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

use crate::service::{Local, Service};
use interprocess::local_socket::{prelude::*, GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream};
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
    let base = format!("{ID}.{user}.worker.sock");
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
    let result = {
        let mut progress = |msg: &str| {
            if out_err.is_none() {
                out_err = send(conn, &json!({"progress": msg})).err();
            }
        };
        local.call(&method, params, &mut progress)
    };
    let reply = match result {
        Ok(v) => json!({"result": v}),
        Err(e) => json!({"error": e}),
    };
    send(conn, &reply)?;
    Ok(false)
}

/// Run the worker until `shutdown`. Exits 0 at once if another worker already listens.
pub fn serve() -> i32 {
    let name = match socket_name() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("worker: {e}");
            return 1;
        }
    };
    if Stream::connect(name.clone()).is_ok() {
        eprintln!("worker: already running");
        return 0;
    }
    let listener = match ListenerOptions::new().name(name).try_overwrite(true).create_sync() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("worker: cannot listen: {e}");
            return 1;
        }
    };
    let local = Arc::new(Local::default());
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let local = local.clone();
        std::thread::spawn(move || {
            if handle(conn, &local).unwrap_or(false) {
                #[cfg(feature = "postgres")]
                desktop_core::pgstore::shutdown();
                std::process::exit(0);
            }
        });
    }
    0
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
            if ping.get("version").and_then(Value::as_str) != Some(env!("CARGO_PKG_VERSION")) {
                shutdown_worker();
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        request(method, params, true, progress)
    }
}
