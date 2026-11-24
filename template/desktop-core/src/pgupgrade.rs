//! Upgrading the embedded Postgres data folder to a new major version (feature `postgres`).
//!
//! An app update can ship a newer major Postgres than the one that made the user's data
//! folder (Postgres refuses to start on another major's files). On start, `pgstore` reads
//! `PG_VERSION` in the data folder ([`need`]) and, when it is older than the bundled major,
//! acts on the `postgres.upgrade` setting ([`UpgradeMode`]):
//!
//! - `auto`: upgrade now;
//! - `ask` (the default): stop with an error that starts with [`UPGRADE_NEEDED`], so the UI
//!   can ask; its answer starts the app again with `auto` (upgrade) or `off` (not now);
//! - `off`: keep running on the old major's server (still cached in the install folder).
//!
//! The upgrade ([`plan`], then [`execute`]):
//!
//! 1. gets the old major's server: the copy the previous app version left in `install/`,
//!    or a download from a pinned release URL checked against a pinned SHA-256
//!    ([`OLD_SERVERS`]; the server only, no extension bundle);
//! 2. starts the old server once to read its encoding and locale (this also finishes a
//!    crash recovery, which `pg_upgrade` needs), and stops it cleanly;
//! 3. makes a new cluster (`data.pg<new>.new`) with the same encoding, locale, superuser and
//!    password;
//! 4. runs `pg_upgrade --check`, then `pg_upgrade --link` (or `--copy` with
//!    `postgres.upgradeCopy`);
//! 5. renames `data` to `data.pg<old>.bak` and the new cluster to `data`;
//! 6. starts the new server, runs a smoke query, `ALTER EXTENSION ... UPDATE` for pgvector
//!    (and pg_tokenizer and VectorChord-BM25 with feature `bm25`; their libraries for the new
//!    major are installed before the check) and `ANALYZE`;
//! 7. only then removes `data.pg<old>.bak`.
//!
//! When any step up to `pg_upgrade` fails, the old folder is put back as it was, the error is
//! kept in `pg-upgrade-failed.txt` and the app keeps running on the old server; `auto` then
//! does not try again by itself until `<app> --pg-upgrade` or the UI's Upgrade button does.
//! When the new server fails its smoke test, a `--copy` upgrade is rolled back the same way;
//! after `--link` the old files are shared with a server that has started, so the old
//! folder is kept as `data.pg<old>.bak` and the error says so. User data is never deleted
//! on failure. A crash part-way is finished or undone on the next start ([`recover`]).
//!
//! Everything that runs a Postgres program, downloads or talks to a server goes through the
//! [`Runner`] trait, so the plan and the rollback are tested without Postgres.
//!
//! # Example
//!
//! ```
//! use desktop_core::pgupgrade::{need, plan, Method, Need, PlanOptions, Step};
//!
//! let dir = tempfile::tempdir()?;
//! std::fs::create_dir(dir.path().join("data"))?;
//! std::fs::write(dir.path().join("data/PG_VERSION"), "15\n")?;
//! assert_eq!(need(&dir.path().join("data"), 16)?, Need::Upgrade { from: 15, to: 16 });
//!
//! // The old server is not cached, so it is downloaded first; `--check` runs before the upgrade.
//! let opts = PlanOptions { old_version: "15.20.0".into(), old_cached: false, method: Method::Link, check_only: false };
//! let steps = plan(&opts);
//! assert_eq!(steps[0], Step::FetchOld { version: "15.20.0".into() });
//! let check = steps.iter().position(|s| *s == Step::Check).unwrap();
//! let upgrade = steps.iter().position(|s| *s == Step::Upgrade(Method::Link)).unwrap();
//! assert!(check < upgrade);
//! assert_eq!(steps.last(), Some(&Step::RemoveBackup));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Start of the error `pgstore` returns in `ask` mode when the data folder needs an upgrade;
/// the UI looks for it to show its prompt.
pub const UPGRADE_NEEDED: &str = "pg-upgrade-needed";

/// The superuser every embedded cluster is made with (as `postgresql_embedded` does).
const SUPERUSER: &str = "postgres";

/// What to do when the data folder was made by an older major Postgres (`postgres.upgrade`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum UpgradeMode {
    /// Upgrade at start, with progress in the setup screen.
    Auto,
    /// Ask first (the UI shows a prompt); until then the app does not open the store.
    #[default]
    Ask,
    /// Never upgrade: keep running on the old major's server.
    Off,
}

/// How `pg_upgrade` moves the data files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Method {
    /// `--link`: hard links, fast and with no extra disk space (the default).
    #[default]
    Link,
    /// `--copy`: a full copy, slower and twice the space, but the old folder stays usable.
    Copy,
}

impl Method {
    fn flag(self) -> &'static str {
        match self {
            Method::Link => "--link",
            Method::Copy => "--copy",
        }
    }
}

/// The major version in a version string: `"16"` and `"16.4.0"` are 16. Majors before 10
/// (`"9.6"`) are not supported and give `None`.
///
/// ```
/// use desktop_core::pgupgrade::parse_major;
/// assert_eq!(parse_major("15\n"), Some(15));
/// assert_eq!(parse_major("16.16.0"), Some(16));
/// assert_eq!(parse_major("9.6"), None);
/// ```
pub fn parse_major(text: &str) -> Option<u32> {
    text.trim().split('.').next()?.parse().ok().filter(|m| *m >= 10)
}

