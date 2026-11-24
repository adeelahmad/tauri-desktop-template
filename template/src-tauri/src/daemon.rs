//! The app as a daemon: `<app> --daemon` runs the backend with no window, and
//! `<app> --service ...` installs it as a service of the operating system.
//!
//! ```text
//! my-app --daemon                       # run headless in this terminal (ctrl-c stops it)
//! my-app --service install              # a user service: starts at login, restarts on failure
//! my-app --service status               # installed? running? pid, log file
//! my-app --service logs -n 100 --follow # tail the daemon's log
//! my-app --service stop | start | restart | uninstall
//! my-app --service install --system     # at boot, for the whole machine (prints the sudo
//!                                       # or administrator command when it needs one)
//! ```
//!
//! ## The daemon
//!
//! `--daemon` serves the same local socket as the worker process (feature `ipc-worker`),
//! so the window and the MCP server (`--mcp`) use the daemon when it runs. It opens the
//! store with the saved settings, starts the [`Headless`] tasks (the job workers and the
//! scheduler of feature `jobs-queue`, when built in), writes the JSON-lines log
//! `<data>/logs/daemon.log` and the pid and health file `<data>/daemon.json`
//! ([`desktop_core::daemon::Health`]), and stops cleanly on SIGTERM, ctrl-c (SIGINT) or the
//! Windows service stop control: tasks stop, running ingests are cancelled, Postgres shuts
//! down and the health file is removed. A health file left behind means the last run
//! crashed; after several crashes in a row the next start waits longer and longer
//! (2 s, 4 s ... 5 minutes), on every platform.
//!
//! When a window asks it to `shutdown` (after an update, so the new binary takes over) it
//! exits with code 3, which the service manager treats as a failure and restarts it from
//! the new binary; SIGTERM, `--service stop` and the Windows stop control exit with 0, so it
//! stays stopped.
//!
//! ## The service
//!
//! | | user level (default) | `--system` (administrator) |
//! |---|---|---|
//! | macOS | LaunchAgent, `~/Library/LaunchAgents/<label>.plist` | LaunchDaemon, `/Library/LaunchDaemons/<label>.plist` |
//! | Linux | `systemctl --user` unit, `~/.config/systemd/user/<label>.service` | system unit, `/etc/systemd/system/<label>.service` (OpenRC elsewhere) |
//! | Windows | a Run entry in `HKCU\...\Run` (starts at login, no restart) | a Windows Service (SCM), restart 5 s / 30 s / 2 min |
//!
//! The label is `<identifier>.daemon` (`<identifier>.<profile>.daemon` for another profile).
//! launchd and systemd restart the daemon only after a failure (`KeepAlive` /
//! `SuccessfulExit` false; `Restart=on-failure` with growing `RestartSteps`), and send its
//! stdout and stderr to `<data>/logs/daemon.out.log`. A system service on macOS and Linux
//! runs as the person who ran `sudo` (`SUDO_USER`), so it uses their data folder and their
//! window reaches it. The Windows Service runs as LocalSystem: it has its own data folder
//! and the signed-in user's window does not talk to it, so it suits machine-wide headless
//! work. The app never asks for a password itself: without the rights it prints the
//! command to run (see [`desktop_core::daemon::elevated_command`]).
//!
//! ### Why `service-manager`
//!
//! Installing, starting, stopping and querying go through the
//! [`service-manager`](https://crates.io/crates/service-manager) crate (default features
//! off) rather than one hand-written adapter per manager: it speaks launchd, systemd,
//! OpenRC, rc.d and the Windows SCM, is pure Rust with no build script, and is only built
//! with this feature. It costs about ten small crates (`dirs`, `which`, `xml-rs` and a
//! few older duplicates such as `rustix` 0.38), none of them C. Its own service file
//! templates have no growing backoff and no log paths, so on launchd and systemd we hand
//! it ours ([`desktop_core::daemon::launchd_plist`], [`desktop_core::daemon::systemd_unit`]),
//! and on Windows we add the restart actions with `sc failure`. Windows has no per-user
//! service manager, so the user level there is a Run entry, like the background agent.
//! The Windows Service entry point uses `windows-service` (Windows only, on `windows-sys`)
//! and SIGTERM / ctrl-c use `signal-hook` (a flag set from the handler, nothing else).
//!
//! ## The background agent
//!
//! Feature `background-agent` starts a plain `--worker` at login. The daemon does the same
//! and more (restart on failure, logs, health, headless jobs, system level), so installing
//! the user service removes the background agent; use one or the other. With `daemon` off
//! nothing changes.
//!
//! ## Headless tasks
//!
//! Anything that should run while no window is open implements [`Headless`] and is listed
//! in [`headless_tasks`]. The daemon starts each with a [`Context`] (the backend, the
//! folders, a stop flag) and stops them in reverse order when it stops.

