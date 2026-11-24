//! Tauri shell around `desktop-core`.
//!
//! The UI calls these commands (all off the main thread):
//! - `app_info`: version, the parts built in, saved settings, the open store's status,
//! - `run_setup`: open the store and load the model with `setup-progress` events, then
//!   `setup-done`; the first-run wizard calls it once, later starts call it with the saved
//!   settings,
//! - `ingest_text`, `ingest_file` (PDF or text), `search`, `remove_source`, `clear`,
//! - `check_update` / `install_update` (feature `updater`); `update_channel_status` and
//!   `rollback_update` (feature `release-channels`),
//! - `third_party_notices` (feature `supply-chain`: Settings → About → Licences),
//! - `agent_status` / `set_agent` (feature `background-agent`),
//! - `service_status`, `set_service` and `service_system_hint` (feature `daemon`: the
//!   backend as a service, see `daemon`),
//! - `python_call` (feature `python-example`),
//! - `register_claude` (feature `claude-mcp`),
//! - `mcp_setup`, `mcp_register_claude`, `mcp_pending` and `mcp_decide` (feature `mcp`).
//! - `plugins_list` / `plugin_call` (feature `plugins`),
//! - `rclone_status`, `rclone_config`, `rclone_install`, `rclone_sync` (feature `rclone`),
//! - `tailscale_status` / `tailscale_apply` (feature `tailscale`),
//! - `oauth_login`, `oauth_logout`, `oauth_status` and `oauth_client_secret` (feature `oauth`).
//! - `webview_open`, `webview_close`, `webview_post` (feature `webview`, desktop only),
//! - `jobs_list`, `jobs_enqueue`, `jobs_cancel`, `jobs_retry`, `jobs_schedules` and
//!   `jobs_set_schedule` (feature `jobs-queue`),
//! - `tray_status` / `run_action` (feature `tray-menu`: the tray, global shortcuts and the app menu),
//! - `http_api_info`: the local REST API's URL and token (feature `http-api`).
//! - `open_settings`, `get_appearance` (feature `app-shell`: the Settings window and themes; see `shell`).
//!
//! The backend sits behind `service::Service`: in this process by default, or in a
//! separate worker process over a local socket with feature `ipc-worker`.
//!
//! The same crate is the iOS and Android app (`run` is the mobile entry point). There the
//! desktop-only parts are compiled out with `cfg(desktop)`: the worker process and the
//! background agent (the backend runs in this process), the updater (the app stores
//! update the app), single instance, the stdio MCP server, Python, Claude Desktop,
//! folder watching and embedded Postgres (the file store is used).
//!
//! # Layers
//!
//! ```text
//! UI (ui/ or ui-react/)  --invoke-->  Tauri commands (this file)
//!                                      |  call(method, params)
//!                                      v
//!                          service::Service
//!                      Local (this process)  |  ipc::Remote --socket--> `<app> --worker` (ipc::serve -> Local)
//!                                      |
//!                                      v
//!                          desktop_core::Engine (store, embedder, config)
//! ```
//!
//! Every command is a thin wrapper: it turns its arguments into a JSON `params` object and
//! calls the service by method name, so the same backend serves the window, the worker
//! process, `--mcp` and `--self-test`. A failed command rejects with an [`error::CommandError`]
//! (`{code, message, details?}`, stable codes in docs/GUIDE.md). Settings live in `<config>/settings.json` (see
//! `paths` for profiles); `get_config` / `set_config` read and write the `config` part, and
//! `set_config` also hands it to the running engine (`configure`).
//!
//! Build-time app config (the `config/` folder, merged per environment and OS) is
//! [`app_config`]; the user's run-time settings are `settings.json`.
//!
//! Modules: `app_config` and `config_layers` (build-time config), `service` (the backend), `paths` (folders and profiles), `guard` (window
//! security), and per feature `ipc` + `agent` (worker process and login agent), `mcp`, `eval` (`--eval`),
//! `update`, `single`, `python`, `claude`, `webview`, `http` (the local REST API), `shell`
//! (titlebar, themes, Settings window).
//!
//! # Example
//!
//! The entry points are what `main.rs` calls; opening a window needs a desktop session:
//!
//! ```no_run
//! // `<app>`: the window.
//! app_lib::run();
//! ```
//!
//! ```no_run
//! // `<app> --self-test report.json`: index and search a sample with every built-in store.
//! std::process::exit(app_lib::self_test(Some(std::path::Path::new("report.json"))));
//! ```

use crate::error::CommandError;
use desktop_core::{Backend, Config, Engine, EngineOptions, Hit, IngestReport, SearchConfig, Status};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Emitter, Manager};

{% if ipc_worker %}{% if background_agent %}#[cfg(all(desktop, feature = "background-agent"))]
mod agent;
{% endif %}{% if daemon %}#[cfg(all(desktop, feature = "daemon"))]
mod daemon;
{% endif %}{% endif %}{% if claude_mcp %}#[cfg(all(desktop, feature = "claude-mcp"))]
mod claude;
{% endif %}{% if ipc_worker %}#[cfg(all(desktop, feature = "ipc-worker"))]
mod ipc;
{% endif %}{% if python_example %}#[cfg(all(desktop, feature = "python-example"))]
mod python;
{% endif %}{% if mcp_server %}#[cfg(all(desktop, feature = "mcp"))]
mod mcp;
{% endif %}{% if webview %}#[cfg(all(desktop, feature = "webview"))]
mod webview;
{% endif %}{% if http_api %}#[cfg(all(desktop, feature = "http-api"))]
mod http;
{% endif %}{% if evals %}#[cfg(feature = "evals")]
mod eval;
{% endif %}pub mod app_config;
pub mod config_layers;
pub mod error;
mod guard;
mod paths;
mod service;
// kitchen:mods (`kitchen generate command` adds modules above this line)
{% if single_instance %}#[cfg(all(desktop, feature = "single-instance"))]
mod single;
{% endif %}{% if tray_menu %}#[cfg(all(desktop, feature = "tray-menu"))]
mod tray;
{% endif %}{% if updater %}#[cfg(all(desktop, feature = "updater"))]
mod update;
{% endif %}{% if app_shell %}#[cfg(feature = "app-shell")]
mod shell;
{% endif %}{% if supply_chain %}#[cfg(feature = "supply-chain")]
mod notices;
{% endif %}
use service::Service;

struct AppState {
    service: Arc<dyn Service>,
}

/// Saved in the app's config folder; the wizard writes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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

/// The settings of the running profile (see `paths`).
fn settings_file(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(paths::app_dirs().0.join("settings.json"))
}

fn load_settings(app: &tauri::AppHandle) -> Settings {
    settings_file(app).ok().map(|f| settings_at(&f)).unwrap_or_default()
}

/// The saved settings, read without a window (they are optional: defaults when missing).
fn settings_at(file: &Path) -> Settings {
    desktop_core::untrusted::read_capped(file, desktop_core::untrusted::limits::CONFIG_BYTES)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Written to a temporary file and renamed, so a crash never leaves half a settings file.
fn save_settings(app: &tauri::AppHandle, s: &Settings) -> Result<(), String> {
    write_settings(&settings_file(app)?, s)
}

fn write_settings(file: &Path, s: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(file.parent().ok_or("no config folder")?).map_err(err)?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).map_err(err)?).map_err(err)?;
    std::fs::rename(&tmp, file).map_err(err)
}

/// Save `config` as the settings' `config` part (`PUT /v1/settings` on the HTTP API).
#[cfg(all(desktop, feature = "http-api"))]
fn save_config(config: Config) -> Result<(), String> {
    let file = paths::app_dirs().0.join("settings.json");
    let mut s = settings_at(&file);
    s.config = config;
    write_settings(&file, &s)
}

