//! Run the backend as a background service (a daemon): the pieces that do not depend on
//! the operating system's service manager, so they can be tested anywhere.
//!
//! The app binary has a headless mode, `<app> --daemon`, and installs itself as a service
//! with `<app> --service install [--system]` (see the app's `daemon` module). This module
//! holds what both need:
//!
//! - [`ServiceSpec`] and the service files it renders: a launchd property list
//!   ([`launchd_plist`], macOS LaunchAgent or LaunchDaemon) and a systemd unit
//!   ([`systemd_unit`], `systemctl --user` or system). Both restart the daemon when it
//!   fails, and send its stdout and stderr to the app's log folder.
//! - [`sc_failure_args`]: the restart-with-backoff actions of a Windows Service.
//! - [`Health`]: the pid and health file the daemon keeps while it runs (`daemon.json` in
//!   the data folder), removed when it stops cleanly.
//! - [`backoff`] and [`Health::next_failures`]: crash-loop backoff the daemon applies itself,
//!   so restarts slow down the same way on every platform (launchd only has a fixed delay).
//! - [`elevated_command`]: the command to run as an administrator, printed instead of
//!   asking for a password (the app never escalates by itself).
//! - [`tail`]: the last lines of a log file.
//!
//! ```
//! use desktop_core::daemon::{backoff, launchd_plist, systemd_unit, Level, ServiceSpec};
//! use std::path::PathBuf;
//! use std::time::Duration;
//!
//! let spec = ServiceSpec {
//!     label: "com.example.notes.daemon".into(),
//!     description: "Notes daemon".into(),
//!     program: PathBuf::from("/opt/Notes/notes"),
//!     args: vec!["--daemon".into()],
//!     level: Level::User,
//!     run_as: None,
//!     env: vec![("APP_PROFILE".into(), "default".into())],
//!     log_file: PathBuf::from("/home/me/.local/share/com.example.notes/logs/daemon.out.log"),
//! };
//! let unit = systemd_unit(&spec);
//! assert!(unit.contains("ExecStart=\"/opt/Notes/notes\" \"--daemon\""));
//! assert!(unit.contains("Restart=on-failure"));
//! assert!(launchd_plist(&spec).contains("<key>SuccessfulExit</key><false/>"));
//!
//! // The first failure restarts at once; later ones wait longer, up to five minutes.
//! assert_eq!(backoff(1), Duration::ZERO);
//! assert_eq!(backoff(3), Duration::from_secs(4));
//! assert_eq!(backoff(40), Duration::from_secs(300));
//! ```

use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Who the service belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// The signed-in user: a LaunchAgent, a `systemctl --user` unit, or a Windows Run entry.
    /// Needs no administrator rights.
    User,
    /// The machine: a LaunchDaemon, a system unit or a Windows Service. Starts at boot,
    /// before anyone signs in, and needs administrator rights to install.
    System,
}

/// The account a system-level service runs as (macOS and Linux), so it uses that
/// person's data folder and the window they open can reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAs {
    /// The user name.
    pub user: String,
    /// Their home folder.
    pub home: PathBuf,
}

/// Everything a service file needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    /// The launchd label, the Windows service name and (with `-` for `.`) the systemd unit name.
    pub label: String,
    /// One line for people (`systemctl status`, the Services console).
    pub description: String,
    /// The app binary.
    pub program: PathBuf,
    /// Its arguments, usually `--daemon`.
    pub args: Vec<String>,
    /// User or system.
    pub level: Level,
    /// System level on macOS and Linux: run as this user instead of root.
    pub run_as: Option<RunAs>,
    /// Extra environment, e.g. `APP_PROFILE`.
    pub env: Vec<(String, String)>,
    /// Where stdout and stderr go (the daemon's own JSON-lines log sits next to it).
    pub log_file: PathBuf,
}

impl ServiceSpec {
    /// The systemd unit file name: the label with `-` for `.`, plus `.service`.
    pub fn unit_name(&self) -> String {
        format!("{}.service", self.label.replace('.', "-"))
    }

    fn all_env(&self) -> Vec<(String, String)> {
        let mut env = self.env.clone();
        if let Some(r) = &self.run_as {
            env.push(("HOME".into(), r.home.to_string_lossy().into_owned()));
            env.push(("USER".into(), r.user.clone()));
        }
        env
    }
}