use crate::paths;
use crate::service::{Local, Service};
use desktop_core::daemon::{backoff, elevated_command, now_secs, Health, Level, RunAs, ServiceSpec, HEARTBEAT_SECS, STABLE_SECS};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const ID: &str = env!("APP_IDENTIFIER");

/// The service's name: the launchd label, the systemd unit (plus `.service`), the Windows
/// service or Run entry.
pub fn label() -> String {
    match paths::profile() {
        paths::DEFAULT => format!("{ID}.daemon"),
        p => format!("{ID}.{p}.daemon"),
    }
}

// ---------------------------------------------------------------------------------------
// Headless tasks
// ---------------------------------------------------------------------------------------

/// What a [`Headless`] task gets when the daemon starts it.
// For the tasks; the built-in ones need only some of it.
#[allow(dead_code)]
pub struct Context {
    /// The backend the daemon serves (the same one the window and MCP clients reach).
    pub local: Arc<Local>,
    /// The profile's config folder (`settings.json`).
    pub config_dir: PathBuf,
    /// The profile's data folder.
    pub data_dir: PathBuf,
    /// Set when the daemon stops; long loops should check it.
    pub stopping: Arc<AtomicBool>,
}

/// Work the daemon runs while no window is open: job workers, a scheduler, a sync loop ...
///
/// `start` should spawn its threads and return; `stop` is called once, in reverse start
/// order, when the daemon stops, and should return within a few seconds (hand unfinished
/// work back to its queue rather than finish it).
pub trait Headless: Send {
    /// A short name for the log and the health file.
    fn name(&self) -> &str;
    /// Start. An error is logged and the daemon runs on without this task.
    fn start(&mut self, ctx: &Context) -> Result<(), String>;
    /// Stop (the default does nothing).
    fn stop(&mut self) {}
}

/// The tasks this build runs headless: the job queue's workers and scheduler (feature
/// `jobs-queue`), and whatever you add.
#[allow(clippy::vec_init_then_push)] // the list depends on the features
pub fn headless_tasks() -> Vec<Box<dyn Headless>> {
    #[allow(unused_mut)]
    let mut tasks: Vec<Box<dyn Headless>> = Vec::new();
    #[cfg(feature = "jobs-queue")]
    tasks.push(Box::new(JobsTask::default()));
    // kitchen:headless (add `tasks.push(Box::new(...))` lines above)
    tasks
}

/// The background job queue under the daemon (feature `jobs-queue`). Its workers and
/// scheduler run inside the backend as soon as the store is open and `jobs.enabled` is
/// on in Settings (the daemon opens the store at start); stopping stops the dispatcher and
/// hands running jobs back to the queue, their attempt not counted, so the next start
/// runs them again.
#[cfg(feature = "jobs-queue")]
#[derive(Default)]
pub struct JobsTask {
    local: Option<Arc<Local>>,
}

#[cfg(feature = "jobs-queue")]
impl Headless for JobsTask {
    fn name(&self) -> &str {
        "jobs"
    }

    fn start(&mut self, ctx: &Context) -> Result<(), String> {
        self.local = Some(ctx.local.clone());
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(local) = self.local.take() {
            let back = local.stop_jobs(Duration::from_secs(10));
            if back > 0 {
                log::info!("daemon: handed {back} running job(s) back to the queue");
            }
        }
    }
}

/// Start each task; the ones that failed are logged and left out.
fn start_tasks(tasks: Vec<Box<dyn Headless>>, ctx: &Context) -> Vec<Box<dyn Headless>> {
    let mut started = Vec::new();
    for mut t in tasks {
        match t.start(ctx) {
            Ok(()) => {
                log::info!("daemon: started {}", t.name());
                started.push(t);
            }
            Err(e) => log::error!("daemon: {} did not start: {e}", t.name()),
        }
    }
    started
}

/// Stop the tasks, last started first.
fn stop_tasks(tasks: &mut Vec<Box<dyn Headless>>) {
    while let Some(mut t) = tasks.pop() {
        t.stop();
        log::info!("daemon: stopped {}", t.name());
    }
}

// ---------------------------------------------------------------------------------------
// The daemon process
// ---------------------------------------------------------------------------------------

/// Exit code when a window asked the daemon to stop so a new binary takes over: the
/// service manager sees a failure and starts it again.
pub const EXIT_RESTART: i32 = 3;

fn stop_flag() -> &'static Arc<AtomicBool> {
    static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    STOP.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

static RESTART: AtomicBool = AtomicBool::new(false);
static SETUP: OnceLock<Option<Value>> = OnceLock::new();

/// Ask the daemon to stop (`restart`: exit with [`EXIT_RESTART`]).
pub fn request_stop(restart: bool) {
    if restart {
        RESTART.store(true, Ordering::SeqCst);
    }
    stop_flag().store(true, Ordering::SeqCst);
}

fn stopping() -> bool {
    stop_flag().load(Ordering::SeqCst)
}

