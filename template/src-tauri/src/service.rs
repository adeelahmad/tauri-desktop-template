//! The backend as requests and replies, so it can run in this process or in a separate
//! worker process (feature `ipc-worker`) without the commands knowing which.
//!
//! Methods: `ping`, `setup` (EngineOptions), `status`, `ingest_text` {source, text},
//! `ingest_file` {path}, `ingest_folder` {path, recursive}, `search` {query, k, search?},
//! `remove_source` {source}, `clear`, `configure` {config}, `watch_status`, `interrupted`
//! (ingests a crash or quit cut short), `resume_interrupted`, `cancel_ingest`, and with
//! feature `ai`: `ask` {question} (the answer streams as progress lines), `ai_keys`,
//! `set_ai_key` {provider, key}; with feature `llama`: `download_model` {url?}; with feature `tts`: `speak` {text}
//! (a WAV as base64), `download_voice`; with feature `stt`: `transcribe` {pcm16 | wav (base64)
//! | path} (16-bit 16 kHz mono PCM, or any PCM/float WAV), `download_stt` {model?}; with feature
//! `telemetry`: `telemetry_status`; with feature `mail`: `send_mail` {to, cc, bcc, subject, text,
//! html?, attachments: \[path\]}, `test_mail` {to?}, `mail_password` {password?};
//! with feature `plugins`: `plugins` (the list with schemas) and `plugin.<name>.<method>`
//! {..} for any method a plugin offers; with feature `rclone`: `rclone_status`,
//! `rclone_config` {text?}, `rclone_install`, `rclone_sync` (rclone's lines stream as
//! progress); with feature `tailscale`: `tailscale_status`, `tailscale_apply`; with feature
//! `oauth`: `oauth_status`, `oauth_begin` {provider, redirectUri} (the sign-in URL),
//! `oauth_complete` {url} (the redirect the browser came back with), `oauth_logout`
//! {provider}, `oauth_client_secret` {provider, secret?} (kept in the OS keychain);
//! with feature `jobs-queue`: `jobs_list`, `jobs_enqueue` {kind, payload, delaySecs?, runAt?,
//! maxAttempts?, timeoutSecs?}, `jobs_cancel` {id}, `jobs_retry` {id}, `jobs_schedules`,
//! `jobs_set_schedule` {id, kind?, payload?, cron? | everySecs?, paused?, remove?};
//! with feature `tray-menu`: `action` {id} (the backend side of the app's own tray, menu and
//! shortcut actions; `null` when an action has nothing to do here);
//! with feature `kv`: `kv_get` {ns, key}, `kv_set` {ns, key, value, ttlSecs?}, `kv_delete`
//! {ns, key}, `kv_list` {ns, prefix?} (only namespaces in `kv.uiNamespaces`, since the
//! window, the HTTP API and MCP tools all call these) and `kv_info`; with feature `cache`:
//! `cache_stats`, `cache_clear`;
//! with feature `mcp`:
//! `mcp_pending` (MCP calls waiting for approval) and `mcp_decide` {id, allow} (every other call
//! is timed for opt-in traces/analytics).
//!
//! Folder watching (feature `watch`, desktop only) starts with `setup` or `configure` when
//! `config.watch.enabled`; `watch_status` returns the folders and recent events, which
//! the UI polls (the same in this process and in the worker).
//!
//! To add a backend method: add a `"name" => ...` arm to `Local::dispatch` (behind its
//! feature's `#[cfg]`), list it above, and call it from a Tauri command with
//! `call(&state, "name", json!({...}))`. Nothing else changes for the worker process.
//!
//! # Example
//!
//! A call is a method name and a JSON object; this is what goes to the service, and over the
//! socket to the worker:
//!
//! ```
//! use serde_json::json;
//!
//! let params = json!({"query": "heart", "k": 3});
//! let request = json!({"method": "search", "params": params});
//! assert_eq!(request["params"]["k"], 3);
//! // The reply is the engine's value as JSON, here a list of `desktop_core::Hit`.
//! let reply = json!([{"source": "notes.md", "page": null, "text": "The heart pumps blood.", "score": 1.2}]);
//! let hits: Vec<desktop_core::Hit> = serde_json::from_value(reply)?;
//! assert_eq!(hits[0].source, "notes.md");
//! # Ok::<(), serde_json::Error>(())
//! ```