/// The major Postgres that made the data folder (its `PG_VERSION`), or `None` when there is
/// no cluster there yet.
pub fn data_major(data: &Path) -> Result<Option<u32>> {
    match std::fs::read_to_string(data.join("PG_VERSION")) {
        Ok(text) => parse_major(&text).map(Some).ok_or_else(|| Error::Store(format!("unsupported PG_VERSION {:?} in {}", text.trim(), data.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// What the data folder needs before the bundled server can start on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// No cluster yet: the server makes one.
    Fresh,
    /// Made by the bundled major.
    Current,
    /// Made by an older major: upgrade `from` -> `to`.
    Upgrade {
        /// The data folder's major.
        from: u32,
        /// The bundled major.
        to: u32,
    },
    /// Made by a newer major than this app has (an older app version was installed).
    Newer {
        /// The data folder's major.
        found: u32,
        /// The bundled major.
        bundled: u32,
    },
}

/// Compare the data folder's `PG_VERSION` with the `bundled` major.
pub fn need(data: &Path, bundled: u32) -> Result<Need> {
    Ok(match data_major(data)? {
        None => Need::Fresh,
        Some(m) if m == bundled => Need::Current,
        Some(m) if m < bundled => Need::Upgrade { from: m, to: bundled },
        Some(m) => Need::Newer { found: m, bundled },
    })
}

/// An older major's server archive: the exact version and its SHA-256 per target.
#[derive(Debug, Clone, Copy)]
pub struct OldServer {
    /// The exact version, as in the release tag (`15.20.0`).
    pub version: &'static str,
    /// `(target triple, SHA-256 of postgresql-<version>-<target>.tar.gz)`.
    pub sha256: &'static [(&'static str, &'static str)],
}

impl OldServer {
    /// The pinned SHA-256 of this server's archive for `target`.
    pub fn sha256_for(&self, target: &str) -> Option<&'static str> {
        self.sha256.iter().find(|(t, _)| *t == target).map(|(_, h)| *h)
    }
}

/// Older majors this build can upgrade from when their server is not cached, each pinned
/// like the bundled one (from the release's `.sha256` files). When the bundled major moves
/// on, the previous one goes here.
pub const OLD_SERVERS: &[OldServer] = &[
    OldServer {
        version: "15.20.0",
        sha256: &[
            ("aarch64-apple-darwin", "b4f28bbbcb5f195df3f84b5ad9ab8ee60d0fab44e60fdf4b1036f0c7e1451a79"),
            ("x86_64-pc-windows-msvc", "a5f192b45d51d4ff59f33ca1549e6a7e0b8c2727c5e87f01163755c572459c19"),
            ("x86_64-unknown-linux-gnu", "2eec5422941fc12339d86c18542753a6b3ca6822149b18d036a05717d5f58e26"),
        ],
    },
    OldServer {
        version: "16.16.0",
        sha256: &[
            ("aarch64-apple-darwin", "b7f002d554cd8a916d189e24b8f885d088b7909bb6d2de092f82c10274cb8251"),
            ("x86_64-pc-windows-msvc", "4557b8251bb967c1c09eab57439fb2f1179d35c70debfc692fe637bb08d2f20c"),
            ("x86_64-unknown-linux-gnu", "f668fbc685cdcb71d6a82f57035bed7d97b059e2d8a1d3adf317ec36c5e861d2"),
        ],
    },
];

/// The pinned server for `major`, if this build has one.
pub fn old_server(major: u32) -> Option<&'static OldServer> {
    OLD_SERVERS.iter().find(|s| parse_major(s.version) == Some(major))
}

/// The newest `<major>.x.y` server already in `install` (the crate's versioned folders),
/// as its version string.
pub fn installed_version(install: &Path, major: u32) -> Option<String> {
    let mut found: Vec<(Vec<u32>, String)> = std::fs::read_dir(install)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| bin(&e.path().join("bin"), "pg_ctl").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| parse_major(name) == Some(major))
        .map(|name| (name.split('.').map(|p| p.parse().unwrap_or(0)).collect(), name))
        .collect();
    found.sort();
    found.pop().map(|(_, name)| name)
}

/// One step of an upgrade, in the order [`plan`] lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Download and unpack the old major's server (pinned URL and SHA-256).
    FetchOld {
        /// The exact version to fetch.
        version: String,
    },
    /// Start the old server once, read the cluster's encoding and locale, stop it cleanly.
    ReadLocale,
    /// `initdb` the new cluster in `data.pg<new>.new`.
    InitNew,
    /// `pg_upgrade --check`.
    Check,
    /// `pg_upgrade --link` or `--copy`.
    Upgrade(Method),
    /// `data` -> `data.pg<old>.bak`, `data.pg<new>.new` -> `data`.
    Promote,
    /// Start the new server.
    Start,
    /// A query that must succeed on the new server.
    Smoke,
    /// `ALTER EXTENSION ... UPDATE` for each extension the app uses that is installed.
    UpdateExtensions,
    /// `ANALYZE` (pg_upgrade does not carry planner statistics over).
    Analyze,
    /// Remove `data.pg<old>.bak`.
    RemoveBackup,
    /// Remove the new cluster again (after a check-only run).
    RemoveNew,
}

