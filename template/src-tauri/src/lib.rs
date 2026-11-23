//! Tauri shell around `desktop-core`.
//!
//! The UI calls these commands (all off the main thread):
//! - `app_info`: version, the parts built in, saved settings, the open store's status,
//! - `run_setup`: open the store and load the model with `setup-progress` events, then
//!   `setup-done`; the first-run wizard calls it once, later starts call it with the saved
//!   settings,
//! - `ingest_text`, `ingest_file` (PDF or text), `search`, `remove_source`, `clear`,
//! - `check_update` / `install_update` (feature `updater`),
//! - `agent_status` / `set_agent` (feature `background-agent`),
//! - `python_call` (feature `python-example`),
//! - `register_claude` (feature `claude-mcp`).
//!
//! The backend sits behind `service::Service`: in this process by default, or in a
//! separate worker process over a local socket with feature `ipc-worker`.

use desktop_core::{Backend, Config, Engine, EngineOptions, Hit, IngestReport, SearchConfig, Status};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Emitter, Manager};

{% if ipc_worker %}{% if background_agent %}#[cfg(feature = "background-agent")]
mod agent;
{% endif %}{% endif %}{% if claude_mcp %}#[cfg(feature = "claude-mcp")]
mod claude;
{% endif %}{% if ipc_worker %}#[cfg(feature = "ipc-worker")]
mod ipc;
{% endif %}{% if python_example %}#[cfg(feature = "python-example")]
mod python;
{% endif %}{% if mcp_server %}#[cfg(feature = "mcp")]
mod mcp;
{% endif %}mod guard;
mod paths;
mod service;
{% if single_instance %}#[cfg(feature = "single-instance")]
mod single;
{% endif %}{% if updater %}#[cfg(feature = "updater")]
mod update;
{% endif %}
use service::Service;

struct AppState {
    service: Arc<dyn Service>,
}

/// Saved in the app's config folder; the wizard writes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    setup_done: bool,
    #[serde(default)]
    backend: Backend,
    #[serde(default = "yes")]
    embeddings: bool,
    /// Search, ingest and watch settings (all optional behaviour off by default).
    #[serde(default)]
    config: Config,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { setup_done: false, backend: Backend::File, embeddings: true, config: Config::default() }
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn settings_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(err)?.join("settings.json"))
}

fn load_settings(app: &tauri::AppHandle) -> Settings {
    settings_file(app).ok().map(|f| settings_at(&f)).unwrap_or_default()
}

/// The saved settings, read without a window (they are optional: defaults when missing).
fn settings_at(file: &Path) -> Settings {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Written to a temporary file and renamed, so a crash never leaves half a settings file.
fn save_settings(app: &tauri::AppHandle, s: &Settings) -> Result<(), String> {
    let file = settings_file(app)?;
    std::fs::create_dir_all(file.parent().ok_or("no config folder")?).map_err(err)?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).map_err(err)?).map_err(err)?;
    std::fs::rename(&tmp, &file).map_err(err)
}

/// Which optional parts this build has.
fn features() -> Value {
    json!({
        "embed": cfg!(feature = "embed"),
        "postgres": cfg!(feature = "postgres"),
        "bm25": cfg!(feature = "bm25"),
        "watch": cfg!(feature = "watch"),
        "ai": cfg!(feature = "ai"),
        "llama": cfg!(feature = "llama"),
        "tts": cfg!(feature = "tts"),
        "stt": cfg!(feature = "stt"),
        "telemetry": cfg!(feature = "telemetry"),
        "mcp": cfg!(feature = "mcp"),
        "pdf": cfg!(feature = "pdf"),
        "wizard": cfg!(feature = "wizard"),
        "updater": cfg!(feature = "updater"),
        "ipcWorker": cfg!(feature = "ipc-worker"),
        "backgroundAgent": cfg!(feature = "background-agent"),
        "pythonExample": cfg!(feature = "python-example"),
        "claudeMcp": cfg!(feature = "claude-mcp"),
    })
}

fn postgres_available() -> bool {
    #[cfg(feature = "postgres")]
    let available = desktop_core::pgstore::available();
    #[cfg(not(feature = "postgres"))]
    let available = false;
    available
}

fn call(state: &tauri::State<AppState>, method: &str, params: Value) -> Result<Value, String> {
    state.service.call(method, params, &mut |_| {})
}