/// Which optional parts this build has. On iOS and Android the desktop-only parts
/// (embedded Postgres, folder watching, the worker process, the background agent, the
/// updater, the MCP server, Python and Claude Desktop) are compiled out and report `false`,
/// and `mobile` is `true`, so the UI hides their sections.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
struct Features {
    /// This is the iOS or Android app.
    mobile: bool,
    /// Titlebar, themes and the Settings window (`app-shell`).
    app_shell: bool,
    /// Local embeddings (`embed`).
    embed: bool,
    /// Embedded Postgres (`postgres`, desktop).
    postgres: bool,
    /// BM25 keyword ranking in Postgres (`bm25`).
    bm25: bool,
    /// Folder watching (`watch`).
    watch: bool,
    /// AI chat (`ai`).
    ai: bool,
    /// Built-in llama.cpp (`llama`).
    llama: bool,
    /// Text to speech (`tts`).
    tts: bool,
    /// Speech to text (`stt`).
    stt: bool,
    /// Email (`mail`).
    mail: bool,
    /// Plugins (`plugins`).
    plugins: bool,
    /// Background jobs (`jobs-queue`).
    jobs: bool,
    /// Cloud sync with rclone (`rclone`).
    rclone: bool,
    /// Tailscale (`tailscale`).
    tailscale: bool,
    /// Sign-in (`oauth`).
    oauth: bool,
    /// Sign-in can return through the app's own URL scheme.
    oauth_deep_link: bool,
    /// Embedded web views (`webview`, desktop).
    webview: bool,
    /// The local REST API (`http-api`, desktop).
    http_api: bool,
    /// Opt-in telemetry (`telemetry`).
    telemetry: bool,
    /// The MCP server (`mcp`, desktop).
    mcp: bool,
    /// PDF text (`pdf`).
    pdf: bool,
    /// The first-run wizard (`wizard`).
    wizard: bool,
    /// Auto-update (`updater`, desktop).
    updater: bool,
    /// The backend runs in a worker process (`ipc-worker`, desktop).
    ipc_worker: bool,
    /// The worker starts at login (`background-agent`, desktop).
    background_agent: bool,
    /// Daemon mode and OS services (`daemon`, desktop).
    daemon: bool,
    /// The Python worker example (`python-example`, desktop).
    python_example: bool,
    /// Claude Desktop registration (`claude-mcp`, desktop).
    claude_mcp: bool,
    /// Tray, global shortcuts and app menu (`tray-menu`, desktop).
    tray_menu: bool,
    /// Licence notices and supply-chain checks (`supply-chain`).
    supply_chain: bool,
    /// Update channels, staged rollout and rollback (`release-channels`, desktop).
    release_channels: bool,
    /// Search and RAG evals, `--eval` (`evals`).
    evals: bool,
}

fn features() -> Features {
    Features {
        mobile: cfg!(mobile),
        app_shell: cfg!(feature = "app-shell"),
        embed: cfg!(feature = "embed"),
        postgres: cfg!(all(desktop, feature = "postgres")),
        bm25: cfg!(all(desktop, feature = "bm25")),
        watch: cfg!(all(desktop, feature = "watch")),
        ai: cfg!(feature = "ai"),
        llama: cfg!(feature = "llama"),
        tts: cfg!(feature = "tts"),
        stt: cfg!(feature = "stt"),
        mail: cfg!(feature = "mail"),
        plugins: cfg!(feature = "plugins"),
        jobs: cfg!(feature = "jobs-queue"),
        rclone: cfg!(feature = "rclone"),
        tailscale: cfg!(feature = "tailscale"),
        oauth: cfg!(feature = "oauth"),
        oauth_deep_link: {% if tauri_plugins %}{% if tauri_deep_link %}cfg!(feature = "oauth"){% else %}false{% endif %}{% else %}false{% endif %},
        webview: cfg!(all(desktop, feature = "webview")),
        http_api: cfg!(all(desktop, feature = "http-api")),
        telemetry: cfg!(feature = "telemetry"),
        mcp: cfg!(all(desktop, feature = "mcp")),
        pdf: cfg!(feature = "pdf"),
        wizard: cfg!(feature = "wizard"),
        updater: cfg!(all(desktop, feature = "updater")),
        ipc_worker: cfg!(all(desktop, feature = "ipc-worker")),
        background_agent: cfg!(all(desktop, feature = "background-agent")),
        daemon: cfg!(all(desktop, feature = "daemon")),
        python_example: cfg!(all(desktop, feature = "python-example")),
        claude_mcp: cfg!(all(desktop, feature = "claude-mcp")),
        tray_menu: cfg!(all(desktop, feature = "tray-menu")),
        supply_chain: cfg!(feature = "supply-chain"),
        release_channels: cfg!(all(desktop, feature = "release-channels")),
        evals: cfg!(feature = "evals"),
    }
}

/// Embedded Postgres runs on desktop only: iOS and Android always use the file store.
fn postgres_available() -> bool {
    #[cfg(all(desktop, feature = "postgres"))]
    let available = desktop_core::pgstore::available();
    #[cfg(not(all(desktop, feature = "postgres")))]
    let available = false;
    available
}

/// The store to open: the chosen one, except that a Postgres choice falls back to the file
/// store where Postgres cannot run (iOS and Android), so settings copied from a desktop or a
/// restored profile still open.
fn store_for(chosen: Backend) -> Backend {
    if cfg!(mobile) && chosen == Backend::Postgres {
        log::info!("Postgres is not available on this device; using the file store");
        return Backend::File;
    }
    chosen
}

fn call(state: &tauri::State<AppState>, method: &str, params: Value) -> Result<Value, CommandError> {
    state.service.call(method, params, &mut |_| {})
}

/// What the UI needs at start: the app, the parts built in, the saved settings and the store.
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: String,
    version: String,
    features: Features,
    /// Postgres can run on this computer (when built in).
    postgres_available: bool,
    settings: Settings,
    /// The open store, or `null` before setup.
    status: Option<Status>,
    data_dir: PathBuf,
    /// The running profile's id.
    profile: String,
    /// The build-time app config (config/*.json merged for this build); see `app_config`.
    config: Value,
}