/// `<app> --daemon [--windows-service]`. `setup` opens the store at start (the saved
/// settings, when the first-run setup is done). Returns the exit code.
pub fn main(args: &[String], setup: Option<Value>) -> i32 {
    let _ = SETUP.set(setup);
    #[cfg(windows)]
    if args.iter().any(|a| a == "--windows-service") {
        return windows::dispatch();
    }
    let _ = args;
    for sig in signals() {
        if let Err(e) = signal_hook::flag::register(sig, stop_flag().clone()) {
            log::warn!("daemon: cannot watch signal {sig}: {e}");
        }
    }
    run()
}

#[cfg(unix)]
fn signals() -> [i32; 3] {
    use signal_hook::consts::{SIGINT, SIGQUIT, SIGTERM};
    [SIGTERM, SIGINT, SIGQUIT]
}

#[cfg(not(unix))]
fn signals() -> [i32; 2] {
    use signal_hook::consts::{SIGINT, SIGTERM};
    [SIGTERM, SIGINT]
}

/// Sleep for `d` or until asked to stop; true when asked to stop.
fn sleep_unless_stopped(d: Duration) -> bool {
    let until = Instant::now() + d;
    while Instant::now() < until {
        if stopping() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100).min(until.saturating_duration_since(Instant::now())));
    }
    stopping()
}

fn ipc_shutdown() {
    log::info!("daemon: a window asked it to stop so a new version takes over");
    request_stop(true);
}

/// Run until asked to stop. Logging must already be set up.
fn run() -> i32 {
    let (config_dir, data_dir) = paths::app_dirs();
    let health_file = Health::file(&data_dir);
    // Another worker or daemon already serves this profile: nothing to do.
    if crate::ipc::request("ping", Value::Null, false, &mut |_| {}).is_ok() {
        log::info!("daemon: a worker or daemon already runs for this profile");
        return 0;
    }
    let left_over = Health::read(&health_file);
    let mut failures = Health::next_failures(left_over.as_ref(), now_secs());
    let wait = backoff(failures);
    if failures > 0 {
        log::warn!("daemon: the last run stopped unexpectedly ({failures} in a row); starting in {}s", wait.as_secs());
    }
    if sleep_unless_stopped(wait) {
        return 0;
    }
    let listener = match crate::ipc::bind() {
        Ok(Some(l)) => l,
        Ok(None) => {
            log::info!("daemon: a worker or daemon already runs for this profile");
            return 0;
        }
        Err(e) => {
            log::error!("daemon: {e}");
            return 1;
        }
    };
    crate::ipc::set_daemon();
    let local = Arc::new(Local::default());
    if let Some(Some(setup)) = SETUP.get().cloned() {
        let local = local.clone();
        std::thread::spawn(move || match local.call("setup", setup, &mut |p| log::debug!("daemon setup: {p}")) {
            Ok(_) => log::info!("daemon: store open"),
            Err(e) => log::error!("daemon: could not open the store: {e}"),
        });
    }
    let ctx = Context { local: local.clone(), config_dir, data_dir, stopping: stop_flag().clone() };
    let mut tasks = start_tasks(headless_tasks(), &ctx);
    let started = now_secs();
    let mut health = Health {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").into(),
        profile: paths::profile().into(),
        started,
        heartbeat: started,
        failures,
        tasks: tasks.iter().map(|t| t.name().to_string()).collect(),
    };
    if let Err(e) = health.write(&health_file) {
        log::warn!("daemon: cannot write {}: {e}", health_file.display());
    }
    {
        let local = local.clone();
        std::thread::Builder::new()
            .name("daemon-ipc".into())
            .spawn(move || crate::ipc::serve_on(listener, local, ipc_shutdown))
            .map_err(|e| log::error!("daemon: {e}"))
            .ok();
    }
    log::info!("daemon: running as pid {} (profile {})", health.pid, health.profile);
    while !sleep_unless_stopped(Duration::from_secs(HEARTBEAT_SECS)) {
        health.heartbeat = now_secs();
        if failures > 0 && health.heartbeat - started >= STABLE_SECS {
            failures = 0;
            health.failures = 0;
        }
        let _ = health.write(&health_file);
    }
    log::info!("daemon: stopping");
    stop_tasks(&mut tasks);
    let _ = local.call("cancel_ingest", Value::Null, &mut |_| {});
    #[cfg(feature = "postgres")]
    desktop_core::pgstore::shutdown();
    let _ = std::fs::remove_file(&health_file);
    log::info!("daemon: stopped");
    if RESTART.load(Ordering::SeqCst) {
        EXIT_RESTART
    } else {
        0
    }
}