/// The seconds launchd waits before it starts a daemon that stopped (`ThrottleInterval`),
/// and systemd's first restart delay.
pub const RESTART_DELAY_SECS: u64 = 5;

/// The longest wait between restarts.
pub const MAX_BACKOFF_SECS: u64 = 300;

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A launchd property list: `RunAtLoad`, restart only after a failure
/// (`KeepAlive`/`SuccessfulExit` false, so `launchctl stop` and a clean exit stay stopped),
/// `ThrottleInterval` between restarts, and stdout/stderr into the log folder.
pub fn launchd_plist(spec: &ServiceSpec) -> String {
    let mut args = format!("    <string>{}</string>\n", xml(&spec.program.to_string_lossy()));
    for a in &spec.args {
        args.push_str(&format!("    <string>{}</string>\n", xml(a)));
    }
    let env = spec.all_env();
    let mut env_xml = String::new();
    if !env.is_empty() {
        env_xml.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (k, v) in &env {
            env_xml.push_str(&format!("    <key>{}</key><string>{}</string>\n", xml(k), xml(v)));
        }
        env_xml.push_str("  </dict>\n");
    }
    let user = spec.run_as.as_ref().map(|r| format!("  <key>UserName</key><string>{}</string>\n", xml(&r.user))).unwrap_or_default();
    let log = xml(&spec.log_file.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key><false/>
  </dict>
  <key>ThrottleInterval</key><integer>{delay}</integer>
  <key>ExitTimeOut</key><integer>30</integer>
  <key>ProcessType</key><string>Background</string>
{user}{env_xml}  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        label = xml(&spec.label),
        delay = RESTART_DELAY_SECS,
    )
}

/// One word for a systemd command line or `Environment=`: in double quotes, with `\`, `"`
/// escaped, and `%` (a specifier) doubled.
pub fn systemd_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
}

/// A systemd unit: `Restart=on-failure` with a growing delay (`RestartSteps` and
/// `RestartMaxDelaySec`, systemd 254 and later; older versions log that they ignore them
/// and keep the fixed `RestartSec`), no start limit, stdout/stderr appended to the log file.
pub fn systemd_unit(spec: &ServiceSpec) -> String {
    let mut exec = systemd_quote(&spec.program.to_string_lossy());
    for a in &spec.args {
        exec.push(' ');
        exec.push_str(&systemd_quote(a));
    }
    let mut service = String::new();
    if let Some(r) = &spec.run_as {
        service.push_str(&format!("User={}\n", r.user));
    }
    for (k, v) in spec.all_env() {
        service.push_str(&format!("Environment={}\n", systemd_quote(&format!("{k}={v}"))));
    }
    let log = spec.log_file.to_string_lossy().replace('%', "%%");
    let (after, wanted_by) = match spec.level {
        // The user manager has no network-online.target.
        Level::User => ("", "default.target"),
        Level::System => ("After=network-online.target\nWants=network-online.target\n", "multi-user.target"),
    };
    format!(
        "[Unit]\nDescription={desc}\n{after}StartLimitIntervalSec=0\n\n\
         [Service]\nType=simple\nExecStart={exec}\n{service}\
         Restart=on-failure\nRestartSec={delay}\nRestartSteps=6\nRestartMaxDelaySec={max}\n\
         TimeoutStopSec=30\nKillSignal=SIGTERM\n\
         StandardOutput=append:{log}\nStandardError=append:{log}\n\n\
         [Install]\nWantedBy={wanted_by}\n",
        desc = spec.description,
        delay = RESTART_DELAY_SECS,
        max = MAX_BACKOFF_SECS,
    )
}

/// Arguments for `sc.exe` that make a Windows Service restart after a failure, waiting 5 s,
/// then 30 s, then 2 minutes for every later failure; the count resets after a day.
pub fn sc_failure_args(service: &str) -> Vec<String> {
    ["failure", service, "reset=", "86400", "actions=", "restart/5000/restart/30000/restart/120000"].map(String::from).to_vec()
}