/// The app, the parts built in, the saved settings and the open store's status.
#[tauri::command(async)]
fn app_info(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<AppInfo, CommandError> {
    let status = call(&state, "status", Value::Null).ok().filter(|s| !s.is_null()).and_then(|s| serde_json::from_value(s).ok());
    Ok(AppInfo {
        name: app.package_info().name.clone(),
        version: app.package_info().version.to_string(),
        features: features(),
        postgres_available: postgres_available(),
        settings: load_settings(&app),
        status,
        data_dir: paths::app_dirs().1,
        profile: paths::profile().to_string(),
        config: app_config::CONFIG.clone(),
    })
}

/// The profiles: which one is running, and the saved list (with the one the next start opens).
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct ProfileList {
    running: String,
    saved: paths::Profiles,
}

/// The profiles, which one is running and which one the next start opens.
#[tauri::command]
fn list_profiles() -> ProfileList {
    ProfileList { running: paths::profile().to_string(), saved: paths::profiles() }
}

#[tauri::command]
fn create_profile(name: String) -> Result<paths::Profile, CommandError> {
    Ok(paths::create(&name)?)
}

#[tauri::command]
fn rename_profile(id: String, name: String) -> Result<(), CommandError> {
    Ok(paths::rename(&id, &name)?)
}

/// Remove a profile; its documents and settings are deleted only with `delete_data`.
#[tauri::command]
fn remove_profile(id: String, delete_data: bool) -> Result<(), CommandError> {
    Ok(paths::remove(&id, delete_data)?)
}

/// Open another profile: it becomes the active one and the app restarts into it.
#[tauri::command]
fn switch_profile(app: tauri::AppHandle, id: String) -> Result<(), CommandError> {
    paths::set_active(&id)?;
    log::info!("switching to profile {id}");
    // The same orderly stop as quitting; this profile's worker goes too (the new one starts its own).
    on_exit(&app);
    #[cfg(all(desktop, feature = "ipc-worker"))]
    ipc::shutdown_worker();
    log::logger().flush();
    app.restart()
}

#[derive(Clone, Serialize)]
struct Done {
    ok: bool,
    status: Option<Value>,
    error: Option<String>,
}

/// Open the store and load the model in the background (`setup-progress` text events,
/// then `setup-done`). On success the choice is saved and later starts skip the wizard.
{% if postgres %}/// `pg_upgrade` answers the prompt a `pg-upgrade-needed` error asks for: `true` upgrades the
/// Postgres data now, `false` runs on the old server this time (the setting is not changed).
{% endif %}#[tauri::command]
fn run_setup(app: tauri::AppHandle, state: tauri::State<AppState>, backend: Option<Backend>, embeddings: Option<bool>{% if postgres %}, pg_upgrade: Option<bool>{% endif %}) -> Result<(), String> {
    let saved = load_settings(&app);
    let settings = Settings {
        setup_done: true,
        backend: store_for(backend.unwrap_or(saved.backend)),
        embeddings: embeddings.unwrap_or(saved.embeddings) && cfg!(feature = "embed"),
        config: saved.config,
    };
    #[allow(unused_mut)]
    let mut opts = EngineOptions {
        data_dir: paths::app_dirs().1,
        backend: settings.backend,
        embeddings: settings.embeddings,
        config: settings.config.clone(),
        app_name: Some(app.package_info().name.clone()),
    };
{% if postgres %}    #[cfg(feature = "postgres")]
    if let Some(now) = pg_upgrade {
        use desktop_core::pgupgrade::{clear_failure, Layout, UpgradeMode};
        opts.config.postgres.upgrade = if now { UpgradeMode::Auto } else { UpgradeMode::Off };
        if now {
            // A yes after a failed try means: try again.
            clear_failure(&Layout { root: opts.data_dir.join("postgres") }).map_err(err)?;
        }
    }
    #[cfg(not(feature = "postgres"))]
    let _ = pg_upgrade;
{% endif %}    let service = state.service.clone();
    std::thread::spawn(move || {
        let mut progress = |msg: &str| {
            let _ = app.emit("setup-progress", msg.to_string());
        };
        let done = match serde_json::to_value(&opts).map_err(CommandError::from).and_then(|o| service.call("setup", o, &mut progress)) {
            Ok(status) => match save_settings(&app, &settings) {
                Ok(()) => {
                    #[cfg(all(desktop, feature = "http-api"))]
                    http::apply(service.clone(), &settings.config.http_api);
                    Done { ok: true, status: Some(status), error: None }
                }
                Err(e) => Done { ok: false, status: Some(status), error: Some(e) },
            },
            Err(e) => Done { ok: false, status: None, error: Some(e.message) },
        };
        let _ = app.emit("setup-done", done);
    });
    Ok(())
}

fn typed<T: serde::de::DeserializeOwned>(v: Result<Value, CommandError>) -> Result<T, CommandError> {
    Ok(serde_json::from_value(v?).map_err(err)?)
}

#[tauri::command(async)]
fn ingest_text(state: tauri::State<AppState>, source: String, text: String) -> Result<usize, CommandError> {
    typed(call(&state, "ingest_text", json!({"source": source, "text": text})))
}

#[tauri::command(async)]
fn ingest_file(state: tauri::State<AppState>, path: String) -> Result<usize, CommandError> {
    typed(call(&state, "ingest_file", json!({"path": path})))
}

/// `search` overrides the saved search settings for this query (the UI's tuning panel).
#[tauri::command(async)]
fn search(state: tauri::State<AppState>, query: String, k: Option<usize>, search: Option<SearchConfig>) -> Result<Vec<Hit>, CommandError> {
    typed(call(&state, "search", json!({"query": query, "k": k, "search": search})))
}

#[tauri::command(async)]
fn ingest_folder(state: tauri::State<AppState>, path: String, recursive: Option<bool>) -> Result<IngestReport, CommandError> {
    typed(call(&state, "ingest_folder", json!({"path": path, "recursive": recursive})))
}

#[tauri::command(async)]
fn get_config(app: tauri::AppHandle) -> Config {
    load_settings(&app).config
}

/// Validate, apply to the running engine (restarting the watcher if needed), and save.
#[tauri::command(async)]
fn set_config(app: tauri::AppHandle, state: tauri::State<AppState>, config: Config) -> Result<Config, CommandError> {
    let config = config.validated();
    let applied: Config = match call(&state, "configure", json!({"config": config})) {
        Ok(v) => serde_json::from_value(v).map_err(err)?,
        // Not set up yet: the saved config applies at the next setup.
        Err(e) if e.code == error::codes::NOT_READY => config,
        Err(e) => return Err(e),
    };
    // Every open window switches theme and fonts at once.
    #[cfg(feature = "app-shell")]
    shell::applied(&app, &applied.appearance);
    let mut s = load_settings(&app);
    s.config = applied.clone();
    save_settings(&app, &s)?;
    #[cfg(all(desktop, feature = "http-api"))]
    http::apply(state.service.clone(), &applied.http_api);
    // The tray, the menus and the shortcuts change at once.
    #[cfg(all(desktop, feature = "tray-menu"))]
    tray::apply(&app, &applied);
    Ok(applied)
}

/// Back up the index now.
#[tauri::command(async)]
fn backup_now(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "backup", Value::Null)
}

/// Backups in the backup folder, newest first.
#[tauri::command(async)]
fn list_backups(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "backups", Value::Null)
}

/// Replace the index with a backup (the current one is backed up first).
#[tauri::command(async)]
fn restore_backup(state: tauri::State<AppState>, path: String, key: Option<String>) -> Result<Status, CommandError> {
    typed(call(&state, "restore", json!({ "path": path, "key": key })))
}

/// The recovery key for encrypted backups (shown so it can be kept somewhere safe).
#[tauri::command(async)]
fn backup_recovery_key(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "backup_key", Value::Null)
}

#[tauri::command(async)]
fn watch_status(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "watch_status", Value::Null)
}

/// Answer from the documents with the configured LLM; the text streams as `ask-token`.
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn ask(app: tauri::AppHandle, state: tauri::State<AppState>, question: String) -> Result<Value, CommandError> {
    state.service.call("ask", json!({"question": question}), &mut |t| {
        let _ = app.emit("ask-token", t.to_string());
    })
}

/// Download a GGUF model for the built-in llama.cpp and select it (`model-progress` events).
#[cfg(feature = "llama")]
#[tauri::command(async)]
fn download_model(app: tauri::AppHandle, state: tauri::State<AppState>, url: Option<String>) -> Result<Value, CommandError> {
    let path = state.service.call("download_model", json!({"url": url}), &mut |p| {
        let _ = app.emit("model-progress", p.to_string());
    })?;
    // Keep the saved settings in step with the engine.
    let mut s = load_settings(&app);
    s.config.llm.local.model_path = serde_json::from_value(path.clone()).ok();
    save_settings(&app, &s)?;
    Ok(path)
}