/// The Windows Service entry point: SCM starts `<app> --daemon --windows-service`.
#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::time::Duration;
    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    define_windows_service!(ffi_service_main, service_main);

    pub fn dispatch() -> i32 {
        match service_dispatcher::start(super::label(), ffi_service_main) {
            Ok(()) => 0,
            Err(e) => {
                log::error!("daemon: not started by the service control manager: {e}");
                1
            }
        }
    }

    fn status(state: ServiceState, code: u32) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(30),
            process_id: None,
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let handler = |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                super::request_stop(false);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let handle = match service_control_handler::register(super::label(), handler) {
            Ok(h) => h,
            Err(e) => {
                log::error!("daemon: {e}");
                return;
            }
        };
        let _ = handle.set_service_status(status(ServiceState::Running, 0));
        let code = super::run();
        let _ = handle.set_service_status(status(ServiceState::Stopped, code as u32));
    }
}

// ---------------------------------------------------------------------------------------
// Installing and controlling the service
// ---------------------------------------------------------------------------------------

/// What `--service status` reports (and the Settings page shows).
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ServiceStatus {
    /// `user` or `system`.
    pub level: Level,
    /// The service manager, e.g. "launchd LaunchAgent".
    pub manager: String,
    /// The service file (or registry entry).
    pub location: String,
    /// Whether the service is installed.
    pub installed: bool,
    /// Whether the daemon runs.
    pub running: bool,
    /// The daemon's process id, when it runs.
    pub pid: Option<u32>,
    /// The daemon's JSON-lines log.
    pub log: String,
    /// The pid and health file, when the daemon runs (or crashed).
    pub health: Option<Health>,
    /// Anything worth knowing (e.g. the command to run as administrator).
    pub note: Option<String>,
}

fn exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))
}

/// Whether this process has administrator rights.
fn is_admin() -> bool {
    if cfg!(windows) {
        // `net session` needs an elevated token.
        Command::new("net").arg("session").output().is_ok_and(|o| o.status.success())
    } else {
        Command::new("id").arg("-u").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
    }
}

/// The person a system-level service runs as: whoever ran `sudo` (macOS and Linux).
fn run_as() -> Option<RunAs> {
    if cfg!(windows) {
        return None;
    }
    let user = std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty() && u != "root")?;
    let home = user_home(&user)?;
    Some(RunAs { user, home })
}

/// A user's home folder: from `/etc/passwd` on Linux, `dscl` on macOS.
fn user_home(user: &str) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let out = Command::new("dscl").args([".", "-read", &format!("/Users/{user}"), "NFSHomeDirectory"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        return text
            .split_once(':')
            .map(|(_, h)| PathBuf::from(h.trim()))
            .filter(|h| h.is_absolute())
            .or_else(|| Some(PathBuf::from("/Users").join(user)));
    }
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd_home(&passwd, user)
}

fn passwd_home(passwd: &str, user: &str) -> Option<PathBuf> {
    passwd.lines().map(|l| l.split(':').collect::<Vec<_>>()).find(|f| f.len() >= 6 && f[0] == user).map(|f| PathBuf::from(f[5]))
}

/// The data folder the daemon uses at this level (a system service runs as the `sudo` user).
fn data_dir(level: Level) -> PathBuf {
    match (level, run_as()) {
        (Level::System, Some(r)) => paths::app_dirs_in(&r.home).1,
        _ => paths::app_dirs().1,
    }
}

fn log_dir(level: Level) -> PathBuf {
    if cfg!(windows) && level == Level::System {
        // LocalSystem's roaming AppData.
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        return root.join(r"System32\config\systemprofile\AppData\Roaming").join(ID).join("logs");
    }
    data_dir(level).join("logs")
}

/// The service description for this machine.
pub fn spec(level: Level) -> Result<ServiceSpec, String> {
    let mut args = vec!["--daemon".to_string()];
    if cfg!(windows) && level == Level::System {
        args.push("--windows-service".into());
    }
    Ok(ServiceSpec {
        label: label(),
        description: format!("{} daemon", env!("CARGO_PKG_NAME")),
        program: exe()?,
        args,
        level,
        run_as: if level == Level::System { run_as() } else { None },
        env: vec![("APP_PROFILE".into(), paths::profile().into())],
        log_file: log_dir(level).join("daemon.out.log"),
    })
}

/// Create `dir` and the missing folders above it, owned by `user` (a system service
/// writes its log there as that user).
fn create_owned(dir: &Path, user: Option<&str>) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut d = Some(dir);
    while let Some(p) = d.filter(|p| !p.exists()) {
        missing.push(p.to_path_buf());
        d = p.parent();
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if let Some(user) = user {
        for p in missing {
            let _ = Command::new("chown").arg(user).arg(&p).output();
        }
    }
    Ok(())
}

/// The elevated command for an action, as an error people can act on.
fn needs_admin(action: &str) -> String {
    let exe = exe().unwrap_or_else(|_| PathBuf::from(env!("CARGO_PKG_NAME")));
    format!(
        "`--service {action} --system` needs administrator rights. Run:\n  {}",
        elevated_command(&exe, &["--service", action, "--system"])
    )
}