/// What [`plan`] needs to know.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanOptions {
    /// The old major's exact server version (cached or pinned).
    pub old_version: String,
    /// The old server is already in the install folder.
    pub old_cached: bool,
    /// `--link` or `--copy`.
    pub method: Method,
    /// Only `pg_upgrade --check` (the `--pg-upgrade --check` command): nothing changes.
    pub check_only: bool,
}

/// The steps of an upgrade (or of a check), in order.
pub fn plan(opts: &PlanOptions) -> Vec<Step> {
    let mut steps = Vec::new();
    if !opts.old_cached {
        steps.push(Step::FetchOld { version: opts.old_version.clone() });
    }
    steps.extend([Step::ReadLocale, Step::InitNew, Step::Check]);
    if opts.check_only {
        steps.push(Step::RemoveNew);
        return steps;
    }
    steps.extend([
        Step::Upgrade(opts.method),
        Step::Promote,
        Step::Start,
        Step::Smoke,
        Step::UpdateExtensions,
        Step::Analyze,
        Step::RemoveBackup,
    ]);
    steps
}

/// The folders of one embedded server (`pgstore`'s data folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The server's folder: `data/`, `install/`, `.pgpass`.
    pub root: PathBuf,
}

impl Layout {
    /// The live cluster.
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    /// Where the new cluster is built.
    pub fn new_data(&self, to: u32) -> PathBuf {
        self.root.join(format!("data.pg{to}.new"))
    }
    /// Where the old cluster waits until the new one has passed its smoke test.
    pub fn backup(&self, from: u32) -> PathBuf {
        self.root.join(format!("data.pg{from}.bak"))
    }
    /// Where a `--copy` upgrade that failed after starting is set aside.
    pub fn failed(&self, to: u32) -> PathBuf {
        self.root.join(format!("data.pg{to}.failed"))
    }
    /// The crate's versioned install folders.
    pub fn install(&self) -> PathBuf {
        self.root.join("install")
    }
    /// The last automatic upgrade's error.
    pub fn failure_marker(&self) -> PathBuf {
        self.root.join("pg-upgrade-failed.txt")
    }
    /// The log of the old server's short start in [`Step::ReadLocale`].
    fn locale_log(&self) -> PathBuf {
        self.root.join("pg-upgrade-old-start.log")
    }
}

/// The last automatic upgrade's error, if it failed and nobody has retried since.
pub fn last_failure(layout: &Layout) -> Option<String> {
    std::fs::read_to_string(layout.failure_marker()).ok().map(|s| s.trim().to_string())
}

/// Forget a failed upgrade, so `auto` tries again (the UI's Upgrade button and
/// `--pg-upgrade` do this).
pub fn clear_failure(layout: &Layout) -> Result<()> {
    match std::fs::remove_file(layout.failure_marker()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn record_failure(layout: &Layout, message: &str) {
    if let Err(e) = std::fs::write(layout.failure_marker(), message) {
        log::warn!("could not record the failed upgrade: {e}");
    }
}

/// Finish or undo an upgrade a crash cut short. Returns what it did, if anything.
///
/// - `data` is missing but a new cluster and a backup exist: the crash came between the
///   two renames, so the new cluster becomes `data` (the backup stays).
/// - `global/pg_control` is missing next to `pg_control.old`: `pg_upgrade --link` was cut
///   off after it disabled the old cluster, before the new one ever started, so the old
///   control file is put back.
pub fn recover(layout: &Layout) -> Result<Option<String>> {
    let data = layout.data();
    if !data.exists() {
        let mut news = sibling_dirs(&layout.root, ".new");
        let backups = sibling_dirs(&layout.root, ".bak");
        if let (Some(new), false) = (news.pop(), backups.is_empty()) {
            std::fs::rename(&new, &data)?;
            return Ok(Some(format!("finished an interrupted upgrade: {} is now the data folder", new.display())));
        }
        return Ok(None);
    }
    let control = data.join("global").join("pg_control");
    let old = data.join("global").join("pg_control.old");
    if !control.exists() && old.exists() {
        std::fs::rename(&old, &control)?;
        return Ok(Some("put back the old cluster's pg_control after an interrupted pg_upgrade".into()));
    }
    Ok(None)
}

/// `data.pg*<suffix>` folders, sorted by name.
fn sibling_dirs(root: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("data.pg") && n.ends_with(suffix)))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// A Postgres program to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    /// The executable (`.../bin/pg_upgrade`).
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// Extra environment (`PGPASSWORD`).
    pub env: Vec<(String, String)>,
    /// The working folder (`pg_upgrade` writes its logs there).
    pub cwd: PathBuf,
}