/// Send an email (feature `mail`): `{to, cc, bcc, subject, text, html?, attachments: [path]}`.
#[cfg(feature = "mail")]
#[tauri::command(async)]
fn send_mail(state: tauri::State<AppState>, message: Value) -> Result<Value, CommandError> {
    call(&state, "send_mail", message)
}

/// Check the mail settings: log in only, or with `to`, send a short test message.
#[cfg(feature = "mail")]
#[tauri::command(async)]
fn test_mail(state: tauri::State<AppState>, to: Option<String>) -> Result<Value, CommandError> {
    call(&state, "test_mail", json!({ "to": to }))
}

/// Save the SMTP password (empty removes it), or with none, say whether one is saved.
#[cfg(feature = "mail")]
#[tauri::command(async)]
fn mail_password(state: tauri::State<AppState>, password: Option<String>) -> Result<Value, CommandError> {
    call(&state, "mail_password", json!({ "password": password }))
}

/// The built-in plugins: name, version, methods, settings schema and whether each runs.
#[cfg(feature = "plugins")]
#[tauri::command(async)]
fn plugins_list(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "plugins", Value::Null)
}

/// Call a plugin's method (`plugin.<name>.<method>` in the backend).
#[cfg(feature = "plugins")]
#[tauri::command(async)]
fn plugin_call(state: tauri::State<AppState>, name: String, method: String, params: Option<Value>) -> Result<Value, CommandError> {
    call(&state, &format!("plugin.{name}.{method}"), params.unwrap_or(Value::Null))
}

/// rclone: installed or not, the remotes in its config, and the last sync.
#[cfg(feature = "rclone")]
#[tauri::command(async)]
fn rclone_status(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "rclone_status", Value::Null)
}

/// Replace the rclone config (kept in an owner-only file; empty removes it).
#[cfg(feature = "rclone")]
#[tauri::command(async)]
fn rclone_config(state: tauri::State<AppState>, text: String) -> Result<Value, CommandError> {
    call(&state, "rclone_config", json!({ "text": text }))
}

/// Download the pinned rclone now (`rclone-progress` events).
#[cfg(feature = "rclone")]
#[tauri::command(async)]
fn rclone_install(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, CommandError> {
    state.service.call("rclone_install", Value::Null, &mut |p| {
        let _ = app.emit("rclone-progress", p.to_string());
    })
}

/// Sync now; rclone's output arrives as `rclone-progress` events.
#[cfg(feature = "rclone")]
#[tauri::command(async)]
fn rclone_sync(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, CommandError> {
    state.service.call("rclone_sync", Value::Null, &mut |p| {
        let _ = app.emit("rclone-progress", p.to_string());
    })
}

/// Tailscale: installed or not (with the download link), this machine and what is shared.
#[cfg(feature = "tailscale")]
#[tauri::command(async)]
fn tailscale_status(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "tailscale_status", Value::Null)
}

/// Start or stop sharing the configured port with the tailnet, as the saved settings say.
#[cfg(feature = "tailscale")]
#[tauri::command(async)]
fn tailscale_apply(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "tailscale_apply", Value::Null)
}

{% if oauth %}/// Where the browser returns with `oauth.redirect = deepLink` (the app's own URL scheme).
#[cfg(feature = "oauth")]
const OAUTH_DEEP_LINK: &str = "{{project-name}}://oauth/callback";

/// Sign-in: providers, who is signed in, and the redirect in use (feature `oauth`).
#[cfg(feature = "oauth")]
#[tauri::command(async)]
fn oauth_status(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "oauth_status", Value::Null)
}

/// Sign in with `provider` in the system browser (authorization code + PKCE). With the
/// loopback redirect this returns once the browser comes back; with the deep link it returns
/// `{pending: true}` at once and an `oauth-changed` event follows. Either way
/// `oauth-changed` carries the provider's new status.
#[cfg(feature = "oauth")]
#[tauri::command(async)]
fn oauth_login(app: tauri::AppHandle, state: tauri::State<AppState>, provider: String) -> Result<Value, CommandError> {
    use desktop_core::oauth::{open_browser, Loopback, RedirectMode};
    let status = call(&state, "oauth_status", Value::Null)?;
    let redirect: RedirectMode = serde_json::from_value(status["redirect"].clone()).unwrap_or_default();
    if redirect == RedirectMode::DeepLink {
{% if tauri_plugins %}{% if tauri_deep_link %}        let begun = call(&state, "oauth_begin", json!({"provider": provider, "redirectUri": OAUTH_DEEP_LINK}))?;
        open_browser(begun["url"].as_str().unwrap_or_default()).map_err(err)?;
        return Ok(json!({"pending": true}));
{% else %}        log::warn!("the deep-link plugin is not built in; signing in through {OAUTH_DEEP_LINK:?} is not possible, using the loopback redirect");
{% endif %}{% else %}        log::warn!("the deep-link plugin is not built in; signing in through {OAUTH_DEEP_LINK:?} is not possible, using the loopback redirect");
{% endif %}    }
    // The wait for the browser happens here, not in the backend, so a slow sign-in never
    // holds up the worker process.
    let loopback = Loopback::bind().map_err(err)?;
    let begun = call(&state, "oauth_begin", json!({"provider": provider, "redirectUri": loopback.redirect_uri()}))?;
    open_browser(begun["url"].as_str().unwrap_or_default()).map_err(err)?;
    let timeout = std::time::Duration::from_secs(status["timeoutSecs"].as_u64().unwrap_or(300));
    let target = loopback.wait(begun["state"].as_str().unwrap_or_default(), timeout).map_err(err)?;
    let done = call(&state, "oauth_complete", json!({"url": target}))?;
    let _ = app.emit("oauth-changed", &done);
    Ok(done)
}

/// Sign out of `provider`: the token is revoked where the provider supports it, then
/// removed from the keychain. Returns `{revoked}`.
#[cfg(feature = "oauth")]
#[tauri::command(async)]
fn oauth_logout(app: tauri::AppHandle, state: tauri::State<AppState>, provider: String) -> Result<Value, CommandError> {
    let done = call(&state, "oauth_logout", json!({ "provider": provider }))?;
    let _ = app.emit("oauth-changed", &done);
    Ok(done)
}

/// Save a provider's client secret in the OS keychain (empty removes it), or with none, say
/// whether one is saved. Secrets never go into settings.
#[cfg(feature = "oauth")]
#[tauri::command(async)]
fn oauth_client_secret(state: tauri::State<AppState>, provider: String, secret: Option<String>) -> Result<Value, CommandError> {
    call(&state, "oauth_client_secret", json!({ "provider": provider, "secret": secret }))
}

{% endif %}{% if background_jobs %}/// The job queue: waiting and running jobs, dead letters, recent runs and the job types.
#[cfg(feature = "jobs-queue")]
#[tauri::command(async)]
fn jobs_list(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "jobs_list", Value::Null)
}

/// Add a job: `kind`, its `payload`, and optionally `options` {delaySecs, runAt, maxAttempts, timeoutSecs}.
#[cfg(feature = "jobs-queue")]
#[tauri::command(async)]
fn jobs_enqueue(state: tauri::State<AppState>, kind: String, payload: Option<Value>, options: Option<Value>) -> Result<Value, CommandError> {
    let mut params = options.filter(Value::is_object).unwrap_or_else(|| json!({}));
    params["kind"] = json!(kind);
    params["payload"] = payload.unwrap_or_else(|| json!({}));
    call(&state, "jobs_enqueue", params)
}