/// The command a person runs as administrator: `sudo ...` on macOS and Linux, an
/// Administrator PowerShell line on Windows. Arguments with spaces are quoted.
pub fn elevated_command(program: &Path, args: &[&str]) -> String {
    let q = |s: &str| if s.contains(' ') || s.is_empty() { format!("\"{s}\"") } else { s.to_string() };
    let words: Vec<String> = std::iter::once(q(&program.to_string_lossy())).chain(args.iter().map(|a| q(a))).collect();
    if cfg!(windows) {
        format!("In a PowerShell opened with \"Run as administrator\": & {}", words.join(" "))
    } else {
        format!("sudo {}", words.join(" "))
    }
}

/// Seconds since 1970.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The wait before the daemon starts after `failures` unclean exits in a row: none for the
/// first, then 2, 4, 8 ... seconds, at most [`MAX_BACKOFF_SECS`].
pub fn backoff(failures: u32) -> Duration {
    if failures <= 1 {
        return Duration::ZERO;
    }
    Duration::from_secs((1u64 << (failures - 1).min(20)).min(MAX_BACKOFF_SECS))
}

/// A run that lasted this long counts as healthy: the failure count starts again.
pub const STABLE_SECS: u64 = 600;

/// The daemon's pid and health file (`daemon.json` in the data folder): written at start,
/// its heartbeat refreshed while it runs, and removed on a clean stop. A file left behind
/// means the last run crashed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Health {
    /// The daemon's process id.
    pub pid: u32,
    /// The app version.
    pub version: String,
    /// The profile it serves.
    pub profile: String,
    /// When it started (seconds since 1970).
    pub started: u64,
    /// Last sign of life (seconds since 1970), refreshed every [`HEARTBEAT_SECS`].
    pub heartbeat: u64,
    /// Unclean exits in a row before this start (0 = the last stop was clean).
    pub failures: u32,
    /// The headless tasks it runs (job workers, schedulers ...).
    #[serde(default)]
    pub tasks: Vec<String>,
}

/// How often the daemon refreshes [`Health::heartbeat`].
pub const HEARTBEAT_SECS: u64 = 30;

impl Health {
    /// The health file in a data folder.
    pub fn file(data_dir: &Path) -> PathBuf {
        data_dir.join("daemon.json")
    }

    /// The file's content, if there is a readable one.
    pub fn read(file: &Path) -> Option<Health> {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// Write it through a temporary file, so a reader never sees half of it.
    pub fn write(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, file)
    }

    /// The failure count for a new start: `left_over` is the file a previous run did not
    /// remove (it crashed). A crash after a run of [`STABLE_SECS`] or more counts as the first.
    pub fn next_failures(left_over: Option<&Health>, now: u64) -> u32 {
        match left_over {
            None => 0,
            Some(h) if h.heartbeat.saturating_sub(h.started) >= STABLE_SECS || now.saturating_sub(h.heartbeat) >= STABLE_SECS => 1,
            Some(h) => h.failures.saturating_add(1),
        }
    }

    /// Whether the heartbeat is recent (the daemon is alive, or died very recently).
    pub fn fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.heartbeat) <= HEARTBEAT_SECS * 3
    }
}