#[cfg(not(windows))]
mod manager {
    //! launchd, systemd, OpenRC and rc.d through `service-manager`.
    use super::*;
    use service_manager::{
        RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceManagerKind, ServiceStartCtx,
        ServiceStatus as SmStatus, ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
    };

    fn sm_label() -> ServiceLabel {
        ServiceLabel { qualifier: None, organization: None, application: label() }
    }

    fn kind() -> Result<ServiceManagerKind, String> {
        ServiceManagerKind::native().map_err(|e| format!("no supported service manager here: {e}"))
    }

    fn open(level: Level) -> Result<Box<dyn ServiceManager>, String> {
        let mut m = <dyn ServiceManager>::target(kind()?);
        m.set_level(match level {
            Level::User => ServiceLevel::User,
            Level::System => ServiceLevel::System,
        })
        .map_err(|e| e.to_string())?;
        if !m.available().unwrap_or(false) {
            return Err(format!("{} is not available on this machine", describe(level)));
        }
        Ok(m)
    }

    pub fn describe(level: Level) -> String {
        match (kind(), level) {
            (Ok(ServiceManagerKind::Launchd), Level::User) => "launchd LaunchAgent".into(),
            (Ok(ServiceManagerKind::Launchd), Level::System) => "launchd LaunchDaemon".into(),
            (Ok(ServiceManagerKind::Systemd), Level::User) => "systemd user unit".into(),
            (Ok(ServiceManagerKind::Systemd), Level::System) => "systemd system unit".into(),
            (Ok(k), _) => format!("{k:?} service"),
            (Err(_), _) => "no service manager".into(),
        }
    }

    pub fn location(level: Level) -> PathBuf {
        let s = sm_label();
        match (kind(), level) {
            (Ok(ServiceManagerKind::Launchd), Level::User) => {
                paths::home().join("Library/LaunchAgents").join(format!("{}.plist", s.to_qualified_name()))
            }
            (Ok(ServiceManagerKind::Launchd), Level::System) => {
                PathBuf::from("/Library/LaunchDaemons").join(format!("{}.plist", s.to_qualified_name()))
            }
            (Ok(ServiceManagerKind::Systemd), Level::User) => {
                service_manager::systemd_user_dir_path().unwrap_or_default().join(format!("{}.service", s.to_script_name()))
            }
            (Ok(ServiceManagerKind::Systemd), Level::System) => {
                service_manager::systemd_global_dir_path().join(format!("{}.service", s.to_script_name()))
            }
            (Ok(ServiceManagerKind::OpenRc), _) => PathBuf::from("/etc/init.d").join(s.to_script_name()),
            _ => PathBuf::from(s.to_qualified_name()),
        }
    }

    fn systemctl(level: Level, args: &[&str]) {
        let mut c = Command::new("systemctl");
        if level == Level::User {
            c.arg("--user");
        }
        let _ = c.args(args).output();
    }

    pub fn install(level: Level) -> Result<(), String> {
        let spec = spec(level)?;
        let kind = kind()?;
        let contents = match kind {
            ServiceManagerKind::Launchd => Some(desktop_core::daemon::launchd_plist(&spec)),
            ServiceManagerKind::Systemd => Some(desktop_core::daemon::systemd_unit(&spec)),
            _ => None,
        };
        let m = open(level)?;
        if let Some(dir) = spec.log_file.parent() {
            create_owned(dir, spec.run_as.as_ref().map(|r| r.user.as_str()))?;
        }
        let ctx = ServiceInstallCtx {
            label: sm_label(),
            program: spec.program.clone(),
            args: spec.args.iter().map(Into::into).collect(),
            contents,
            username: spec.run_as.as_ref().map(|r| r.user.clone()),
            working_directory: None,
            environment: Some(spec.env.clone()),
            autostart: true,
            restart_policy: RestartPolicy::OnFailure {
                delay_secs: Some(desktop_core::daemon::RESTART_DELAY_SECS as u32),
                max_retries: None,
                reset_after_secs: None,
            },
        };
        m.install(ctx).map_err(|e| format!("install: {e}"))?;
        if kind == ServiceManagerKind::Systemd {
            systemctl(level, &["daemon-reload"]);
        }
        // launchd starts it as it loads (RunAtLoad); the others start now.
        if kind != ServiceManagerKind::Launchd {
            start(level).map_err(|e| format!("installed {}, but it did not start: {e}", location(level).display()))?;
        }
        Ok(())
    }

    pub fn uninstall(level: Level) -> Result<(), String> {
        let m = open(level)?;
        let _ = m.stop(ServiceStopCtx { label: sm_label() });
        let result = m.uninstall(ServiceUninstallCtx { label: sm_label() });
        let file = location(level);
        if file.exists() {
            std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        }
        if kind()? == ServiceManagerKind::Systemd {
            systemctl(level, &["daemon-reload"]);
        } else {
            result.map_err(|e| format!("uninstall: {e}"))?;
        }
        Ok(())
    }