/// Cancel a job (a running one is told to stop).
#[cfg(feature = "jobs-queue")]
#[tauri::command(async)]
fn jobs_cancel(state: tauri::State<AppState>, id: String) -> Result<Value, CommandError> {
    call(&state, "jobs_cancel", json!({ "id": id }))
}

/// Run a dead or waiting job again now.
#[cfg(feature = "jobs-queue")]
#[tauri::command(async)]
fn jobs_retry(state: tauri::State<AppState>, id: String) -> Result<Value, CommandError> {
    call(&state, "jobs_retry", json!({ "id": id }))
}

/// The schedules (cron or interval), paused or not, with their next run.
#[cfg(feature = "jobs-queue")]
#[tauri::command(async)]
fn jobs_schedules(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "jobs_schedules", Value::Null)
}

/// Add, change, pause, resume or remove a schedule: `{id, kind?, payload?, cron? | everySecs?, paused?, remove?}`.
#[cfg(feature = "jobs-queue")]
#[tauri::command(async)]
fn jobs_set_schedule(state: tauri::State<AppState>, schedule: Value) -> Result<Value, CommandError> {
    call(&state, "jobs_set_schedule", schedule)
}

/// The worker that the background agent starts at login has no window to set it up: when
/// background jobs are on, it opens the engine from the saved settings itself, so schedules
/// keep running while the window is closed.
#[cfg(all(desktop, feature = "background-agent", feature = "jobs-queue"))]
pub(crate) fn jobs_autostart(local: &Arc<service::Local>) {
    let (config_dir, data_dir) = paths::app_dirs();
    let s = settings_at(&config_dir.join("settings.json"));
    if !s.setup_done || !s.config.jobs.enabled {
        return;
    }
    let opts = EngineOptions {
        data_dir,
        backend: store_for(s.backend),
        embeddings: s.embeddings && cfg!(feature = "embed"),
        config: s.config,
        app_name: Some(r#"{{app_name}}"#.into()),
    };
    let local = local.clone();
    let started = std::thread::Builder::new().name("jobs-autostart".into()).spawn(move || match local.setup_unless_open(&opts) {
        Ok(()) => log::info!("worker: background jobs running"),
        Err(e) => log::warn!("worker: could not start background jobs: {e}"),
    });
    if let Err(e) = started {
        log::warn!("worker: could not start background jobs: {e}");
    }
}

{% endif %}/// Read text aloud with tiny-voca: `{wav: base64, seconds}`.
#[cfg(feature = "tts")]
#[tauri::command(async)]
fn speak(state: tauri::State<AppState>, text: String) -> Result<Value, CommandError> {
    call(&state, "speak", json!({"text": text}))
}

/// Fetch the voice model (about 25 MB, checksummed) with `voice-progress` events.
#[cfg(feature = "tts")]
#[tauri::command(async)]
fn download_voice(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, CommandError> {
    state.service.call("download_voice", Value::Null, &mut |p| {
        let _ = app.emit("voice-progress", p.to_string());
    })
}

/// Speech to text with Whisper. `audio` is base64 16-bit 16 kHz mono PCM (what the UI
/// records); returns `{text, language, seconds}`.
#[cfg(feature = "stt")]
#[tauri::command(async)]
fn transcribe(state: tauri::State<AppState>, audio: String) -> Result<Value, CommandError> {
    call(&state, "transcribe", json!({"pcm16": audio}))
}

/// Fetch the chosen Whisper model (checksummed) with `stt-progress` events.
#[cfg(feature = "stt")]
#[tauri::command(async)]
fn download_stt(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Value, CommandError> {
    state.service.call("download_stt", Value::Null, &mut |p| {
        let _ = app.emit("stt-progress", p.to_string());
    })
}

/// What telemetry is on, the anonymous install id and the last send problem.
#[cfg(feature = "telemetry")]
#[tauri::command(async)]
fn telemetry_status(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "telemetry_status", Value::Null)
}

/// AI usage and cost: today, this month, recent days and months, and the budgets.
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn ai_usage(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "ai_usage", Value::Null)
}

/// Providers that have a key (the keys never leave the backend).
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn ai_keys(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "ai_keys", Value::Null)
}

/// Store a provider's key (an empty key forgets it).
#[cfg(feature = "ai")]
#[tauri::command(async)]
fn set_ai_key(state: tauri::State<AppState>, provider: String, key: Option<String>) -> Result<Value, CommandError> {
    call(&state, "set_ai_key", json!({"provider": provider, "key": key}))
}

#[tauri::command(async)]
fn remove_source(state: tauri::State<AppState>, source: String) -> Result<usize, CommandError> {
    typed(call(&state, "remove_source", json!({"source": source})))
}

#[tauri::command(async)]
fn clear(state: tauri::State<AppState>) -> Result<Status, CommandError> {
    typed(call(&state, "clear", Value::Null))
}

/// `<app> --mcp`: serve MCP over stdio (feature `mcp`), sharing the worker when there is
/// one. Returns the exit code.
#[cfg(all(desktop, feature = "mcp"))]
pub fn mcp_main() -> i32 {
    #[cfg(all(desktop, feature = "ipc-worker"))]
    let service: Arc<dyn Service> = Arc::new(ipc::Remote::default());
    #[cfg(not(all(desktop, feature = "ipc-worker")))]
    let service: Arc<dyn Service> = Arc::new(service::Local::default());
    let load = |f: &Path| std::fs::read(f).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    mcp::main(load, service)
}

/// `<app> --mcp` where there is no MCP server (feature off, or mobile): says so and returns 2.
#[cfg(not(all(desktop, feature = "mcp")))]
pub fn mcp_main() -> i32 {
    eprintln!("this build has no MCP server (feature mcp)");
    2
}

/// How to connect an MCP client to this app (the entry, and the Claude Code command).
#[cfg(all(desktop, feature = "mcp"))]
#[tauri::command(async)]
fn mcp_setup() -> Result<mcp::Setup, CommandError> {
    Ok(mcp::setup()?)
}

/// Add this app to Claude Desktop's MCP servers (the old config is kept as `.bak`).
#[cfg(all(desktop, feature = "mcp"))]
#[tauri::command(async)]
fn mcp_register_claude() -> Result<&'static str, CommandError> {
    Ok(mcp::register(&mcp::claude_desktop_config(), mcp::entry()?)?)
}

/// MCP calls waiting for approval (tools set to "ask"), oldest first.
#[cfg(feature = "mcp")]
#[tauri::command(async)]
fn mcp_pending(state: tauri::State<AppState>) -> Result<Value, CommandError> {
    call(&state, "mcp_pending", Value::Null)
}

/// Approve or refuse a waiting MCP call; returns the calls still waiting.
#[cfg(feature = "mcp")]
#[tauri::command(async)]
fn mcp_decide(state: tauri::State<AppState>, id: String, allow: bool) -> Result<Value, CommandError> {
    call(&state, "mcp_decide", json!({"id": id, "allow": allow}))
}