/// The last `lines` lines of a text file (reads at most the last 1 MB).
pub fn tail(file: &Path, lines: usize) -> std::io::Result<Vec<String>> {
    let mut f = std::fs::File::open(file)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(1 << 20);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        all.remove(0); // probably cut in the middle
    }
    Ok(all[all.len().saturating_sub(lines)..].iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(level: Level) -> ServiceSpec {
        ServiceSpec {
            label: "com.example.app.daemon".into(),
            description: "App daemon".into(),
            program: PathBuf::from("/Applications/My App.app/Contents/MacOS/my-app"),
            args: vec!["--daemon".into()],
            level,
            run_as: None,
            env: vec![("APP_PROFILE".into(), "work".into())],
            log_file: PathBuf::from("/tmp/logs/daemon.out.log"),
        }
    }

    #[test]
    fn plist_restarts_on_failure_and_logs() {
        let p = launchd_plist(&spec(Level::User));
        assert!(p.contains("<key>Label</key><string>com.example.app.daemon</string>"));
        assert!(p.contains("<string>/Applications/My App.app/Contents/MacOS/my-app</string>\n    <string>--daemon</string>"));
        assert!(p.contains("<key>SuccessfulExit</key><false/>"));
        assert!(p.contains("<key>ThrottleInterval</key><integer>5</integer>"));
        assert!(p.contains("<key>StandardErrorPath</key><string>/tmp/logs/daemon.out.log</string>"));
        assert!(p.contains("<key>APP_PROFILE</key><string>work</string>"));
        assert!(!p.contains("UserName"));
    }

    #[test]
    fn system_plist_runs_as_the_user() {
        let mut s = spec(Level::System);
        s.run_as = Some(RunAs { user: "ann".into(), home: "/Users/ann".into() });
        s.label = "a&b".into();
        let p = launchd_plist(&s);
        assert!(p.contains("<key>UserName</key><string>ann</string>"));
        assert!(p.contains("<key>HOME</key><string>/Users/ann</string>"));
        assert!(p.contains("<string>a&amp;b</string>"));
    }

    #[test]
    fn unit_has_backoff_and_logs() {
        let u = systemd_unit(&spec(Level::User));
        assert!(u.contains("ExecStart=\"/Applications/My App.app/Contents/MacOS/my-app\" \"--daemon\"\n"));
        assert!(u.contains("Restart=on-failure\nRestartSec=5\nRestartSteps=6\nRestartMaxDelaySec=300\n"));
        assert!(u.contains("StartLimitIntervalSec=0"));
        assert!(u.contains("StandardError=append:/tmp/logs/daemon.out.log"));
        assert!(u.contains("Environment=\"APP_PROFILE=work\""));
        assert!(u.contains("WantedBy=default.target"));
        assert!(!u.contains("network-online"));
        assert!(!u.contains("User="));
        assert_eq!(spec(Level::User).unit_name(), "com-example-app-daemon.service");
    }

    #[test]
    fn system_unit_runs_as_the_user_at_boot() {
        let mut s = spec(Level::System);
        s.run_as = Some(RunAs { user: "ann".into(), home: "/home/ann".into() });
        let u = systemd_unit(&s);
        assert!(u.contains("User=ann\n"));
        assert!(u.contains("Environment=\"HOME=/home/ann\""));
        assert!(u.contains("WantedBy=multi-user.target"));
        assert!(u.contains("After=network-online.target"));
    }

    #[test]
    fn quotes_for_systemd() {
        assert_eq!(systemd_quote(r#"a "b" \ 50%"#), r#""a \"b\" \\ 50%%""#);
    }

    #[test]
    fn backoff_grows_and_caps() {
        let secs: Vec<u64> = (0..12).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [0, 0, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300]);
        assert_eq!(backoff(u32::MAX).as_secs(), 300);
    }

    #[test]
    fn failures_count_crash_loops_only() {
        let h = |started, heartbeat, failures| Health {
            pid: 1,
            version: "1".into(),
            profile: "default".into(),
            started,
            heartbeat,
            failures,
            tasks: vec![],
        };
        assert_eq!(Health::next_failures(None, 1000), 0);
        // Crashed 10 s after starting, the third time in a row.
        assert_eq!(Health::next_failures(Some(&h(1000, 1010, 2)), 1012), 3);
        // Ran for an hour before it crashed: a fresh start of the count.
        assert_eq!(Health::next_failures(Some(&h(1000, 4600, 7)), 4601), 1);
        // A file left over from long ago.
        assert_eq!(Health::next_failures(Some(&h(1000, 1010, 7)), 99_999), 1);
        assert!(h(0, 100, 0).fresh(150));
        assert!(!h(0, 100, 0).fresh(1000));
    }

    #[test]
    fn health_file_round_trips_and_tail_reads_the_end() {
        let dir = std::env::temp_dir().join(format!("daemon-test-{}", std::process::id()));
        let file = Health::file(&dir);
        let h = Health {
            pid: 42,
            version: "0.1.0".into(),
            profile: "default".into(),
            started: 1,
            heartbeat: 2,
            failures: 0,
            tasks: vec!["jobs".into()],
        };
        h.write(&file).unwrap();
        assert_eq!(Health::read(&file), Some(h));
        let log = dir.join("x.log");
        std::fs::write(&log, "a\nb\nc\nd\n").unwrap();
        assert_eq!(tail(&log, 2).unwrap(), ["c", "d"]);
        assert_eq!(tail(&log, 10).unwrap().len(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn elevated_command_quotes() {
        let c = elevated_command(Path::new("/opt/My App/app"), &["--service", "install", "--system"]);
        assert!(c.contains("\"/opt/My App/app\" --service install --system"), "{c}");
        assert_eq!(sc_failure_args("svc")[0], "failure");
    }
}