use crate::error::CommandError;
use desktop_core::{Config, Engine, EngineOptions, SearchConfig};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The backend, wherever it runs: [`Local`] in this process, or `ipc::Remote` in the
/// worker process. Commands, the MCP server and the self-test only use this.
pub trait Service: Send + Sync {
    /// Run `method`; `progress` gets human-readable lines while it works.
    fn call(&self, method: &str, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, CommandError>;
}

/// The backend in this process.
#[derive(Default)]
pub struct Local {
    engine: Mutex<Option<Arc<Engine>>>,
    app_name: Mutex<Option<String>>,
    #[cfg(all(desktop, feature = "watch"))]
    watcher: Mutex<Option<desktop_core::watch::Watcher>>,
    /// The voice model, loaded on first use.
    #[cfg(feature = "tts")]
    voice: Mutex<Option<Arc<desktop_core::tts::Tts>>>,
    /// The Whisper model, loaded on first use (and again when the size changes).
    #[cfg(feature = "stt")]
    whisper: Mutex<Option<Arc<desktop_core::stt::Stt>>>,
    /// Sign-ins started and waiting for the browser (feature `oauth`).
    #[cfg(feature = "oauth")]
    oauth_pending: Mutex<Vec<desktop_core::oauth::Pending>>,
    /// Recent watch events, newest last (at most 100).
    events: Arc<Mutex<std::collections::VecDeque<Value>>>,
    /// One `setup` at a time (the window and a worker started at login may both ask).
    setup_lock: Mutex<()>,
    /// The background job queue, and its runtime while `jobs.enabled`.
    #[cfg(feature = "jobs-queue")]
    jobs: Mutex<Jobs>,
}

#[cfg(feature = "jobs-queue")]
#[derive(Default)]
struct Jobs {
    queue: Option<desktop_core::queue::Queue>,
    runtime: Option<desktop_core::queue::Runtime>,
}

fn arg<T: DeserializeOwned>(params: &Value, key: &str) -> Result<T, CommandError> {
    serde_json::from_value(params.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| CommandError::invalid(format!("{key}: {e}")))
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Download progress as "12 of 40 MB" (or "12 MB" when the size is unknown).
#[cfg(any(feature = "llama", feature = "tts", feature = "stt"))]
fn megabytes(done: u64, total: Option<u64>) -> String {
    match total {
        Some(t) => format!("{} of {} MB", done >> 20, t >> 20),
        None => format!("{} MB", done >> 20),
    }
}

fn to<T: serde::Serialize>(v: Result<T, desktop_core::Error>) -> Result<Value, CommandError> {
    Ok(serde_json::to_value(v?)?)
}

impl Local {
    fn engine(&self) -> Result<Arc<Engine>, CommandError> {
        self.engine.lock().map_err(err)?.clone().ok_or_else(CommandError::not_ready)
    }

    /// Start, restart or stop the folder watcher to match the engine's config.
    #[cfg(all(desktop, feature = "watch"))]
    fn rewatch(&self) -> Result<(), CommandError> {
        let engine = self.engine()?;
        let mut slot = self.watcher.lock().map_err(err)?;
        *slot = None;
        let cfg = engine.config().watch;
        if !cfg.enabled {
            return Ok(());
        }
        let name = self.app_name.lock().map_err(err)?.clone().unwrap_or_else(|| "App".into());
        let drop = desktop_core::watch::default_drop_folder(&name, engine.data_dir());
        let events = self.events.clone();
        let data_dir = engine.data_dir().to_path_buf();
        let watcher = desktop_core::watch::start(engine, &cfg, &drop, &data_dir, move |ev| {
            if let (Ok(mut q), Ok(v)) = (events.lock(), serde_json::to_value(&ev)) {
                q.push_back(v);
                while q.len() > 100 {
                    q.pop_front();
                }
            }
        })
        ?;
        *slot = Some(watcher);
        Ok(())
    }

    #[cfg(not(all(desktop, feature = "watch")))]
    fn rewatch(&self) -> Result<(), CommandError> {
        Ok(())
    }

    /// Open the job queue in the engine's data folder (once), apply the settings, and start or
    /// stop its runtime to match `jobs.enabled`.
    #[cfg(feature = "jobs-queue")]
    fn rejobs(&self) -> Result<(), CommandError> {
        use desktop_core::queue::{Queue, Runtime};
        let engine = self.engine()?;
        let cfg = engine.config();
        let mut j = self.jobs.lock().map_err(err)?;
        if j.queue.as_ref().is_none_or(|q| q.dir() != engine.data_dir().join("queue")) {
            j.runtime = None;
            j.queue = None;
            j.queue = Some(Queue::open(engine.data_dir(), desktop_core::job_types::registry(), cfg.jobs.clone())?);
        }
        let queue = j.queue.clone().ok_or("no queue")?;
        queue.set_engine(Arc::downgrade(&engine));
        queue.configure(cfg.jobs.clone(), cfg.resources.clone());
        match (cfg.jobs.enabled, j.runtime.is_some()) {
            (true, false) if queue.is_owner() => j.runtime = Some(Runtime::start(queue)?),
            (false, true) => j.runtime = None,
            _ => {}
        }
        Ok(())
    }

    #[cfg(not(feature = "jobs-queue"))]
    fn rejobs(&self) -> Result<(), CommandError> {
        Ok(())
    }

    /// Stop the job runtime and hand running jobs back to the queue (the daemon's clean
    /// stop); returns how many went back.
    #[cfg(all(desktop, feature = "daemon", feature = "jobs-queue"))]
    pub fn stop_jobs(&self, grace: std::time::Duration) -> usize {
        let queue = match self.jobs.lock() {
            Ok(mut j) => {
                j.runtime = None; // no new attempts; the dispatcher thread ends
                j.queue.clone()
            }
            Err(_) => None,
        };
        queue.map_or(0, |q| q.shutdown(grace))
    }

    #[cfg(feature = "jobs-queue")]
    fn queue(&self) -> Result<desktop_core::queue::Queue, CommandError> {
        self.jobs.lock().map_err(err)?.queue.clone().ok_or_else(CommandError::not_ready)
    }

{% if tray_menu %}    /// The backend side of the app's own tray, menu and shortcut actions, by id. Runs
    /// where the engine runs (here, or in the worker process); `null` when there is
    /// nothing to do here. The window gets the `app-action` event either way.
    #[cfg(feature = "tray-menu")]
    #[allow(clippy::match_single_binding)] // `kitchen generate menu` adds arms
    fn action(&self, id: &str) -> Result<Value, CommandError> {
        let _ = self;
        match id {
            // kitchen:action-methods (`kitchen generate menu` adds actions above this line)
            _ => Ok(Value::Null),
        }
    }

{% endif %}    fn watch_status(&self) -> Result<Value, CommandError> {
        #[cfg(all(desktop, feature = "watch"))]
        let folders = self.watcher.lock().map_err(err)?.as_ref().map(|w| w.folders.clone()).unwrap_or_default();
        #[cfg(not(all(desktop, feature = "watch")))]
        let folders: Vec<std::path::PathBuf> = Vec::new();
        let events: Vec<Value> = self.events.lock().map_err(err)?.iter().cloned().collect();
        Ok(json!({"built": cfg!(all(desktop, feature = "watch")), "folders": folders, "events": events}))
    }
}

{% if oauth %}/// Sign-in for the open profile: its settings, with tokens and client secrets in the OS
/// keychain under this app's identifier (one scope per data folder, so profiles stay apart).
#[cfg(feature = "oauth")]
fn oauth(e: &Engine) -> desktop_core::oauth::Oauth {
    use desktop_core::oauth::{scope_for, Keychain, Oauth};
    Oauth::new(e.config().oauth, Keychain::new("{{identifier}}"), &scope_for(e.data_dir()))
}

{% endif %}{% if rclone %}/// The folder cloud sync copies: the backups folder or the data folder, as configured.
#[cfg(feature = "rclone")]
fn rclone_folder(e: &Engine) -> std::path::PathBuf {
    match e.config().rclone.folder {
        desktop_core::rclone::Folder::Backups => e.backup_dir(),
        desktop_core::rclone::Folder::Data => e.data_dir().to_path_buf(),
    }
}

{% endif %}impl Service for Local {
    fn call(&self, method: &str, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, CommandError> {
        let start = std::time::Instant::now();
        let reply = self.dispatch(method, params, progress);
        match &reply {
            Ok(_) => log::debug!("{method} took {} ms", start.elapsed().as_millis()),
            Err(e) => log::warn!("{method} failed after {} ms: {e}", start.elapsed().as_millis()),
        }
        // Only the method name, the time it took and ok/error are recorded (when allowed).
        #[cfg(feature = "telemetry")]
        if !matches!(method, "ping" | "status" | "config" | "watch_status" | "telemetry_status" | "mcp_pending") {
            desktop_core::telemetry::record(method, start, reply.is_ok());
        }
        reply
    }
}

impl Local {
    /// Open the engine (`setup`), then start what runs beside it.
    fn setup(&self, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, CommandError> {
        let opts: EngineOptions = serde_json::from_value(params)?;
        let engine = Engine::open(&opts, progress)?;
        let status = engine.status()?;
        *self.engine.lock().map_err(err)? = Some(Arc::new(engine));
        *self.app_name.lock().map_err(err)? = opts.app_name.clone();
        #[cfg(feature = "telemetry")]
        if let Some(e) = self.engine.lock().map_err(err)?.as_ref() {
            desktop_core::telemetry::init(
                e.data_dir(),
                opts.app_name.as_deref().unwrap_or("app"),
                env!("CARGO_PKG_VERSION"),
                e.config().telemetry,
            );
        }
        if let Err(e) = self.rewatch() {
            progress(&format!("Folder watching is off: {e}"));
        }
        // Finish what a crash or quit interrupted, in the background.
        let e = self.engine()?;
        let weak = Arc::downgrade(&e);
        if e.config().resources.resume_interrupted && !e.interrupted().is_empty() {
            std::thread::spawn(move || {
                let r = e.resume_interrupted(&|_, _, _| {});
                log::info!("resumed ingest: {} files, {} chunks, {} errors", r.files, r.chunks, r.errors.len());
            });
        }
{% if rclone %}        // Scheduled cloud sync: checked every minute while this engine is open.
        #[cfg(feature = "rclone")]
        {
            let weak = weak.clone();
            std::thread::Builder::new()
                .name("rclone".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let Some(e) = weak.upgrade() else { break };
                    let cfg = e.config().rclone;
                    if desktop_core::rclone::due(e.data_dir(), &cfg) {
                        if let Err(err) = desktop_core::rclone::sync(e.data_dir(), &rclone_folder(&e), &cfg, &mut |l| log::debug!("rclone: {l}")) {
                            log::warn!("scheduled cloud sync failed: {err}");
                        }
                    }
                })
                ?;
        }
{% endif %}{% if oauth %}        // Access tokens are refreshed shortly before they expire, while this engine is open.
        #[cfg(feature = "oauth")]
        {
            let weak = weak.clone();
            std::thread::Builder::new()
                .name("oauth-refresh".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let Some(e) = weak.upgrade() else { break };
                    if !e.config().oauth.enabled {
                        continue;
                    }
                    for (provider, result) in oauth(&e).refresh_due() {
                        if let Err(err) = result {
                            log::warn!("could not refresh the {provider} sign-in: {err}");
                        }
                    }
                })
                ?;
        }
{% endif %}        // Scheduled backups: checked every few minutes while this engine is open.
        std::thread::Builder::new()
            .name("backups".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(300));
                let Some(e) = weak.upgrade() else { break };
                if let Some(Err(err)) = e.backup_if_due() {
                    log::warn!("scheduled backup failed: {err}");
                }
            })
            ?;
        // Background jobs: the queue opens with the engine; its runtime runs while `jobs.enabled`.
        if let Err(e) = self.rejobs() {
            progress(&format!("Background jobs are off: {e}"));
        }
        Ok(serde_json::to_value(status)?)
    }