/// While the window runs (and the MCP server is on), tell `--mcp` processes that someone
/// can answer "ask" calls; without this, they refuse those calls at once.
#[cfg(feature = "mcp")]
fn mcp_heartbeat() {
    let spawned = std::thread::Builder::new().name("mcp-heartbeat".into()).spawn(|| loop {
        let (config_dir, data_dir) = paths::app_dirs();
        let q = desktop_core::mcp::Approvals::new(&data_dir);
        if settings_at(&config_dir.join("settings.json")).config.mcp.enabled {
            if let Err(e) = q.heartbeat() {
                log::warn!("MCP approvals heartbeat: {e}");
            }
        } else {
            q.stopped();
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    });
    if let Err(e) = spawned {
        log::warn!("MCP approvals heartbeat did not start: {e}");
    }
}

/// `<app> --worker`: serve the backend over the local socket until told to stop.
/// Returns the exit code.
#[cfg(all(desktop, feature = "ipc-worker"))]
pub fn worker_main() -> i32 {
    let (config, data) = paths::app_dirs();
    let settings = settings_at(&config.join("settings.json"));
    desktop_core::logging::init(&data.join("logs"), "worker", settings.config.log);
    desktop_core::net::configure(settings.config.net);
    ipc::serve()
}

/// `<app> --worker` where there is no worker process (feature off, or mobile): says so and returns 2.
#[cfg(not(all(desktop, feature = "ipc-worker")))]
pub fn worker_main() -> i32 {
    eprintln!("this build has no worker process (feature ipc-worker)");
    2
}

/// `<app> --daemon`: the backend with no window, until SIGTERM, ctrl-c or the Windows
/// service stop control (feature `daemon`). Opens the store with the saved settings.
#[cfg(all(desktop, feature = "daemon"))]
pub fn daemon_main(args: &[String]) -> i32 {
    let (config, data) = paths::app_dirs();
    let settings = settings_at(&config.join("settings.json"));
    desktop_core::logging::init(&data.join("logs"), "daemon", settings.config.log.clone());
    desktop_core::net::configure(settings.config.net.clone());
    let setup = settings.setup_done.then(|| {
        let opts = EngineOptions { data_dir: data.clone(), backend: store_for(settings.backend), embeddings: settings.embeddings && cfg!(feature = "embed"), config: settings.config.clone(), app_name: Some(env!("CARGO_PKG_NAME").into()) };
        serde_json::to_value(opts).unwrap_or(Value::Null)
    });
    daemon::main(args, setup)
}

#[cfg(not(all(desktop, feature = "daemon")))]
pub fn daemon_main(_args: &[String]) -> i32 {
    eprintln!("this build has no daemon mode (feature daemon)");
    2
}

/// `<app> --service <action> [--system]`: install and control the daemon as a service.
#[cfg(all(desktop, feature = "daemon"))]
pub fn service_main(args: &[String]) -> i32 {
    daemon::cli(args)
}

#[cfg(not(all(desktop, feature = "daemon")))]
pub fn service_main(_args: &[String]) -> i32 {
    eprintln!("this build has no service support (feature daemon)");
    2
}

/// `<app> --eval [dataset] [--baseline file] ...`: score search (and optionally AI answers)
/// on a labelled dataset and print a JSON report (feature `evals`). Returns the exit code:
/// 0 passed, 1 a regression against the baseline, 2 an error.
#[cfg(feature = "evals")]
pub fn eval_main(args: &[String]) -> i32 {
    eval::main(args)
}

/// `<app> --eval` where evals are not built in: says so and returns 2.
#[cfg(not(feature = "evals"))]
pub fn eval_main(_args: &[String]) -> i32 {
    eprintln!("this build has no evals (feature evals)");
    2
}

/// `<app> --pg-upgrade [--check]`: upgrade the embedded Postgres data folder of the running
/// profile to the bundled major version now, whatever `postgres.upgrade` says (`--check`
/// only runs `pg_upgrade --check`). Progress goes to stderr, the result to stdout. Returns
/// the exit code: 0 done (or nothing to do), 1 failed (the data is kept), 2 not built in.
#[cfg(feature = "postgres")]
pub fn pg_upgrade_main(args: &[String]) -> i32 {
    let check = args.iter().any(|a| a == "--check");
    let (config_dir, data_dir) = paths::app_dirs();
    let settings = settings_at(&config_dir.join("settings.json"));
    let result = desktop_core::pgstore::upgrade_now(&data_dir.join("postgres"), &settings.config.postgres, check, &mut |p| eprintln!("{p}"));
    desktop_core::pgstore::shutdown();
    match result {
        Ok(done) => {
            println!("{done}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// `<app> --pg-upgrade` where Postgres is not built in: says so and returns 2.
#[cfg(not(feature = "postgres"))]
pub fn pg_upgrade_main(_args: &[String]) -> i32 {
    eprintln!("this build has no embedded Postgres (feature postgres)");
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
    report.insert("features".into(), json!(features()));
    // Which config/ layers this build was made with (see `app_config`).
    report.insert("env".into(), json!(app_config::env()));
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
    #[cfg(all(desktop, feature = "claude-mcp"))]
    {
        // Release builds bundle uv; a dev build only finds one on PATH.
        let uv = claude::uv_version();
        ok &= uv.is_some() || cfg!(debug_assertions);
        report.insert("uv".into(), serde_json::json!(uv));
    }
    #[cfg(all(desktop, feature = "ipc-worker"))]
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
    #[cfg(all(desktop, feature = "python-example"))]
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

/// Logs and network settings for this process, from the saved settings.
fn start_logging() {
    let (config_dir, data_dir) = paths::app_dirs();
    let process = if cfg!(all(desktop, feature = "ipc-worker")) { "window" } else { "app" };
    let settings = settings_at(&config_dir.join("settings.json"));
    desktop_core::logging::init(&data_dir.join("logs"), process, settings.config.log);
    desktop_core::net::configure(settings.config.net);
}

/// The app: on desktop from `main`, on iOS and Android from the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // On iOS and Android the app's folders are known only once Tauri runs (see `setup`).
    #[cfg(desktop)]
    start_logging();
    #[cfg(all(desktop, feature = "single-instance"))]
    let instance = match single::claim(&paths::app_dirs().1) {
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
    #[cfg(all(desktop, feature = "ipc-worker"))]
    let service: Arc<dyn Service> = Arc::new(ipc::Remote::default());
    #[cfg(not(all(desktop, feature = "ipc-worker")))]
    let service: Arc<dyn Service> = Arc::new(service::Local::default());
    let builder = tauri::Builder::default().manage(AppState { service }).plugin(guard::plugin());
    // Theme and fonts before the first paint, the Settings window and its menu item.
    #[cfg(all(desktop, feature = "app-shell"))]
    let builder = builder
        .plugin(shell::plugin(&settings_at(&paths::app_dirs().0.join("settings.json")).config.appearance))
        .on_menu_event(shell::on_menu_event)
        .on_window_event(shell::on_window_event);
    // iOS and Android know the settings folder only once Tauri runs: the UI applies them.
    #[cfg(all(mobile, feature = "app-shell"))]
    let builder = builder.plugin(shell::plugin(&Default::default()));
    #[cfg(all(desktop, feature = "python-example"))]
    let builder = builder.manage(python::PythonWorker::default());
    #[cfg(all(desktop, feature = "updater"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
{% if tauri_plugins %}    // Official Tauri plugins chosen when the app was generated (see the README's table).
{% if tauri_log %}    // Frontend logs go to the app's own logger (desktop_core::logging), so no second logger.
    let builder = builder.plugin(tauri_plugin_log::Builder::new().skip_logger().build());
{% endif %}{% if tauri_dialog %}    let builder = builder.plugin(tauri_plugin_dialog::init());
{% endif %}{% if tauri_fs %}    let builder = builder.plugin(tauri_plugin_fs::init());
{% endif %}{% if tauri_clipboard %}    let builder = builder.plugin(tauri_plugin_clipboard_manager::init());
{% endif %}{% if tauri_notification %}    let builder = builder.plugin(tauri_plugin_notification::init());
{% endif %}{% if tauri_opener %}    let builder = builder.plugin(tauri_plugin_opener::init());
{% endif %}{% if tauri_store %}    let builder = builder.plugin(tauri_plugin_store::Builder::new().build());
{% endif %}{% if tauri_deep_link %}    let builder = builder.plugin(tauri_plugin_deep_link::init());
{% endif %}{% if tauri_os %}    let builder = builder.plugin(tauri_plugin_os::init());
{% endif %}{% if tauri_http %}    // Frontend fetch, limited to the URLs in capabilities/default.json.
    let builder = builder.plugin(tauri_plugin_http::init());
{% endif %}{% if tauri_positioner %}    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_positioner::init());
{% endif %}{% if tauri_shell %}    // Runs only the commands listed in capabilities/desktop.json (never arbitrary ones).
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_shell::init());
{% endif %}{% if tauri_global_shortcut %}    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
{% endif %}{% if tauri_autostart %}    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None));
{% endif %}{% if tauri_window_state %}    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());
{% endif %}{% if tauri_process %}    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_process::init());
{% endif %}{% endif %}{% if tray_menu %}    // Tray, global shortcuts and app menu: one action registry (see tray.rs).
{% unless tauri_plugins and tauri_global_shortcut %}    #[cfg(all(desktop, feature = "tray-menu"))]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
{% endunless %}    #[cfg(all(desktop, feature = "tray-menu"))]
    let builder = builder
        .manage(tray::Tray::default())
        .on_menu_event(|app, e| {
            if let Some(action) = desktop_core::actions::action_of(e.id().as_ref()) {
                tray::run(app, action);
            }
        })
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                if tray::closes_to_tray(w.app_handle()) {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        });
{% endif %}    let builder = builder.setup(move |app| {
        let _ = &app;
{% if tauri_plugins %}{% if tauri_deep_link %}        // Installers register the URL scheme; this covers development builds on Windows and Linux.
        #[cfg(any(windows, target_os = "linux"))]
        {
            use tauri_plugin_deep_link::DeepLinkExt;
            if let Err(e) = app.deep_link().register_all() {
                log::warn!("could not register the deep-link scheme: {e}");
            }
        }
{% endif %}{% endif %}{% if oauth %}{% if tauri_plugins %}{% if tauri_deep_link %}        // Sign-in replies that come back through the deep link (`oauth.redirect = deepLink`).
        #[cfg(feature = "oauth")]
        {
            use tauri_plugin_deep_link::DeepLinkExt;
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let url = url.to_string();
                    if !url.starts_with(OAUTH_DEEP_LINK) {
                        continue;
                    }
                    let handle = handle.clone();
                    std::thread::spawn(move || {
                        let Some(state) = handle.try_state::<AppState>() else { return };
                        let reply = state.service.call("oauth_complete", json!({ "url": url }), &mut |_| {}).unwrap_or_else(|e| json!({ "error": e.message }));
                        let _ = handle.emit("oauth-changed", reply);
                    });
                }
            });
        }
{% endif %}{% endif %}{% endif %}        // iOS and Android: everything lives in the app's private folder.
        #[cfg(mobile)]
        {
            paths::set_mobile_root(app.path().app_data_dir()?);
            start_logging();
        }
        #[cfg(feature = "mcp")]
        mcp_heartbeat();
        #[cfg(all(desktop, feature = "tray-menu"))]
        tray::apply(app.handle(), &load_settings(app.handle()).config);
        // Panics in the window process are written to <data>/crashes too (sent only if allowed).
        #[cfg(feature = "telemetry")]
        {
            let dir = paths::app_dirs().1;
            desktop_core::telemetry::install_panic_hook(&dir, &app.package_info().name, &app.package_info().version.to_string());
        }
        #[cfg(all(desktop, feature = "single-instance"))]
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
        // macOS: "Settings…" (Cmd+,) in the app menu.
        #[cfg(all(target_os = "macos", feature = "app-shell"))]
        if let Err(e) = shell::add_menu_item(app.handle()) {
            log::warn!("could not add Settings to the app menu: {e}");
        }
        Ok(())
    });
    builder
        .invoke_handler({% if webview %}guarded({% endif %}tauri::generate_handler![
            app_info,
            run_setup,
            ingest_text,
            ingest_file,
            search,
            ingest_folder,
            get_config,
            set_config,
            watch_status,
            list_profiles,
            create_profile,
            rename_profile,
            remove_profile,
            switch_profile,
            backup_now,
            list_backups,
            restore_backup,
            backup_recovery_key,
            #[cfg(feature = "ai")]
            ask,
            #[cfg(feature = "ai")]
            ai_keys,
            #[cfg(feature = "ai")]
            ai_usage,
            #[cfg(feature = "ai")]
            set_ai_key,
            #[cfg(feature = "llama")]
            download_model,
            #[cfg(feature = "mail")]
            send_mail,
            #[cfg(feature = "mail")]
            test_mail,
            #[cfg(feature = "mail")]
            mail_password,
            #[cfg(feature = "plugins")]
            plugins_list,
            #[cfg(feature = "plugins")]
            plugin_call,
            #[cfg(feature = "rclone")]
            rclone_status,
            #[cfg(feature = "rclone")]
            rclone_config,
            #[cfg(feature = "rclone")]
            rclone_install,
            #[cfg(feature = "rclone")]
            rclone_sync,
            #[cfg(feature = "tailscale")]
            tailscale_status,
            #[cfg(feature = "tailscale")]
            tailscale_apply,
            #[cfg(feature = "oauth")]
            oauth_status,
            #[cfg(feature = "oauth")]
            oauth_login,
            #[cfg(feature = "oauth")]
            oauth_logout,
            #[cfg(feature = "oauth")]
            oauth_client_secret,
            #[cfg(all(desktop, feature = "http-api"))]
            http::http_api_info,
            #[cfg(feature = "jobs-queue")]
            jobs_list,
            #[cfg(feature = "jobs-queue")]
            jobs_enqueue,
            #[cfg(feature = "jobs-queue")]
            jobs_cancel,
            #[cfg(feature = "jobs-queue")]
            jobs_retry,
            #[cfg(feature = "jobs-queue")]
            jobs_schedules,
            #[cfg(feature = "jobs-queue")]
            jobs_set_schedule,
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
            #[cfg(all(desktop, feature = "mcp"))]
            mcp_setup,
            #[cfg(all(desktop, feature = "mcp"))]
            mcp_register_claude,
            #[cfg(feature = "mcp")]
            mcp_pending,
            #[cfg(feature = "mcp")]
            mcp_decide,
            remove_source,
            clear,
            #[cfg(feature = "app-shell")]
            shell::get_appearance,
            #[cfg(all(desktop, feature = "app-shell"))]
            shell::open_settings,
            #[cfg(all(desktop, feature = "updater"))]
            update::check_update,
            #[cfg(all(desktop, feature = "updater"))]
            update::install_update,
            #[cfg(all(desktop, feature = "release-channels"))]
            update::update_channel_status,
            #[cfg(all(desktop, feature = "release-channels"))]
            update::rollback_update,
            #[cfg(feature = "supply-chain")]
            notices::third_party_notices,
            #[cfg(all(desktop, feature = "background-agent"))]
            agent::agent_status,
            #[cfg(all(desktop, feature = "background-agent"))]
            agent::set_agent,
            #[cfg(all(desktop, feature = "daemon"))]
            daemon::service_status,
            #[cfg(all(desktop, feature = "daemon"))]
            daemon::set_service,
            #[cfg(all(desktop, feature = "daemon"))]
            daemon::service_system_hint,
            #[cfg(all(desktop, feature = "python-example"))]
            python::python_call,
            #[cfg(all(desktop, feature = "claude-mcp"))]
            claude::register_claude,
            #[cfg(all(desktop, feature = "tray-menu"))]
            tray::tray_status,
            #[cfg(all(desktop, feature = "tray-menu"))]
            tray::run_action,
{% if webview %}            #[cfg(all(desktop, feature = "webview"))]
            webview::webview_open,
            #[cfg(all(desktop, feature = "webview"))]
            webview::webview_close,
            #[cfg(all(desktop, feature = "webview"))]
            webview::webview_post,
{% endif %}            // kitchen:handlers
        ]{% if webview %}){% endif %})
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                on_exit(app);
            }
        });
}