impl Cmd {
    /// The program's file name without `.exe` (`pg_upgrade`), for logs and tests.
    pub fn name(&self) -> String {
        self.program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Whether `arg` is one of the arguments.
    pub fn has(&self, arg: &str) -> bool {
        self.args.iter().any(|a| a == arg)
    }
}

/// The side effects of an upgrade: programs, downloads and the new server.
/// `pgstore` runs real ones; tests use a fake that records the calls.
pub trait Runner {
    /// Run a Postgres program to the end; its stdout, or an error with its output.
    fn exec(&mut self, cmd: &Cmd) -> Result<String>;
    /// Download the old server `version` (pinned SHA-256) into the install folder.
    fn fetch(&mut self, version: &str) -> Result<()>;
    /// Start the new server on the (promoted) data folder.
    fn start(&mut self) -> Result<()>;
    /// Run `sql` on the new server's app database; the first column of the first row as
    /// text (empty when there is none).
    fn query(&mut self, sql: &str) -> Result<String>;
    /// Stop the new server.
    fn stop(&mut self) -> Result<()>;
}

/// Runs programs with `std::process::Command` (what `pgstore` uses for [`Runner::exec`]).
pub fn exec_command(cmd: &Cmd) -> Result<String> {
    let mut c = std::process::Command::new(&cmd.program);
    c.args(&cmd.args).current_dir(&cmd.cwd);
    for (k, v) in &cmd.env {
        c.env(k, v);
    }
    let out = c.output().map_err(|e| Error::Store(format!("{}: {e}", cmd.program.display())))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(Error::Store(format!("{} failed ({}): {}{}", cmd.name(), out.status, stdout.trim(), if stderr.trim().is_empty() { String::new() } else { format!("\n{}", stderr.trim()) })))
}

/// Everything [`execute`] needs besides the steps.
#[derive(Debug, Clone)]
pub struct Job {
    /// The server's folders.
    pub layout: Layout,
    /// The old major.
    pub from: u32,
    /// The new (bundled) major.
    pub to: u32,
    /// The old server's `bin` folder.
    pub old_bin: PathBuf,
    /// The new server's `bin` folder.
    pub new_bin: PathBuf,
    /// The superuser's password (also in `password_file`).
    pub password: String,
    /// The file `initdb --pwfile` reads.
    pub password_file: PathBuf,
    /// Server options for the old server (`-c shared_preload_libraries=...`), may be empty.
    pub old_options: String,
    /// Server options for the new server, may be empty.
    pub new_options: String,
    /// Extensions to update (`vector`, and `pg_tokenizer`, `vchord_bm25` with BM25).
    pub extensions: Vec<&'static str>,
    /// `--link` or `--copy` (the same as the plan's [`Step::Upgrade`]).
    pub method: Method,
    /// A short folder for `pg_upgrade`'s sockets (Unix paths are limited to about 100 bytes).
    pub socket_dir: PathBuf,
}

/// An upgrade that did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The old cluster is back in `data` and can run on the old server.
    pub rolled_back: bool,
    /// What failed, and where the folders are.
    pub message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a step is called in the progress line.
fn label(step: &Step) -> &'static str {
    match step {
        Step::FetchOld { .. } => "downloading the old server",
        Step::ReadLocale => "reading the old cluster",
        Step::InitNew => "making the new cluster",
        Step::Check => "checking (pg_upgrade --check)",
        Step::Upgrade(_) => "upgrading (pg_upgrade)",
        Step::Promote => "switching folders",
        Step::Start => "starting the new server",
        Step::Smoke => "testing the new server",
        Step::UpdateExtensions => "updating extensions",
        Step::Analyze => "updating statistics",
        Step::RemoveBackup => "removing the old folder",
        Step::RemoveNew => "cleaning up",
    }
}

/// Run `steps` for `job`. Progress lines start with "Upgrading database". On failure the
/// folders are rolled back where that is safe (see the module docs) and the error is
/// recorded for [`last_failure`].
pub fn execute(job: &Job, steps: &[Step], runner: &mut dyn Runner, progress: &mut dyn FnMut(&str)) -> std::result::Result<(), Failure> {
    progress(&format!("Upgrading database from Postgres {} to {}", job.from, job.to));
    let mut locale = Locale::default();
    let mut started = false;
    for (i, step) in steps.iter().enumerate() {
        progress(&format!("Upgrading database: {} ({}/{})", label(step), i + 1, steps.len()));
        log::info!("postgres upgrade {} -> {}: {:?}", job.from, job.to, step);
        // A start that fails may still have written to the new cluster, so it counts.
        started |= *step == Step::Start;
        let result = run_step(job, step, runner, &mut locale);
        if let Err(e) = result {
            let failure = fail(job, step, started, runner, &e.to_string());
            log::error!("{}", failure.message);
            record_failure(&job.layout, &failure.message);
            return Err(failure);
        }
    }
    let _ = clear_failure(&job.layout);
    Ok(())
}

/// The old cluster's encoding and locale, which the new one must match.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Locale {
    encoding: String,
    collate: String,
    ctype: String,
}

impl Default for Locale {
    fn default() -> Self {
        Self { encoding: "UTF8".into(), collate: String::new(), ctype: String::new() }
    }
}