#[tauri::command(async)]
fn app_info(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, String> {
    let status = call(&state, "status", Value::Null).ok().filter(|s| !s.is_null());
    Ok(json!({
        "name": app.package_info().name,
        "version": app.package_info().version.to_string(),
        "features": features(),
        "postgresAvailable": postgres_available(),
        "settings": load_settings(&app),
        "status": status,
        "dataDir": app.path().app_data_dir().map_err(err)?,
    }))
}

#[derive(Clone, Serialize)]
struct Done {
    ok: bool,
    status: Option<Value>,
    error: Option<String>,
}

/// Open the store and load the model in the background (`setup-progress` text events,
/// then `setup-done`). On success the choice is saved and later starts skip the wizard.
#[tauri::command]
fn run_setup(app: tauri::AppHandle, state: tauri::State<AppState>, backend: Option<Backend>, embeddings: Option<bool>) -> Result<(), String> {
    let saved = load_settings(&app);
    let settings = Settings {
        setup_done: true,
        backend: backend.unwrap_or(saved.backend),
        embeddings: embeddings.unwrap_or(saved.embeddings) && cfg!(feature = "embed"),
        config: saved.config,
    };
    let opts = EngineOptions {
        data_dir: app.path().app_data_dir().map_err(err)?,
        backend: settings.backend,
        embeddings: settings.embeddings,
        config: settings.config.clone(),
        app_name: Some(app.package_info().name.clone()),
    };
    let service = state.service.clone();
    std::thread::spawn(move || {
        let mut progress = |msg: &str| {
            let _ = app.emit("setup-progress", msg.to_string());
        };
        let done = match serde_json::to_value(&opts).map_err(err).and_then(|o| service.call("setup", o, &mut progress)) {
            Ok(status) => match save_settings(&app, &settings) {
                Ok(()) => Done { ok: true, status: Some(status), error: None },
                Err(e) => Done { ok: false, status: Some(status), error: Some(e) },
            },
            Err(e) => Done { ok: false, status: None, error: Some(e) },
        };
        let _ = app.emit("setup-done", done);
    });
    Ok(())
}

fn typed<T: serde::de::DeserializeOwned>(v: Result<Value, String>) -> Result<T, String> {
    serde_json::from_value(v?).map_err(err)
}

#[tauri::command(async)]
fn ingest_text(state: tauri::State<AppState>, source: String, text: String) -> Result<usize, String> {
    typed(call(&state, "ingest_text", json!({"source": source, "text": text})))
}

#[tauri::command(async)]
fn ingest_file(state: tauri::State<AppState>, path: String) -> Result<usize, String> {
    typed(call(&state, "ingest_file", json!({"path": path})))
}

/// `search` overrides the saved search settings for this query (the UI's tuning panel).
#[tauri::command(async)]
fn search(state: tauri::State<AppState>, query: String, k: Option<usize>, search: Option<SearchConfig>) -> Result<Vec<Hit>, String> {
    typed(call(&state, "search", json!({"query": query, "k": k, "search": search})))
}

#[tauri::command(async)]
fn ingest_folder(state: tauri::State<AppState>, path: String, recursive: Option<bool>) -> Result<IngestReport, String> {
    typed(call(&state, "ingest_folder", json!({"path": path, "recursive": recursive})))
}

#[tauri::command(async)]
fn get_config(app: tauri::AppHandle) -> Config {
    load_settings(&app).config
}

/// Validate, apply to the running engine (restarting the watcher if needed), and save.
#[tauri::command(async)]
fn set_config(app: tauri::AppHandle, state: tauri::State<AppState>, config: Config) -> Result<Config, String> {
    let config = config.validated();
    let applied: Config = match call(&state, "configure", json!({"config": config})) {
        Ok(v) => serde_json::from_value(v).map_err(err)?,
        // Not set up yet: the saved config applies at the next setup.
        Err(e) if e.starts_with("Still starting") => config,
        Err(e) => return Err(e),
    };
    let mut s = load_settings(&app);
    s.config = applied.clone();
    save_settings(&app, &s)?;
    Ok(applied)
}

#[tauri::command(async)]
fn watch_status(state: tauri::State<AppState>) -> Result<Value, String> {
    call(&state, "watch_status", Value::Null)
}

/// Answer from the documents with the configured LLM; the text streams as `ask-token`.
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn ask(app: tauri::AppHandle, state: tauri::State<AppState>, question: String) -> Result<Value, String> {
    state.service.call("ask", json!({"question": question}), &mut |t| {
        let _ = app.emit("ask-token", t.to_string());
    })
}

/// Download a GGUF model for the built-in llama.cpp and select it (`model-progress` events).
#[cfg(feature = "llama")]
#[tauri::command(async)]
fn download_model(app: tauri::AppHandle, state: tauri::State<AppState>, url: Option<String>) -> Result<Value, String> {
    let path = state.service.call("download_model", json!({"url": url}), &mut |p| {
        let _ = app.emit("model-progress", p.to_string());
    })?;
    // Keep the saved settings in step with the engine.
    let mut s = load_settings(&app);
    s.config.llm.local.model_path = serde_json::from_value(path.clone()).ok();
    save_settings(&app, &s)?;
    Ok(path)
}

/// Read text aloud with tiny-voca: `{wav: base64, seconds}`.
#[cfg(feature = "tts")]
#[tauri::command(async)]
fn speak(state: tauri::State<AppState>, text: String) -> Result<Value, String> {
    call(&state, "speak", json!({"text": text}))
}

/// Fetch the voice model (about 25 MB, checksummed) with `voice-progress` events.
#[cfg(feature = "tts")]
#[tauri::command(async)]
fn download_voice(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, String> {
    state.service.call("download_voice", Value::Null, &mut |p| {
        let _ = app.emit("voice-progress", p.to_string());
    })
}

/// Speech to text with Whisper. `audio` is base64 16-bit 16 kHz mono PCM (what the UI
/// records); returns `{text, language, seconds}`.
#[cfg(feature = "stt")]
#[tauri::command(async)]
fn transcribe(state: tauri::State<AppState>, audio: String) -> Result<Value, String> {
    call(&state, "transcribe", json!({"pcm16": audio}))
}

/// Fetch the chosen Whisper model (checksummed) with `stt-progress` events.
#[cfg(feature = "stt")]
#[tauri::command(async)]
fn download_stt(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, String> {
    state.service.call("download_stt", Value::Null, &mut |p| {
        let _ = app.emit("stt-progress", p.to_string());
    })
}

/// What telemetry is on, the anonymous install id and the last send problem.
#[cfg(feature = "telemetry")]
#[tauri::command(async)]
fn telemetry_status(state: tauri::State<AppState>) -> Result<Value, String> {
    call(&state, "telemetry_status", Value::Null)
}

/// Providers that have a key (the keys never leave the backend).
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn ai_keys(state: tauri::State<AppState>) -> Result<Value, String> {
    call(&state, "ai_keys", Value::Null)
}

/// Store a provider's key (an empty key forgets it).
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn set_ai_key(state: tauri::State<AppState>, provider: String, key: Option<String>) -> Result<Value, String> {
    call(&state, "set_ai_key", json!({"provider": provider, "key": key}))
}

#[tauri::command(async)]
fn remove_source(state: tauri::State<AppState>, source: String) -> Result<usize, String> {
    typed(call(&state, "remove_source", json!({"source": source})))
}

#[tauri::command(async)]
fn clear(state: tauri::State<AppState>) -> Result<Status, String> {
    typed(call(&state, "clear", Value::Null))
}

/// `<app> --worker`: serve the backend over the local socket until told to stop.
/// `--mcp`: serve MCP over stdio (feature `mcp`), sharing the worker when there is one.
#[cfg(feature = "mcp")]
pub fn mcp_main() -> i32 {
    #[cfg(feature = "ipc-worker")]
    let service: Arc<dyn Service> = Arc::new(ipc::Remote::default());
    #[cfg(not(feature = "ipc-worker"))]
    let service: Arc<dyn Service> = Arc::new(service::Local::default());
    let load = |f: &Path| std::fs::read(f).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    mcp::main(load, service)
}

#[cfg(not(feature = "mcp"))]
pub fn mcp_main() -> i32 {
    eprintln!("this build has no MCP server (feature mcp)");
    2
}

/// How to connect an MCP client to this app (the entry, and the Claude Code command).
#[cfg(feature = "mcp")]
#[tauri::command(async)]
fn mcp_setup() -> Result<mcp::Setup, String> {
    mcp::setup()
}

/// Add this app to Claude Desktop's MCP servers (the old config is kept as `.bak`).
#[cfg(feature = "mcp")]
#[tauri::command(async)]
fn mcp_register_claude() -> Result<&'static str, String> {
    mcp::register(&mcp::claude_desktop_config(), mcp::entry()?)
}

#[cfg(feature = "ipc-worker")]
pub fn worker_main() -> i32 {
    let (config, data) = paths::app_dirs();
    desktop_core::logging::init(&data.join("logs"), "worker", settings_at(&config.join("settings.json")).config.log);
    ipc::serve()
}

#[cfg(not(feature = "ipc-worker"))]
pub fn worker_main() -> i32 {
    eprintln!("this build has no worker process (feature ipc-worker)");
    2
}

const SAMPLE: &str = "Tauri apps draw their interface with the system web view.\n\n\
    The heart pumps blood through the arteries to the body.\n\n\
    Rust programs compile to fast native code without a garbage collector.";

/// Check a packaged build without opening a window (CI runs it on the installed app):
/// the store, the model (when built in), and Postgres (when built in and this CPU can run
/// it) each index a sample and find the right chunk. Prints JSON; returns the exit code.
pub fn self_test(out: Option<&Path>) -> i32 {
    let mut report = serde_json::Map::new();
    report.insert("features".into(), features());
    let mut ok = true;
    let tmp = std::env::temp_dir().join(format!("app-self-test-{}", std::process::id()));
    let mut backends = vec![Backend::File];
    if postgres_available() {
        backends.push(Backend::Postgres);
    }
    for backend in backends {
        let opts = EngineOptions { data_dir: tmp.clone(), backend, embeddings: true, config: Config::default(), app_name: None };
        let result = Engine::open(&opts, &mut |_| {}).and_then(|e| {
            e.ingest_text("sample", SAMPLE)?;
            let top = e.search("which organ pumps blood", 1)?;
            // Hybrid mode too (BM25 in Postgres when built in, else full text; BM25 in the file store).
            let hybrid = SearchConfig { mode: desktop_core::SearchMode::Hybrid, top_k: 1, ..SearchConfig::default() };
            let fused = e.search_with("heart pumps blood", &hybrid)?;
            let found = top.first().is_some_and(|h| h.text.contains("heart")) && fused.first().is_some_and(|h| h.text.contains("heart"));
            Ok((e.status()?, found))
        });
        let name = format!("{backend:?}").to_lowercase();
        match result {
            Ok((status, found)) => {
                ok &= found && status.embedder.is_some() == cfg!(feature = "embed");
                report.insert(name, serde_json::json!({"found": found, "status": status}));
            }
            Err(e) => {
                ok = false;
                report.insert(name, serde_json::json!({"error": e.to_string()}));
            }
        }
    }
    #[cfg(feature = "postgres")]
    desktop_core::pgstore::shutdown();
    let _ = std::fs::remove_dir_all(&tmp);
    #[cfg(feature = "claude-mcp")]
    {
        // Release builds bundle uv; a dev build only finds one on PATH.
        let uv = claude::uv_version();
        ok &= uv.is_some() || cfg!(debug_assertions);
        report.insert("uv".into(), serde_json::json!(uv));
    }
    #[cfg(feature = "ipc-worker")]
    {
        // The same sample through a separate worker process.
        let dir = std::env::temp_dir().join(format!("app-self-test-ipc-{}", std::process::id()));
        let opts = json!({"data_dir": dir, "backend": "file", "embeddings": true});
        let r = ipc::request("setup", opts, true, &mut |_| {})
            .and_then(|_| ipc::request("ingest_text", json!({"source": "sample", "text": SAMPLE}), false, &mut |_| {}))
            .and_then(|_| ipc::request("search", json!({"query": "which organ pumps blood", "k": 1}), false, &mut |_| {}));
        ipc::shutdown_worker();
        let _ = std::fs::remove_dir_all(&dir);
        let found = r.as_ref().ok().and_then(|v| v.get(0)).and_then(|h| h.get("text")).and_then(Value::as_str).is_some_and(|t| t.contains("heart"));
        ok &= found;
        report.insert("ipc_worker".into(), json!({"found": found, "error": r.err()}));
    }
    #[cfg(feature = "python-example")]
    {
        // Reported, not required: the machine may have no Python.
        let w = python::PythonWorker::default();
        report.insert("python".into(), match w.call(None, "ping", Value::Null) {
            Ok(v) => v,
            Err(e) => json!({"error": e}),
        });
    }
    report.insert("ok".into(), serde_json::json!(ok));
    let text = serde_json::Value::Object(report).to_string();
    println!("{text}");
    if let Some(out) = out {
        let _ = std::fs::write(out, &text);
    }
    if ok {
        0
    } else {
        1
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (config_dir, data_dir) = paths::app_dirs();
    let process = if cfg!(feature = "ipc-worker") { "window" } else { "app" };
    desktop_core::logging::init(&data_dir.join("logs"), process, settings_at(&config_dir.join("settings.json")).config.log);
    #[cfg(feature = "single-instance")]
    let instance = match single::claim(&data_dir) {
        Ok(single::Instance::Second) => {
            log::info!("already running: asked that window to come to the front");
            return;
        }
        Ok(single::Instance::First(lock, listener)) => Some((lock, listener)),
        Err(e) => {
            log::warn!("single-instance check failed, starting anyway: {e}");
            None
        }
    };
    #[cfg(feature = "ipc-worker")]
    let service: Arc<dyn Service> = Arc::new(ipc::Remote::default());
    #[cfg(not(feature = "ipc-worker"))]
    let service: Arc<dyn Service> = Arc::new(service::Local::default());
    let builder = tauri::Builder::default().manage(AppState { service }).plugin(guard::plugin());
    #[cfg(feature = "python-example")]
    let builder = builder.manage(python::PythonWorker::default());
    #[cfg(feature = "updater")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let builder = builder.setup(move |app| {
        let _ = &app;
        // Panics in the window process are written to <data>/crashes too (sent only if allowed).
        #[cfg(feature = "telemetry")]
        if let Ok(dir) = app.path().app_data_dir() {
            desktop_core::telemetry::install_panic_hook(&dir, &app.package_info().name, &app.package_info().version.to_string());
        }
        #[cfg(feature = "single-instance")]
        if let Some((lock, listener)) = instance {
            app.manage(single::Held(lock));
            let handle = app.handle().clone();
            single::listen(listener, move || {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            });
        }
        Ok(())
    });
    builder
        .invoke_handler(tauri::generate_handler![
            app_info,
            run_setup,
            ingest_text,
            ingest_file,
            search,
            ingest_folder,
            get_config,
            set_config,
            watch_status,
            #[cfg(feature = "ai")]
            ask,
            #[cfg(feature = "ai")]
            ai_keys,
            #[cfg(feature = "ai")]
            set_ai_key,
            #[cfg(feature = "llama")]
            download_model,
            #[cfg(feature = "tts")]
            speak,
            #[cfg(feature = "tts")]
            download_voice,
            #[cfg(feature = "stt")]
            transcribe,
            #[cfg(feature = "stt")]
            download_stt,
            #[cfg(feature = "telemetry")]
            telemetry_status,
            #[cfg(feature = "mcp")]
            mcp_setup,
            #[cfg(feature = "mcp")]
            mcp_register_claude,
            remove_source,
            clear,
            #[cfg(feature = "updater")]
            update::check_update,
            #[cfg(feature = "updater")]
            update::install_update,
            #[cfg(feature = "background-agent")]
            agent::agent_status,
            #[cfg(feature = "background-agent")]
            agent::set_agent,
            #[cfg(feature = "python-example")]
            python::python_call,
            #[cfg(feature = "claude-mcp")]
            claude::register_claude,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                on_exit();
            }
        });
}

/// Stop what this window started. A worker kept by the background agent keeps running.
fn on_exit() {
    #[cfg(feature = "telemetry")]
    desktop_core::telemetry::flush(std::time::Duration::from_secs(2));
    #[cfg(feature = "ipc-worker")]
    {
        #[cfg(feature = "background-agent")]
        let keep = agent::installed();
        #[cfg(not(feature = "background-agent"))]
        let keep = false;
        if !keep {
            ipc::shutdown_worker();
        }
    }
    #[cfg(all(feature = "postgres", not(feature = "ipc-worker")))]
    desktop_core::pgstore::shutdown();
}