{% if webview %}/// Every command call goes through here: calls from embedded web views (feature `webview`)
/// are refused before any command runs.
#[cfg(all(desktop, feature = "webview"))]
fn guarded(handler: fn(tauri::ipc::Invoke) -> bool) -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    move |invoke| webview::guard_invoke(invoke, handler)
}

/// Without feature `webview` there are no web view windows to refuse.
#[cfg(not(all(desktop, feature = "webview")))]
fn guarded(handler: fn(tauri::ipc::Invoke) -> bool) -> fn(tauri::ipc::Invoke) -> bool {
    handler
}

{% endif %}/// Stop what this window started, in order: a running ingest stops after its current
/// files (the rest is journaled and resumes next time), telemetry is flushed, then the
/// worker and the database stop. A worker kept by the background agent keeps running.
fn on_exit(app: &tauri::AppHandle) {
    #[cfg(all(desktop, feature = "http-api"))]
    http::stop();
    if let Some(state) = app.try_state::<AppState>() {
        let _ = state.service.call("cancel_ingest", Value::Null, &mut |_| {});
    }
    #[cfg(feature = "telemetry")]
    desktop_core::telemetry::flush(std::time::Duration::from_secs(2));
    // MCP "ask" calls are refused at once from now on, instead of waiting for a timeout.
    #[cfg(feature = "mcp")]
    desktop_core::mcp::Approvals::new(&paths::app_dirs().1).stopped();
    #[cfg(all(desktop, feature = "ipc-worker"))]
    {
        #[cfg(all(desktop, feature = "background-agent"))]
        let keep = agent::installed();
        #[cfg(not(all(desktop, feature = "background-agent")))]
        let keep = false;
        // A daemon (feature `daemon`) runs on after the window closes.
        #[cfg(all(desktop, feature = "daemon"))]
        let keep = keep || ipc::backend_is_daemon();
        if !keep {
            ipc::shutdown_worker();
        }
    }
    #[cfg(all(feature = "postgres", not(all(desktop, feature = "ipc-worker"))))]
    desktop_core::pgstore::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_falls_back_to_the_file_store_on_mobile() {
        assert_eq!(store_for(Backend::File), Backend::File);
        let expected = if cfg!(mobile) { Backend::File } else { Backend::Postgres };
        assert_eq!(store_for(Backend::Postgres), expected);
    }

    #[test]
    fn features_say_whether_this_is_the_mobile_app() {
        assert_eq!(features().mobile, cfg!(mobile));
    }

    /// Typed IPC: writes the TypeScript types of every command and the types they use into
    /// the UI ({% if frontend == "react" %}`ui-react/src/bindings/`{% else %}`ui/bindings/`{% endif %}). With `CI` set it only compares, and fails when they
    /// are stale. Run it with the default features: `cargo test -p {{project-name}} export_bindings`.
    /// A new command that returns a new struct: derive `ts_rs::TS` on it
    /// (`#[cfg_attr(test, derive(ts_rs::TS))]`) and add it below. See docs/GUIDE.md, "Typed IPC".
    #[test]
    fn export_bindings() {
        use desktop_core::bindings::{check_mode, sync, Bindings};
        let mut b = Bindings::new();
        // The types the commands name, under the path each command's file writes them with.
        b.add::<AppInfo>("AppInfo").add::<ProfileList>("ProfileList").add::<paths::Profile>("paths::Profile");
        b.add::<Backend>("Backend").add::<Config>("Config").add::<SearchConfig>("SearchConfig");
        b.add::<Hit>("Hit").add::<IngestReport>("IngestReport").add::<Status>("Status");
        // What every command rejects with (`{code, message, details?}`).
        b.add::<error::CommandError>("error::CommandError");
{% if mcp_server %}        #[cfg(all(desktop, feature = "mcp"))]
        b.add::<mcp::Setup>("mcp::Setup");
{% endif %}{% if ipc_worker %}{% if background_agent %}        #[cfg(all(desktop, feature = "background-agent"))]
        b.add::<agent::AgentStatus>("agent::AgentStatus");
{% endif %}{% if daemon %}        #[cfg(all(desktop, feature = "daemon"))]
        b.add::<daemon::ServiceStatus>("daemon::ServiceStatus");
{% endif %}{% endif %}{% if claude_mcp %}        #[cfg(all(desktop, feature = "claude-mcp"))]
        b.add::<claude::Registration>("claude::Registration");
{% endif %}{% if tray_menu %}        #[cfg(all(desktop, feature = "tray-menu"))]
        b.add::<tray::Status>("tray::Status");
{% endif %}{% if updater %}        #[cfg(all(desktop, feature = "updater"))]
        b.add::<update::UpdateInfo>("update::UpdateInfo");
{% if release_channels %}        #[cfg(all(desktop, feature = "release-channels"))]
        b.add::<update::ChannelStatus>("update::ChannelStatus");
{% endif %}{% endif %}        // kitchen:bindings (types of generated commands go above this line)
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let module = |m: &str| {
            let base = src.join(m.replace("::", "/"));
            std::fs::read_to_string(base.with_extension("rs")).or_else(|_| std::fs::read_to_string(base.join("mod.rs"))).ok()
        };
        let lib = std::fs::read_to_string(src.join("lib.rs")).expect("read src/lib.rs");
        let commands = b.commands(&lib, &module, &cfg_holds).unwrap_or_else(|e| panic!("{e}"));
        let how = "cargo test -p {{project-name}} export_bindings";
        let files = b.render(&commands, how);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("{% if frontend == "react" %}../ui-react/src/bindings{% else %}../ui/bindings{% endif %}");
        for f in sync(&dir, &files, check_mode(), how).unwrap_or_else(|e| panic!("{e}")) {
            println!("wrote {}", f.display());
        }
    }

    /// Whether a `#[cfg(...)]` predicate in the command list holds in this build.
    fn cfg_holds(key: &str, value: Option<&str>) -> Option<bool> {
        macro_rules! features {
            ($($f:literal),* $(,)?) => {
                match value {
                    $(Some($f) => Some(cfg!(feature = $f)),)*
                    _ => None,
                }
            };
        }
        match key {
            "desktop" => Some(cfg!(desktop)),
            "mobile" => Some(cfg!(mobile)),
            "debug_assertions" => Some(cfg!(debug_assertions)),
            "target_os" => value.map(|os| os == std::env::consts::OS),
            "feature" => features!(
                "embed", "postgres", "bm25", "watch", "ai", "llama", "mcp", "tts", "stt", "mail", "oauth", "telemetry", "plugins", "jobs-queue", "rclone", "tailscale",
                "http-api", "webview", "single-instance", "pdf", "wizard", "updater", "ipc-worker", "background-agent", "daemon", "python-example", "claude-mcp",
                "tray-menu", "app-shell", "supply-chain", "release-channels", "ts",
            ),
            _ => None,
        }
    }
}