    pub fn start(level: Level) -> Result<(), String> {
        open(level)?.start(ServiceStartCtx { label: sm_label() }).map_err(|e| format!("start: {e}"))
    }

    pub fn stop(level: Level) -> Result<(), String> {
        open(level)?.stop(ServiceStopCtx { label: sm_label() }).map_err(|e| format!("stop: {e}"))
    }

    /// (installed, running)
    pub fn state(level: Level) -> (bool, bool) {
        let installed = location(level).exists();
        let running = open(level).and_then(|m| m.status(ServiceStatusCtx { label: sm_label() }).map_err(|e| e.to_string()));
        match running {
            Ok(SmStatus::Running) => (true, true),
            Ok(SmStatus::NotInstalled) => (installed, false),
            Ok(SmStatus::Stopped(_)) | Err(_) => (installed, false),
        }
    }
}

#[cfg(windows)]
mod manager {
    //! The Windows Service through `service-manager` (SCM, `sc.exe`); the user level is a
    //! Run entry (Windows has no per-user service manager).
    use super::*;
    use service_manager::{
        RestartPolicy, ScServiceManager, ServiceInstallCtx, ServiceLabel, ServiceManager, ServiceStartCtx, ServiceStatus as SmStatus,
        ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
    };

    use desktop_core::daemon::sc_failure_args;

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    fn sm_label() -> ServiceLabel {
        ServiceLabel { qualifier: None, organization: None, application: label() }
    }