    /// `setup` from the saved settings, unless the window already did it (the worker that
    /// the background agent starts at login has no window to ask it).
    #[cfg(all(desktop, feature = "background-agent", feature = "jobs-queue"))]
    pub fn setup_unless_open(&self, opts: &EngineOptions) -> Result<(), CommandError> {
        let _one_at_a_time = self.setup_lock.lock().map_err(err)?;
        if self.engine.lock().map_err(err)?.is_some() {
            return Ok(());
        }
        self.setup(serde_json::to_value(opts)?, &mut |p| log::debug!("setup: {p}")).map(|_| ())
    }

    fn dispatch(&self, method: &str, params: Value, progress: &mut dyn FnMut(&str)) -> Result<Value, CommandError> {
        match method {
            "ping" => Ok(json!({"pid": std::process::id(), "version": env!("CARGO_PKG_VERSION")})),
            "setup" => {
                let _one_at_a_time = self.setup_lock.lock().map_err(err)?;
                self.setup(params, progress)
            }
            "status" => match self.engine.lock().map_err(err)?.as_ref() {
                Some(e) => to(e.status()),
                None => Ok(Value::Null),
            },
            "ingest_text" => to(self.engine()?.ingest_text(&arg::<String>(&params, "source")?, &arg::<String>(&params, "text")?)),
            "ingest_file" => to(self.engine()?.ingest_file(Path::new(&arg::<String>(&params, "path")?))),
            "ingest_folder" => {
                let e = self.engine()?;
                let recursive = arg::<Option<bool>>(&params, "recursive")?.unwrap_or(true);
                let files = e.files_in(Path::new(&arg::<String>(&params, "path")?), recursive);
                progress(&format!("Ingesting {} files with {} workers", files.len(), e.config().workers()));
                to(Ok(e.ingest_paths(&files, &|_, _, _| {})))
            }
            "search" => {
                let e = self.engine()?;
                let query = arg::<String>(&params, "query")?;
                let mut cfg = arg::<Option<SearchConfig>>(&params, "search")?.unwrap_or_else(|| e.config().search);
                if let Some(k) = arg::<Option<usize>>(&params, "k")? {
                    cfg.top_k = k;
                }
                to(e.search_with(&query, &cfg))
            }
            "configure" => {
                let e = self.engine()?;
                e.configure(arg::<Config>(&params, "config")?)?;
                self.rewatch()?;
                self.rejobs()?;
                desktop_core::logging::configure(e.config().log);
                #[cfg(feature = "telemetry")]
                desktop_core::telemetry::configure(e.config().telemetry);
                to(Ok(e.config()))
            }
            "config" => to(Ok(self.engine()?.config())),
            "watch_status" => self.watch_status(),
            "interrupted" => to(Ok(self.engine()?.interrupted())),
            "resume_interrupted" => to(Ok(self.engine()?.resume_interrupted(&|_, _, _| {}))),
            "cancel_ingest" => {
                if let Some(e) = self.engine.lock().map_err(err)?.as_ref() {
                    e.cancel_ingest();
                }
                Ok(Value::Null)
            }
            "backup" => to(self.engine()?.backup()),
            "backups" => to(Ok(self.engine()?.backups())),
            "backup_key" => Ok(json!({ "key": self.engine()?.backup_key()?.to_text() })),
            "restore" => {
                let e = self.engine()?;
                e.restore(Path::new(&arg::<String>(&params, "path")?), arg::<Option<String>>(&params, "key")?.as_deref())?;
                to(e.status())
            }
            "remove_source" => to(self.engine()?.remove_source(&arg::<String>(&params, "source")?)),
            "clear" => {
                let e = self.engine()?;
                e.clear()?;
                to(e.status())
            }
            #[cfg(feature = "ai")]
            "ask" => {
                let e = self.engine()?;
                let keys = desktop_core::llm::KeyFile::new(e.data_dir().join("keys.json"));
                let lookup = |p| keys.get(p);
                to(e.ask(&arg::<String>(&params, "question")?, &lookup, &mut |t| progress(t)))
            }
            #[cfg(feature = "ai")]
            "ai_usage" => to(Ok(self.engine()?.ai_usage())),
            #[cfg(feature = "ai")]
            "ai_keys" => to(Ok(desktop_core::llm::KeyFile::new(self.engine()?.data_dir().join("keys.json")).present())),
            #[cfg(feature = "ai")]
            "set_ai_key" => {
                let keys = desktop_core::llm::KeyFile::new(self.engine()?.data_dir().join("keys.json"));
                keys.set(arg(&params, "provider")?, arg::<Option<String>>(&params, "key")?.as_deref())?;
                to(Ok(keys.present()))
            }
            #[cfg(feature = "llama")]
            "download_model" => {
                let url = arg::<Option<String>>(&params, "url")?;
                let path = self.engine()?.download_model(url.as_deref(), &mut |done, total| {
                    progress(&megabytes(done, total))
                });
                to(path)
            }
            #[cfg(feature = "mail")]
            "send_mail" => {
                use desktop_core::mail;
                let e = self.engine()?;
                let mut msg: mail::Message = serde_json::from_value(params.clone())?;
                for path in arg::<Option<Vec<std::path::PathBuf>>>(&params, "attachments")?.unwrap_or_default() {
                    msg.attachments.push(mail::Attachment::from_path(&path)?);
                }
                let password = mail::PasswordFile::new(e.data_dir()).get();
                Ok(json!(mail::send(&e.config().mail, password.as_deref(), &msg)?))
            }
            #[cfg(feature = "mail")]
            "test_mail" => {
                use desktop_core::mail;
                let e = self.engine()?;
                let cfg = e.config().mail;
                let password = mail::PasswordFile::new(e.data_dir()).get();
                // With `to`, send a short message there; otherwise only connect and log in.
                match arg::<Option<String>>(&params, "to")? {
                    Some(to) => {
                        let msg = mail::Message { to: vec![to], subject: "Test email".into(), text: "Email from this app works.".into(), ..Default::default() };
                        Ok(json!(mail::send(&cfg, password.as_deref(), &msg)?))
                    }
                    None => Ok(mail::test(&cfg, password.as_deref()).map(|()| json!(true))?),
                }
            }
            #[cfg(feature = "mail")]
            "mail_password" => {
                let file = desktop_core::mail::PasswordFile::new(self.engine()?.data_dir());
                if let Some(p) = arg::<Option<String>>(&params, "password")? {
                    file.set(&p)?;
                }
                Ok(json!({ "set": file.get().is_some() }))
            }
            #[cfg(feature = "tts")]
            "speak" => {
                use desktop_core::tts;
                let e = self.engine()?;
                let cfg = e.config().tts;
                if !cfg.enabled {
                    return Err(CommandError::unavailable("Speech is turned off in Settings"));
                }
                let voice = {
                    let mut v = self.voice.lock().map_err(err)?;
                    if v.is_none() {
                        let dir = tts::locate(e.data_dir())?;
                        if !dir.join("model.onnx").is_file() {
                            return Err(CommandError::unavailable("Download the voice first (Settings → Speech)"));
                        }
                        *v = Some(Arc::new(tts::Tts::load(&dir, cfg.threads)?));
                    }
                    v.clone().ok_or("no voice")?
                };
                let audio = voice.speak(&arg::<String>(&params, "text")?, &cfg)?;
                Ok(
                    json!({"wav": desktop_core::audio::base64(&desktop_core::audio::wav(&audio, tts::SAMPLE_RATE)), "seconds": audio.len() as f32 / tts::SAMPLE_RATE as f32}),
                )
            }
            #[cfg(feature = "tts")]
            "download_voice" => {
                let dir = desktop_core::tts::locate(self.engine()?.data_dir())?;
                desktop_core::tts::download(&dir, &mut |name, done, total| {
                    progress(&format!("{name}: {}", megabytes(done, total)))
                })
                ?;
                Ok(json!(dir))
            }
            #[cfg(feature = "stt")]
            "transcribe" => {
                use desktop_core::{audio, stt};
                let e = self.engine()?;
                let cfg = e.config().stt;
                if !cfg.enabled {
                    return Err(CommandError::unavailable("Speech to text is turned off in Settings"));
                }
                let text = |k: &str| params.get(k).and_then(Value::as_str);
                let samples = if let Some(b) = text("pcm16") {
                    audio::from_pcm16(&audio::base64_decode(b)?)
                } else {
                    let bytes = match (text("wav"), text("path")) {
                        (Some(b), _) => audio::base64_decode(b)?,
                        (None, Some(p)) => desktop_core::untrusted::read_capped(p.as_ref(), desktop_core::untrusted::limits::WAV_BYTES)?,
                        _ => return Err(CommandError::invalid("send pcm16, wav or path")),
                    };
                    let (s, rate) = audio::read_wav(&bytes)?;
                    // Check the length before resampling, which allocates in proportion to it.
                    if s.len() / rate as usize > cfg.max_seconds as usize {
                        return Err(CommandError::invalid(format!("Recordings longer than {} s are turned off in Settings", cfg.max_seconds)));
                    }
                    audio::resample(&s, rate, stt::SAMPLE_RATE)
                };
                if samples.len() > cfg.max_seconds as usize * stt::SAMPLE_RATE as usize {
                    return Err(CommandError::invalid(format!("Recordings longer than {} s are turned off in Settings", cfg.max_seconds)));
                }
                let model = {
                    let mut m = self.whisper.lock().map_err(err)?;
                    if m.as_ref().is_none_or(|m| m.model != cfg.model) {
                        *m = None;
                        let dir = stt::locate(e.data_dir(), &cfg.model)?;
                        if !stt::is_downloaded(&dir) {
                            return Err(CommandError::unavailable(format!("Download the {} model first (Settings → Dictation)", cfg.model)));
                        }
                        *m = Some(Arc::new(stt::Stt::load(&dir, &cfg.model, cfg.threads)?));
                    }
                    m.clone().ok_or("no model")?
                };
                to(model.transcribe(&samples, &cfg))
            }
            #[cfg(feature = "stt")]
            "download_stt" => {
                let e = self.engine()?;
                let model = params.get("model").and_then(Value::as_str).map(String::from).unwrap_or(e.config().stt.model);
                let dir = desktop_core::stt::locate(e.data_dir(), &model)?;
                desktop_core::stt::download(&dir, &model, &mut |name, done, total| {
                    progress(&format!("{name}: {}", megabytes(done, total)))
                })
                ?;
                Ok(json!(dir))
            }
            #[cfg(feature = "mcp")]
            "mcp_pending" => to(Ok(desktop_core::mcp::Approvals::new(self.engine()?.data_dir()).pending())),
            #[cfg(feature = "mcp")]
            "mcp_decide" => {
                let q = desktop_core::mcp::Approvals::new(self.engine()?.data_dir());
                q.decide(&arg::<String>(&params, "id")?, arg(&params, "allow")?)?;
                to(Ok(q.pending()))
            }
            #[cfg(feature = "telemetry")]
            "telemetry_status" => Ok(desktop_core::telemetry::status()),
{% if plugins %}            #[cfg(feature = "plugins")]
            "plugins" => {
                let e = self.engine()?;
                to(Ok(e.plugins().list(&e.config().plugins)))
            }
            #[cfg(feature = "plugins")]
            other if other.starts_with("plugin.") => Ok(self.engine()?.plugin_call(other, &params)?),
{% endif %}{% if rclone %}            #[cfg(feature = "rclone")]
            "rclone_status" => to(Ok(desktop_core::rclone::status(self.engine()?.data_dir()))),
            #[cfg(feature = "rclone")]
            "rclone_config" => {
                let e = self.engine()?;
                if let Some(text) = arg::<Option<String>>(&params, "text")? {
                    desktop_core::rclone::ConfigFile::new(e.data_dir()).set(&text)?;
                }
                to(Ok(desktop_core::rclone::status(e.data_dir())))
            }
            #[cfg(feature = "rclone")]
            "rclone_install" => {
                let e = self.engine()?;
                let path = desktop_core::rclone::install(e.data_dir(), &mut |done, total| {
                    progress(&match total {
                        Some(t) => format!("{} of {} MB", done >> 20, t >> 20),
                        None => format!("{} MB", done >> 20),
                    })
                })
                ?;
                Ok(json!(path))
            }
            #[cfg(feature = "rclone")]
            "rclone_sync" => {
                let e = self.engine()?;
                let cfg = e.config().rclone;
                if !cfg.enabled {
                    return Err(CommandError::unavailable("Cloud sync is turned off in Settings"));
                }
                to(Ok(desktop_core::rclone::sync(e.data_dir(), &rclone_folder(&e), &cfg, progress)?))
            }
{% endif %}{% if tailscale %}            #[cfg(feature = "tailscale")]
            "tailscale_status" => to(Ok(desktop_core::tailscale::status(&self.engine()?.config().tailscale))),
            #[cfg(feature = "tailscale")]
            "tailscale_apply" => to(Ok(desktop_core::tailscale::apply(&self.engine()?.config().tailscale)?)),
{% endif %}{% if oauth %}            #[cfg(feature = "oauth")]
            "oauth_status" => {
                let e = self.engine()?;
                let cfg = e.config().oauth;
                Ok(json!({"enabled": cfg.enabled, "redirect": cfg.redirect, "timeoutSecs": cfg.timeout_secs, "providers": oauth(&e).status()}))
            }
            #[cfg(feature = "oauth")]
            "oauth_begin" => {
                let e = self.engine()?;
                let o = oauth(&e);
                let (url, pending) = o.begin(&arg::<String>(&params, "provider")?, &arg::<String>(&params, "redirectUri")?)?;
                let state = pending.state.clone();
                let timeout = std::time::Duration::from_secs(o.config().timeout_secs);
                let mut list = self.oauth_pending.lock().map_err(err)?;
                list.retain(|p| p.age() < timeout);
                list.push(pending);
                Ok(json!({"url": url, "state": state}))
            }
            #[cfg(feature = "oauth")]
            "oauth_complete" => {
                let url = arg::<String>(&params, "url")?;
                let state = desktop_core::oauth::callback_state(&url).ok_or("the sign-in reply has no state")?;
                let pending = {
                    let mut list = self.oauth_pending.lock().map_err(err)?;
                    let i = list.iter().position(|p| p.state == state).ok_or("no sign-in in progress for this reply; start again")?;
                    list.remove(i)
                };
                let e = self.engine()?;
                to(Ok(oauth(&e).complete(&pending, &url)?))
            }
            #[cfg(feature = "oauth")]
            "oauth_logout" => {
                let e = self.engine()?;
                Ok(json!({"revoked": oauth(&e).logout(&arg::<String>(&params, "provider")?)?}))
            }
            #[cfg(feature = "oauth")]
            "oauth_client_secret" => {
                let e = self.engine()?;
                let o = oauth(&e);
                let provider = arg::<String>(&params, "provider")?;
                o.provider(&provider)?;
                if let Some(secret) = arg::<Option<String>>(&params, "secret")? {
                    o.set_client_secret(&provider, Some(&secret))?;
                }
                Ok(json!({"set": o.client_secret(&provider)?.is_some()}))
            }
{% endif %}{% if background_jobs %}            #[cfg(feature = "jobs-queue")]
            "jobs_list" => to(Ok(self.queue()?.list())),
            #[cfg(feature = "jobs-queue")]
            "jobs_enqueue" => {
                let opts: desktop_core::queue::Enqueue = serde_json::from_value(params.clone())?;
                let payload = params.get("payload").cloned().filter(|p| !p.is_null()).unwrap_or_else(|| json!({}));
                to(self.queue()?.enqueue(&arg::<String>(&params, "kind")?, payload, opts))
            }
            #[cfg(feature = "jobs-queue")]
            "jobs_cancel" => to(self.queue()?.cancel(&arg::<String>(&params, "id")?)),
            #[cfg(feature = "jobs-queue")]
            "jobs_retry" => to(self.queue()?.retry(&arg::<String>(&params, "id")?)),
            #[cfg(feature = "jobs-queue")]
            "jobs_schedules" => to(Ok(self.queue()?.schedules())),
            #[cfg(feature = "jobs-queue")]
            "jobs_set_schedule" => to(self.queue()?.set_schedule(serde_json::from_value(params)?)),
{% endif %}{% if kv %}            #[cfg(feature = "kv")]
            "kv_get" => {
                let (ns, key) = (arg::<String>(&params, "ns")?, arg::<String>(&params, "key")?);
                to(self.engine()?.kv_exposed(&ns).and_then(|ns| ns.get_json::<Value>(&key)))
            }
            #[cfg(feature = "kv")]
            "kv_set" => {
                let ns = self.engine()?.kv_exposed(&arg::<String>(&params, "ns")?)?;
                let (key, value) = (arg::<String>(&params, "key")?, arg::<Value>(&params, "value")?);
                match arg::<Option<u64>>(&params, "ttlSecs")?.filter(|s| *s > 0) {
                    Some(secs) => ns.set_json_ttl(&key, &value, std::time::Duration::from_secs(secs))?,
                    None => ns.set_json(&key, &value)?,
                }
                Ok(Value::Null)
            }
            #[cfg(feature = "kv")]
            "kv_delete" => {
                let (ns, key) = (arg::<String>(&params, "ns")?, arg::<String>(&params, "key")?);
                to(self.engine()?.kv_exposed(&ns).and_then(|ns| ns.delete(&key)))
            }
            #[cfg(feature = "kv")]
            "kv_list" => {
                let prefix = arg::<Option<String>>(&params, "prefix")?.unwrap_or_default();
                to(self.engine()?.kv_exposed(&arg::<String>(&params, "ns")?).and_then(|ns| ns.entries(&prefix)))
            }
            #[cfg(feature = "kv")]
            "kv_info" => to(self.engine()?.kv_info()),
{% endif %}{% if cache %}            #[cfg(feature = "cache")]
            "cache_stats" => to(Ok(self.engine()?.cache().stats())),
            #[cfg(feature = "cache")]
            "cache_clear" => {
                let cache = self.engine()?.cache();
                cache.clear();
                to(Ok(cache.stats()))
            }
{% endif %}{% if tray_menu %}            #[cfg(feature = "tray-menu")]
            "action" => self.action(&arg::<String>(&params, "id")?),
{% endif %}            // kitchen:methods (`kitchen generate command` adds methods above this line)
            other => Err(CommandError::invalid(format!("unknown method {other:?}"))),
        }
    }
}
