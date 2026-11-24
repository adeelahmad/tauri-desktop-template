//! Python worker example: a long-lived Python child process (`python/worker.py`) behind
//! the `python_call` command, speaking newline-delimited JSON over stdin/stdout.
//!
//! Interpreter, first that works: `APP_PYTHON`; the bundled uv (feature `claude-mcp`),
//! which fetches Python itself; `python3`; `python`; `py -3` on Windows. Release builds
//! ship the script as a resource (`bundle.resources` in tauri.conf.json).
//!
//! # Example
//!
//! The JSON lines between the app and `worker.py`:
//!
//! ```
//! use serde_json::json;
//!
//! let request = json!({"id": 1, "method": "stats", "params": {"text": "hi"}});
//! let reply: serde_json::Value = serde_json::from_str(r#"{"id": 1, "result": {"words": 1}}"#)?;
//! assert_eq!(reply["id"], request["id"]);
//! # Ok::<(), serde_json::Error>(())
//! ```

use crate::error::CommandError;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

/// The Python child process, started on the first call (Tauri state).
#[derive(Default)]
pub struct PythonWorker {
    proc: Mutex<Option<Proc>>,
}

/// Where `worker.py` is: the app's resources, next to the executable, or the source tree.
pub fn script(resource_dir: Option<PathBuf>) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(r) = resource_dir {
        candidates.push(r.join("python/worker.py"));
    }
    if let Some(d) = exe_dir {
        candidates.push(d.join("python/worker.py"));
        candidates.push(d.join("../Resources/python/worker.py"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../python/worker.py"));
    candidates.into_iter().find(|p| p.is_file())
}

fn interpreters(script: &Path) -> Vec<Command> {
    let mut out = Vec::new();
    let with = |prog: &str, pre: &[&str]| {
        let mut c = Command::new(prog);
        c.args(pre).arg(script);
        desktop_core::net::apply_env(&mut c);
        // python.exe is a console program: without this the windowed app opens a console for it.
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
        c
    };
    if let Ok(p) = std::env::var("APP_PYTHON") {
        out.push(with(&p, &[]));
    }
    #[cfg(feature = "claude-mcp")]
    if let Some(uv) = crate::claude::bundled_uv() {
        out.push(with(&uv.to_string_lossy(), &["run", "--no-project", "--python", crate::claude::PYTHON]));
    }
    out.push(with("python3", &[]));
    out.push(with("python", &[]));
    if cfg!(windows) {
        out.push(with("py", &["-3"]));
    }
    out
}

fn start(script: &Path) -> Result<Proc, String> {
    let mut last = String::from("no Python found");
    for mut cmd in interpreters(script) {
        match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn() {
            Ok(mut child) => {
                let stdin = child.stdin.take().ok_or("no stdin")?;
                let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
                let mut p = Proc { child, stdin, stdout, next_id: 1 };
                // A stub launcher (e.g. the Microsoft Store `python`) starts but never answers.
                match exchange(&mut p, "ping", Value::Null) {
                    Ok(_) => return Ok(p),
                    Err(e) => {
                        let _ = p.child.kill();
                        last = e;
                    }
                }
            }
            Err(e) => last = format!("{cmd:?}: {e}"),
        }
    }
    Err(format!("Could not start Python ({last}). Install Python 3 or set APP_PYTHON."))
}

fn exchange(p: &mut Proc, method: &str, params: Value) -> Result<Value, String> {
    let id = p.next_id;
    p.next_id += 1;
    let mut line = serde_json::to_vec(&json!({"id": id, "method": method, "params": params})).map_err(|e| e.to_string())?;
    line.push(b'\n');
    p.stdin.write_all(&line).and_then(|_| p.stdin.flush()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    if p.stdout.read_line(&mut reply).map_err(|e| e.to_string())? == 0 {
        return Err("the Python worker exited".into());
    }
    let msg: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
    match msg.get("error") {
        Some(e) if !e.is_null() => Err(e.as_str().map(String::from).unwrap_or_else(|| e.to_string())),
        _ => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
    }
}

impl PythonWorker {
    /// Call `method`, starting the worker on first use and once more if it died.
    pub fn call(&self, resource_dir: Option<PathBuf>, method: &str, params: Value) -> Result<Value, String> {
        let mut guard = self.proc.lock().map_err(|e| e.to_string())?;
        for attempt in 0..2 {
            if guard.is_none() {
                let script = script(resource_dir.clone()).ok_or("python/worker.py is missing from this build")?;
                *guard = Some(start(&script)?);
            }
            match exchange(guard.as_mut().unwrap(), method, params.clone()) {
                Err(e) if attempt == 0 && e.contains("exited") => *guard = None,
                other => return other,
            }
        }
        unreachable!()
    }
}

impl Drop for PythonWorker {
    fn drop(&mut self) {
        if let Ok(mut g) = self.proc.lock() {
            if let Some(mut p) = g.take() {
                let _ = p.child.kill();
            }
        }
    }
}

/// Tauri command: call `method` in `worker.py` with `params` and return its result.
#[tauri::command(async)]
pub fn python_call(app: tauri::AppHandle, worker: tauri::State<PythonWorker>, method: String, params: Option<Value>) -> Result<Value, CommandError> {
    use tauri::Manager;
    Ok(worker.call(app.path().resource_dir().ok(), &method, params.unwrap_or(Value::Null))?)
}