    fn reg(args: &[&str]) -> Result<(), String> {
        let out = Command::new("reg").args(args).output().map_err(|e| format!("reg: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("reg {}: {}", args.first().copied().unwrap_or_default(), String::from_utf8_lossy(&out.stderr).trim()))
        }
    }

    fn sc(args: &[String]) -> Result<(), String> {
        let out = Command::new("sc.exe").args(args).output().map_err(|e| format!("sc.exe: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("sc.exe {}: {}", args.join(" "), String::from_utf8_lossy(&out.stdout).trim()))
        }
    }

    pub fn describe(level: Level) -> String {
        match level {
            Level::User => "Windows Run entry (starts at login, no restart on failure)".into(),
            Level::System => "Windows Service".into(),
        }
    }

    pub fn location(level: Level) -> PathBuf {
        match level {
            Level::User => PathBuf::from(format!(r"{RUN_KEY}\{}", label())),
            Level::System => PathBuf::from(format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{}", label())),
        }
    }

    fn spawn() -> Result<(), String> {
        Command::new(exe()?).arg("--daemon").env("APP_PROFILE", paths::profile()).spawn().map(|_| ()).map_err(|e| format!("start: {e}"))
    }

    pub fn install(level: Level) -> Result<(), String> {
        let spec = spec(level)?;
        if level == Level::User {
            let exe = spec.program.to_string_lossy().into_owned();
            reg(&["add", RUN_KEY, "/v", &label(), "/t", "REG_SZ", "/d", &format!("\"{exe}\" --daemon"), "/f"])?;
            return spawn();
        }
        let ctx = ServiceInstallCtx {
            label: sm_label(),
            program: spec.program.clone(),
            args: spec.args.iter().map(Into::into).collect(),
            contents: None,
            username: None,
            working_directory: None,
            environment: None,
            autostart: true,
            restart_policy: RestartPolicy::OnFailure { delay_secs: Some(5), max_retries: None, reset_after_secs: None },
        };
        ScServiceManager::system().install(ctx).map_err(|e| format!("install: {e}"))?;
        sc(&sc_failure_args(&label()))?;
        let _ = sc(&["description".into(), label(), spec.description.clone()]);
        start(level)
    }

    pub fn uninstall(level: Level) -> Result<(), String> {
        let _ = stop(level);
        if level == Level::User {
            if state(level).0 {
                reg(&["delete", RUN_KEY, "/v", &label(), "/f"])?;
            }
            return Ok(());
        }
        ScServiceManager::system().uninstall(ServiceUninstallCtx { label: sm_label() }).map_err(|e| format!("uninstall: {e}"))
    }

    pub fn start(level: Level) -> Result<(), String> {
        match level {
            Level::User => spawn(),
            Level::System => ScServiceManager::system().start(ServiceStartCtx { label: sm_label() }).map_err(|e| format!("start: {e}")),
        }
    }

    pub fn stop(level: Level) -> Result<(), String> {
        match level {
            Level::User => {
                crate::ipc::shutdown_worker();
                Ok(())
            }
            Level::System => ScServiceManager::system().stop(ServiceStopCtx { label: sm_label() }).map_err(|e| format!("stop: {e}")),
        }
    }

    /// (installed, running)
    pub fn state(level: Level) -> (bool, bool) {
        match level {
            Level::User => {
                let installed = Command::new("reg").args(["query", RUN_KEY, "/v", &label()]).output().is_ok_and(|o| o.status.success());
                (installed, super::ping_daemon().is_some())
            }
            Level::System => match ScServiceManager::system().status(ServiceStatusCtx { label: sm_label() }) {
                Ok(SmStatus::Running) => (true, true),
                Ok(SmStatus::Stopped(_)) => (true, false),
                _ => (false, false),
            },
        }
    }
}

/// The pid of a daemon serving this profile's socket, if one answers.
fn ping_daemon() -> Option<u32> {
    let v = crate::ipc::request("ping", Value::Null, false, &mut |_| {}).ok()?;
    if v.get("daemon").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    v.get("pid").and_then(Value::as_u64).map(|p| p as u32)
}

/// Installed, running, pid and where things are.
pub fn status(level: Level) -> ServiceStatus {
    let (installed, running) = manager::state(level);
    let health = Health::read(&Health::file(&data_dir(level)));
    // The daemon answering on its socket counts too (the manager may not see one started by hand).
    let answered = ping_daemon();
    let running = running || answered.is_some();
    let pid = if running { answered.or_else(|| health.as_ref().filter(|h| h.fresh(now_secs())).map(|h| h.pid)) } else { None };
    let note = match level {
        Level::System if !is_admin() && !installed => Some(needs_admin("install")),
        _ if !installed && running => Some("a daemon runs, but not as an installed service (started by hand?)".into()),
        _ => health.as_ref().filter(|h| h.failures > 0).map(|h| format!("restarted after {} unexpected stop(s) in a row", h.failures)),
    };
    ServiceStatus {
        level,
        manager: manager::describe(level),
        location: manager::location(level).to_string_lossy().into_owned(),
        installed,
        running,
        pid,
        log: log_dir(level).join("daemon.log").to_string_lossy().into_owned(),
        health,
        note,
    }
}

/// Install (and start) the service. At user level this replaces the background agent.
pub fn install(level: Level) -> Result<String, String> {
    if level == Level::System && !is_admin() {
        return Err(needs_admin("install"));
    }
    let mut said = String::new();
    #[cfg(feature = "background-agent")]
    if level == Level::User && crate::agent::installed() {
        crate::agent::set_agent(false)?;
        crate::ipc::shutdown_worker();
        said.push_str("Removed the background agent (the service replaces it).\n");
    }
    manager::install(level)?;
    let s = status(level);
    said.push_str(&format!("Installed the {} at {}.\nLogs: {}", s.manager, s.location, s.log));
    Ok(said)
}

/// Stop and remove the service.
pub fn uninstall(level: Level) -> Result<String, String> {
    if level == Level::System && !is_admin() {
        return Err(needs_admin("uninstall"));
    }
    if !manager::state(level).0 {
        return Ok(format!("The {} is not installed.", manager::describe(level)));
    }
    manager::uninstall(level)?;
    Ok(format!("Removed the {}.", manager::describe(level)))
}

fn control(level: Level, action: &str) -> Result<String, String> {
    if level == Level::System && !is_admin() {
        return Err(needs_admin(action));
    }
    if !manager::state(level).0 {
        return Err(format!(
            "the service is not installed; run `--service install{}` first",
            if level == Level::System { " --system" } else { "" }
        ));
    }
    match action {
        "start" => manager::start(level)?,
        "stop" => manager::stop(level)?,
        _ => {
            let _ = manager::stop(level);
            std::thread::sleep(Duration::from_millis(500));
            manager::start(level)?;
        }
    }
    Ok(format!("{action}: done."))
}

/// Print the last `lines` of the daemon's log, then (with `follow`) what is added.
fn logs(level: Level, lines: usize, follow: bool) -> Result<(), String> {
    let file = log_dir(level).join("daemon.log");
    let out_file = log_dir(level).join("daemon.out.log");
    match desktop_core::daemon::tail(&file, lines) {
        Ok(l) => l.iter().for_each(|l| println!("{l}")),
        Err(e) => return Err(format!("{}: {e} (has the daemon run yet?)", file.display())),
    }
    if out_file.exists() {
        eprintln!("(stdout and stderr: {})", out_file.display());
    }
    if follow {
        use std::io::{Read, Seek, SeekFrom};
        let mut pos = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            if len < pos {
                pos = 0; // rotated
            }
            if len > pos {
                let mut f = std::fs::File::open(&file).map_err(|e| e.to_string())?;
                f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                print!("{}", String::from_utf8_lossy(&buf));
                pos = len;
            }
        }
    }
    Ok(())
}

/// The `--service` help.
pub const USAGE: &str = "usage: --service <install|uninstall|start|stop|restart|status|logs> [--system] [--json] [-n <lines>] [--follow]";

/// `<app> --service <action> [--system] [--json] [-n N] [--follow]`; prints and returns the
/// exit code.
pub fn cli(args: &[String]) -> i32 {
    let level = if args.iter().any(|a| a == "--system") { Level::System } else { Level::User };
    let json_out = args.iter().any(|a| a == "--json");
    let action = args.iter().find(|a| !a.starts_with('-') && a.parse::<usize>().is_err()).map(String::as_str).unwrap_or("status");
    let result = match action {
        "install" => install(level),
        "uninstall" => uninstall(level),
        "start" | "stop" | "restart" => control(level, action),
        "status" => {
            let s = status(level);
            if json_out {
                println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
            } else {
                println!("{}: {}", s.manager, if s.installed { "installed" } else { "not installed" });
                println!("  file:    {}", s.location);
                println!(
                    "  running: {}",
                    if s.running { s.pid.map(|p| format!("yes, pid {p}")).unwrap_or_else(|| "yes".into()) } else { "no".into() }
                );
                println!("  log:     {}", s.log);
                if let Some(n) = &s.note {
                    println!("  note:    {n}");
                }
            }
            return 0;
        }
        "logs" => {
            let n = args
                .iter()
                .position(|a| a == "-n" || a == "--lines")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            logs(level, n, args.iter().any(|a| a == "--follow" || a == "-f")).map(|()| String::new())
        }
        "help" | "--help" => Ok(USAGE.to_string()),
        other => Err(format!("unknown action {other:?}\n{USAGE}")),
    };
    match result {
        Ok(text) => {
            if json_out {
                println!("{}", json!({"ok": true, "message": text}));
            } else if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        Err(e) => {
            if json_out {
                println!("{}", json!({"ok": false, "error": e}));
            } else {
                eprintln!("{e}");
            }
            1
        }
    }
}

/// Settings: the user-level service.
#[tauri::command(async)]
pub fn service_status() -> ServiceStatus {
    status(Level::User)
}

/// Settings: install (`true`) or remove (`false`) the user-level service.
#[tauri::command(async)]
pub fn set_service(enabled: bool) -> Result<ServiceStatus, String> {
    if enabled {
        install(Level::User)?;
    } else {
        uninstall(Level::User)?;
    }
    Ok(status(Level::User))
}

/// Settings: how to install the system-level service (it needs administrator rights).
#[tauri::command(async)]
pub fn service_system_hint() -> String {
    let exe = exe().unwrap_or_else(|_| PathBuf::from(env!("CARGO_PKG_NAME")));
    elevated_command(&exe, &["--service", "install", "--system"])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe(&'static str, Arc<std::sync::Mutex<Vec<String>>>, bool);

    impl Headless for Probe {
        fn name(&self) -> &str {
            self.0
        }
        fn start(&mut self, _ctx: &Context) -> Result<(), String> {
            self.1.lock().unwrap().push(format!("start {}", self.0));
            if self.2 {
                Err("broken".into())
            } else {
                Ok(())
            }
        }
        fn stop(&mut self) {
            self.1.lock().unwrap().push(format!("stop {}", self.0));
        }
    }

    #[test]
    fn tasks_start_in_order_and_stop_in_reverse() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ctx = Context {
            local: Arc::new(Local::default()),
            config_dir: PathBuf::new(),
            data_dir: PathBuf::new(),
            stopping: Arc::new(AtomicBool::new(false)),
        };
        let tasks: Vec<Box<dyn Headless>> = vec![
            Box::new(Probe("a", log.clone(), false)),
            Box::new(Probe("bad", log.clone(), true)),
            Box::new(Probe("b", log.clone(), false)),
        ];
        let mut started = start_tasks(tasks, &ctx);
        assert_eq!(started.iter().map(|t| t.name().to_string()).collect::<Vec<_>>(), ["a", "b"]);
        stop_tasks(&mut started);
        assert_eq!(*log.lock().unwrap(), ["start a", "start bad", "start b", "stop b", "stop a"]);
    }

    #[test]
    fn this_build_lists_its_headless_tasks() {
        let names: Vec<String> = headless_tasks().iter().map(|t| t.name().to_string()).collect();
        let expected: &[&str] = if cfg!(feature = "jobs-queue") { &["jobs"] } else { &[] };
        assert_eq!(names, expected);
    }

    #[test]
    fn reads_homes_from_passwd() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nann:x:1000:1000:Ann:/home/ann:/bin/zsh\n";
        assert_eq!(passwd_home(passwd, "ann"), Some(PathBuf::from("/home/ann")));
        assert_eq!(passwd_home(passwd, "bob"), None);
    }

    #[test]
    fn the_label_names_the_app() {
        assert!(label().starts_with(ID));
        assert!(label().ends_with(".daemon"));
        let s = spec(Level::User).unwrap();
        assert_eq!(s.args[0], "--daemon");
        assert!(s.log_file.ends_with("logs/daemon.out.log"));
    }

    #[test]
    fn stop_requests_set_the_flags() {
        assert!(!sleep_unless_stopped(Duration::from_millis(1)));
        request_stop(true);
        assert!(stopping());
        assert!(RESTART.load(Ordering::SeqCst));
    }
}