fn bin(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn os(args: &[&dyn AsRef<std::ffi::OsStr>]) -> Vec<OsString> {
    args.iter().map(|a| a.as_ref().to_os_string()).collect()
}

/// A free TCP port on this computer.
fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn run_step(job: &Job, step: &Step, runner: &mut dyn Runner, locale: &mut Locale) -> Result<()> {
    let l = &job.layout;
    let env = vec![("PGPASSWORD".to_string(), job.password.clone())];
    let cmd = |program: PathBuf, args: Vec<OsString>| Cmd { program, args, env: env.clone(), cwd: l.root.clone() };
    match step {
        Step::FetchOld { version } => runner.fetch(version),
        Step::ReadLocale => {
            let port = free_port()?.to_string();
            let data = l.data();
            let opts = format!("-c listen_addresses=localhost -p {port} {}", job.old_options);
            runner.exec(&cmd(bin(&job.old_bin, "pg_ctl"), os(&[&"start", &"-w", &"-t", &"120", &"-D", &data, &"-l", &l.locale_log(), &"-o", &opts.trim()])))?;
            let sql = "select pg_encoding_to_char(encoding) || '|' || datcollate || '|' || datctype from pg_database where datname = 'template0'";
            let read = runner.exec(&cmd(
                bin(&job.old_bin, "psql"),
                os(&[&"-h", &"localhost", &"-p", &port, &"-U", &SUPERUSER, &"-d", &"postgres", &"-At", &"-c", &sql]),
            ));
            // Always stop it again (cleanly, which pg_upgrade needs), even when the query failed.
            let stop = runner.exec(&cmd(bin(&job.old_bin, "pg_ctl"), os(&[&"stop", &"-w", &"-t", &"120", &"-m", &"fast", &"-D", &data])));
            let line = read?;
            stop?;
            *locale = parse_locale(&line).ok_or_else(|| Error::Store(format!("could not read the old cluster's locale from {line:?}")))?;
            Ok(())
        }
        Step::InitNew => {
            let new = l.new_data(job.to);
            if new.exists() {
                // A check or an attempt that stopped half-way: only ours, never the user's data.
                std::fs::remove_dir_all(&new)?;
            }
            if !job.password_file.exists() {
                std::fs::write(&job.password_file, &job.password)?;
            }
            let mut args = os(&[&"-D", &new, &"-U", &SUPERUSER, &"-A", &"password", &"--pwfile", &job.password_file, &"-E", &locale.encoding]);
            if !locale.collate.is_empty() {
                args.push(format!("--lc-collate={}", locale.collate).into());
            }
            if !locale.ctype.is_empty() {
                args.push(format!("--lc-ctype={}", locale.ctype).into());
            }
            runner.exec(&cmd(bin(&job.new_bin, "initdb"), args)).map(|_| ())
        }
        Step::Check => runner.exec(&cmd(bin(&job.new_bin, "pg_upgrade"), upgrade_args(job, true, job.method)?)).map(|_| ()),
        Step::Upgrade(method) => runner.exec(&cmd(bin(&job.new_bin, "pg_upgrade"), upgrade_args(job, false, *method)?)).map(|_| ()),
        Step::Promote => {
            let backup = free_name(l.backup(job.from));
            std::fs::rename(l.data(), &backup)?;
            if let Err(e) = std::fs::rename(l.new_data(job.to), l.data()) {
                // Put the old one back before reporting.
                let _ = std::fs::rename(&backup, l.data());
                return Err(e.into());
            }
            Ok(())
        }
        Step::Start => runner.start(),
        Step::Smoke => {
            let n = runner.query("select count(*) from pg_database")?;
            if n.trim().parse::<u32>().map_or(true, |n| n == 0) {
                return Err(Error::Store(format!("smoke query returned {n:?}")));
            }
            // The app's own table, when it has one, must read.
            if runner.query("select (to_regclass('chunks') is not null)::text")? == "true" {
                runner.query("select count(*) from chunks")?;
            }
            Ok(())
        }
        Step::UpdateExtensions => {
            for ext in &job.extensions {
                if runner.query(&format!("select count(*) from pg_extension where extname = '{ext}'"))?.trim() == "1" {
                    runner.query(&format!("alter extension \"{ext}\" update"))?;
                }
            }
            Ok(())
        }
        Step::Analyze => runner.query("analyze").map(|_| ()),
        Step::RemoveBackup => {
            let backup = l.backup(job.from);
            if backup.exists() {
                std::fs::remove_dir_all(&backup)?;
            }
            Ok(())
        }
        Step::RemoveNew => {
            let new = l.new_data(job.to);
            if new.exists() {
                std::fs::remove_dir_all(&new)?;
            }
            Ok(())
        }
    }
}

/// `path`, or `path.2`, `path.3`, ... when it is taken (a backup is never overwritten).
fn free_name(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    (2..).map(|n| PathBuf::from(format!("{}.{n}", path.display()))).find(|p| !p.exists()).unwrap_or(path)
}

fn parse_locale(line: &str) -> Option<Locale> {
    let mut parts = line.trim().splitn(3, '|');
    let (encoding, collate, ctype) = (parts.next()?, parts.next()?, parts.next()?);
    (!encoding.is_empty()).then(|| Locale { encoding: encoding.into(), collate: collate.into(), ctype: ctype.into() })
}

fn upgrade_args(job: &Job, check: bool, method: Method) -> Result<Vec<OsString>> {
    let l = &job.layout;
    let (p_old, p_new) = (free_port()?.to_string(), free_port()?.to_string());
    let mut args = os(&[
        &"-b",
        &job.old_bin,
        &"-B",
        &job.new_bin,
        &"-d",
        &l.data(),
        &"-D",
        &l.new_data(job.to),
        &"-U",
        &SUPERUSER,
        &"-p",
        &p_old,
        &"-P",
        &p_new,
    ]);
    args.push(method.flag().into());
    if check {
        args.push("--check".into());
    }
    if !job.old_options.is_empty() {
        args.extend(["-o".into(), job.old_options.clone().into()]);
    }
    if !job.new_options.is_empty() {
        args.extend(["-O".into(), job.new_options.clone().into()]);
    }
    if cfg!(unix) {
        std::fs::create_dir_all(&job.socket_dir)?;
        args.extend(["-s".into(), job.socket_dir.clone().into_os_string()]);
    }
    Ok(args)
}

/// Undo what can safely be undone after `step` failed.
fn fail(job: &Job, step: &Step, started: bool, runner: &mut dyn Runner, error: &str) -> Failure {
    let l = &job.layout;
    let backup = l.backup(job.from);
    let what = format!("Upgrading the database from Postgres {} to {} failed at \"{}\": {error}", job.from, job.to, label(step));
    if !started && !backup.exists() {
        // Before the folders were switched: the old cluster is still `data`.
        let _ = std::fs::remove_dir_all(l.new_data(job.to));
        let _ = recover(l);
        return Failure { rolled_back: true, message: format!("{what}. The database is as it was and still runs on Postgres {}.", job.from) };
    }
    if started {
        let _ = runner.stop();
    }
    if matches!(step, Step::RemoveBackup) {
        // The new server is fine; only the old folder is left over.
        return Failure { rolled_back: false, message: format!("{what}. The upgrade itself worked; remove {} by hand.", backup.display()) };
    }
    let link = job.method == Method::Link;
    if started && link {
        return Failure {
            rolled_back: false,
            message: format!(
                "{what}. The new cluster is in {} and the old one is kept in {} (after --link the old one shares files with a server that started, so restore it from a backup rather than starting it).",
                l.data().display(),
                backup.display()
            ),
        };
    }
    // --copy, or the switch itself failed: the old folder goes back.
    let failed = free_name(l.failed(job.to));
    let moved = !l.data().exists() || std::fs::rename(l.data(), &failed).is_ok();
    if moved && std::fs::rename(&backup, l.data()).is_ok() {
        let _ = recover(l);
        return Failure {
            rolled_back: true,
            message: format!("{what}. The old database is back in {} and still runs on Postgres {}; the new one is in {}.", l.data().display(), job.from, failed.display()),
        };
    }
    Failure { rolled_back: false, message: format!("{what}. The old database is kept in {}.", backup.display()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every call; `fail` names the call that errs (`pg_upgrade`, `pg_upgrade --check`,
    /// `start`, `query`, ...). `initdb` makes the new cluster's folder, as the real one does.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        fail: Option<&'static str>,
    }

    impl Fake {
        fn call(&mut self, name: String) -> Result<()> {
            let failing = self.fail.is_some_and(|f| f == name);
            self.calls.push(name.clone());
            if failing {
                return Err(Error::Store(format!("{name} failed")));
            }
            Ok(())
        }
    }

    impl Runner for Fake {
        fn exec(&mut self, cmd: &Cmd) -> Result<String> {
            let name = match cmd.name().as_str() {
                "pg_upgrade" if cmd.has("--check") => "pg_upgrade --check".to_string(),
                "pg_ctl" => format!("pg_ctl {}", cmd.args[0].to_string_lossy()),
                n => n.to_string(),
            };
            self.call(name.clone())?;
            match name.as_str() {
                "initdb" => {
                    let dir = PathBuf::from(&cmd.args[1]);
                    std::fs::create_dir_all(dir.join("global"))?;
                    std::fs::write(dir.join("PG_VERSION"), "16\n")?;
                    Ok(String::new())
                }
                "psql" => Ok("UTF8|C|C\n".into()),
                _ => Ok(String::new()),
            }
        }
        fn fetch(&mut self, version: &str) -> Result<()> {
            self.call(format!("fetch {version}"))
        }
        fn start(&mut self) -> Result<()> {
            self.call("start".into())
        }
        fn query(&mut self, sql: &str) -> Result<String> {
            self.call("query".into())?;
            Ok(if sql.contains("pg_database") || sql.contains("pg_extension") {
                "1".into()
            } else if sql.contains("to_regclass") {
                "true".into()
            } else {
                String::new()
            })
        }
        fn stop(&mut self) -> Result<()> {
            self.call("stop".into())
        }
    }

    fn cluster(dir: &Path, major: u32) {
        std::fs::create_dir_all(dir.join("global")).unwrap();
        std::fs::write(dir.join("PG_VERSION"), format!("{major}\n")).unwrap();
        std::fs::write(dir.join("global/pg_control"), b"control").unwrap();
        std::fs::write(dir.join("user-rows"), b"precious").unwrap();
    }

    fn job(root: &Path, method: Method) -> Job {
        Job {
            layout: Layout { root: root.to_path_buf() },
            from: 15,
            to: 16,
            old_bin: root.join("install/15.20.0/bin"),
            new_bin: root.join("install/16.16.0/bin"),
            password: "secret".into(),
            password_file: root.join(".pgpass"),
            old_options: String::new(),
            new_options: String::new(),
            extensions: vec!["vector", "vchord_bm25"],
            method,
            socket_dir: root.join("sock"),
        }
    }

    fn steps(method: Method, check_only: bool) -> Vec<Step> {
        plan(&PlanOptions { old_version: "15.20.0".into(), old_cached: true, method, check_only })
    }

    #[test]
    fn version_detection() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        assert_eq!(need(&data, 16).unwrap(), Need::Fresh);
        cluster(&data, 16);
        assert_eq!(need(&data, 16).unwrap(), Need::Current);
        std::fs::write(data.join("PG_VERSION"), "15\n").unwrap();
        assert_eq!(need(&data, 16).unwrap(), Need::Upgrade { from: 15, to: 16 });
        std::fs::write(data.join("PG_VERSION"), "17").unwrap();
        assert_eq!(need(&data, 16).unwrap(), Need::Newer { found: 17, bundled: 16 });
        std::fs::write(data.join("PG_VERSION"), "9.6").unwrap();
        assert!(need(&data, 16).is_err(), "majors before 10 are refused, not guessed");
        assert_eq!(parse_major(" 16.16.0 "), Some(16));
        assert_eq!(parse_major("x"), None);
    }

    #[test]
    fn plans_check_before_upgrade_and_backup_removal_last() {
        let full = steps(Method::Link, false);
        assert_eq!(
            full,
            [
                Step::ReadLocale,
                Step::InitNew,
                Step::Check,
                Step::Upgrade(Method::Link),
                Step::Promote,
                Step::Start,
                Step::Smoke,
                Step::UpdateExtensions,
                Step::Analyze,
                Step::RemoveBackup
            ]
        );
        assert!(steps(Method::Copy, false).contains(&Step::Upgrade(Method::Copy)));
        let fetch = plan(&PlanOptions { old_version: "15.20.0".into(), old_cached: false, ..PlanOptions::default() });
        assert_eq!(fetch[0], Step::FetchOld { version: "15.20.0".into() });
        // A check changes nothing: no upgrade, no switch, and the new cluster goes again.
        let check = steps(Method::Link, true);
        assert!(!check.iter().any(|s| matches!(s, Step::Upgrade(_) | Step::Promote | Step::Start | Step::RemoveBackup)));
        assert_eq!(check.last(), Some(&Step::RemoveNew));
    }

    #[test]
    fn pinned_old_servers() {
        let old = old_server(15).unwrap();
        for target in ["aarch64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"] {
            let h = old.sha256_for(target).unwrap();
            assert!(h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert!(old_server(12).is_none());
    }

    #[test]
    fn finds_the_newest_cached_server_of_a_major() {
        let dir = tempfile::tempdir().unwrap();
        for v in ["15.2.0", "15.20.0", "16.16.0", "15.9.0"] {
            std::fs::create_dir_all(dir.path().join(v).join("bin")).unwrap();
            std::fs::write(bin(&dir.path().join(v).join("bin"), "pg_ctl"), b"").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("15.30.0")).unwrap(); // no binaries: not a server
        assert_eq!(installed_version(dir.path(), 15).as_deref(), Some("15.20.0"));
        assert_eq!(installed_version(dir.path(), 14), None);
    }

    #[test]
    fn runs_every_step_and_switches_folders() {
        let dir = tempfile::tempdir().unwrap();
        let j = job(dir.path(), Method::Link);
        cluster(&j.layout.data(), 15);
        let mut fake = Fake::default();
        let mut lines = Vec::new();
        execute(&j, &steps(Method::Link, false), &mut fake, &mut |l| lines.push(l.to_string())).unwrap();
        let names: Vec<&str> = fake.calls.iter().map(String::as_str).collect();
        assert_eq!(&names[..6], ["pg_ctl start", "psql", "pg_ctl stop", "initdb", "pg_upgrade --check", "pg_upgrade"]);
        assert_eq!(names[6], "start");
        assert_eq!(data_major(&j.layout.data()).unwrap(), Some(16));
        assert!(!j.layout.backup(15).exists(), "the old folder goes once the new one passed");
        assert!(!j.layout.new_data(16).exists());
        assert!(lines.iter().all(|l| l.starts_with("Upgrading database")));
        assert_eq!(last_failure(&j.layout), None);
    }

    #[test]
    fn initdb_matches_the_old_locale_and_password() {
        let dir = tempfile::tempdir().unwrap();
        let j = job(dir.path(), Method::Copy);
        cluster(&j.layout.data(), 15);
        struct Spy(Fake, Vec<Cmd>);
        impl Runner for Spy {
            fn exec(&mut self, cmd: &Cmd) -> Result<String> {
                self.1.push(cmd.clone());
                self.0.exec(cmd)
            }
            fn fetch(&mut self, v: &str) -> Result<()> {
                self.0.fetch(v)
            }
            fn start(&mut self) -> Result<()> {
                self.0.start()
            }
            fn query(&mut self, sql: &str) -> Result<String> {
                self.0.query(sql)
            }
            fn stop(&mut self) -> Result<()> {
                self.0.stop()
            }
        }
        let mut spy = Spy(Fake::default(), Vec::new());
        execute(&j, &steps(Method::Copy, true), &mut spy, &mut |_| {}).unwrap();
        let initdb = spy.1.iter().find(|c| c.name() == "initdb").unwrap();
        assert!(initdb.has("--lc-collate=C") && initdb.has("--lc-ctype=C") && initdb.has("UTF8") && initdb.has("postgres"));
        assert_eq!(std::fs::read_to_string(&j.password_file).unwrap(), "secret");
        let check = spy.1.iter().find(|c| c.name() == "pg_upgrade").unwrap();
        assert!(check.has("--check") && check.has("--copy"));
        assert!(check.env.iter().any(|(k, v)| k == "PGPASSWORD" && v == "secret"));
        // A check leaves the data as it was.
        assert_eq!(data_major(&j.layout.data()).unwrap(), Some(15));
        assert!(!j.layout.new_data(16).exists());
    }

    #[test]
    fn a_failed_pg_upgrade_rolls_back_and_is_remembered() {
        for failing in ["pg_upgrade --check", "pg_upgrade", "initdb", "psql"] {
            let dir = tempfile::tempdir().unwrap();
            let j = job(dir.path(), Method::Link);
            cluster(&j.layout.data(), 15);
            let mut fake = Fake { fail: Some(failing), ..Fake::default() };
            let f = execute(&j, &steps(Method::Link, false), &mut fake, &mut |_| {}).unwrap_err();
            assert!(f.rolled_back, "{failing}: {}", f.message);
            assert_eq!(data_major(&j.layout.data()).unwrap(), Some(15), "{failing}");
            assert_eq!(std::fs::read(j.layout.data().join("user-rows")).unwrap(), b"precious");
            assert!(!j.layout.new_data(16).exists());
            assert!(!fake.calls.contains(&"start".to_string()));
            assert!(last_failure(&j.layout).unwrap().contains(failing));
            if failing == "psql" {
                assert_eq!(fake.calls.last().map(String::as_str), Some("pg_ctl stop"), "the old server is stopped again");
            }
            clear_failure(&j.layout).unwrap();
            assert_eq!(last_failure(&j.layout), None);
        }
    }

    #[test]
    fn link_mode_keeps_the_backup_when_the_new_server_fails() {
        let dir = tempfile::tempdir().unwrap();
        let j = job(dir.path(), Method::Link);
        cluster(&j.layout.data(), 15);
        let mut fake = Fake { fail: Some("query"), ..Fake::default() };
        let f = execute(&j, &steps(Method::Link, false), &mut fake, &mut |_| {}).unwrap_err();
        assert!(!f.rolled_back);
        assert!(f.message.contains("data.pg15.bak"), "{}", f.message);
        assert_eq!(std::fs::read(j.layout.backup(15).join("user-rows")).unwrap(), b"precious");
        assert_eq!(fake.calls.last().map(String::as_str), Some("stop"));
    }

    #[test]
    fn copy_mode_rolls_back_when_the_new_server_fails() {
        let dir = tempfile::tempdir().unwrap();
        let j = job(dir.path(), Method::Copy);
        cluster(&j.layout.data(), 15);
        let mut fake = Fake { fail: Some("start"), ..Fake::default() };
        let f = execute(&j, &steps(Method::Copy, false), &mut fake, &mut |_| {}).unwrap_err();
        assert!(f.rolled_back, "{}", f.message);
        assert_eq!(data_major(&j.layout.data()).unwrap(), Some(15));
        assert_eq!(std::fs::read(j.layout.data().join("user-rows")).unwrap(), b"precious");
        assert_eq!(data_major(&j.layout.failed(16)).unwrap(), Some(16), "the new cluster is set aside, not deleted");
    }

    #[test]
    fn recovers_from_a_crash_between_the_renames() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout { root: dir.path().to_path_buf() };
        cluster(&l.backup(15), 15);
        cluster(&l.new_data(16), 16);
        assert!(recover(&l).unwrap().is_some());
        assert_eq!(data_major(&l.data()).unwrap(), Some(16));
        assert!(l.backup(15).exists(), "the backup stays");
        assert_eq!(recover(&l).unwrap(), None);
    }

    #[test]
    fn recovers_pg_control_after_an_interrupted_link() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout { root: dir.path().to_path_buf() };
        cluster(&l.data(), 15);
        std::fs::rename(l.data().join("global/pg_control"), l.data().join("global/pg_control.old")).unwrap();
        assert!(recover(&l).unwrap().is_some());
        assert!(l.data().join("global/pg_control").exists());
    }

    #[test]
    fn a_backup_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("data.pg15.bak");
        std::fs::create_dir(&taken).unwrap();
        assert_eq!(free_name(taken.clone()), dir.path().join("data.pg15.bak.2"));
    }

    #[test]
    fn modes_read_as_lowercase_with_ask_by_default() {
        assert_eq!(UpgradeMode::default(), UpgradeMode::Ask);
        assert_eq!(serde_json::from_str::<UpgradeMode>("\"off\"").unwrap(), UpgradeMode::Off);
        assert_eq!(serde_json::to_string(&UpgradeMode::Auto).unwrap(), "\"auto\"");
    }
}
