//! Cloud sync with [rclone](https://rclone.org): copy the backups folder (or the data
//! folder) to any of the 70+ storage services rclone speaks, on a schedule or on demand.
//!
//! The rclone binary is not bundled: a pinned release ([`VERSION`]) is downloaded on first
//! use through [`crate::net::download_to`] (so the network policy applies), checked against
//! its SHA-256 and kept in `<data>/rclone/`. The remote's settings (`rclone.conf`, which
//! holds tokens and passwords) live in an owner-only file there too, never in settings,
//! and are never synced. Settings say which remote and path, the direction, how often and
//! how fast; [`args`] turns them into rclone's command line (no shell is involved).
//!
//! The data folder only goes one way (up): pulling into a running app's store would pull
//! the floor from under it. Secrets in it (keys, passwords, the rclone config), models,
//! logs and the Postgres cluster are left out; backups are the safe way to restore.
//!
//! ```
//! use desktop_core::rclone::{args, Direction, Folder, RcloneConfig};
//! use std::path::Path;
//!
//! let config = RcloneConfig {
//!     enabled: true,
//!     remote: "gdrive".into(),
//!     path: "Apps/MyApp".into(),
//!     folder: Folder::Backups,
//!     direction: Direction::Push,
//!     bandwidth: "2M".into(),
//!     ..RcloneConfig::default()
//! }
//! .validated();
//! let argv = args(&config, Path::new("/data/backups"), Path::new("/data/rclone/rclone.conf"), false).unwrap();
//! let argv: Vec<String> = argv.iter().map(|a| a.to_string_lossy().into_owned()).collect();
//! assert_eq!(&argv[..4], ["copy", "--config", "/data/rclone/rclone.conf", "--bwlimit"]);
//! assert!(argv.ends_with(&["--".into(), "/data/backups".into(), "gdrive:Apps/MyApp".into()]));
//! ```

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The rclone release this build downloads.
pub const VERSION: &str = "v1.75.2";

/// Where to get rclone.
pub const DOWNLOADS: &str = "https://downloads.rclone.org";

/// (os, arch, SHA-256 of `rclone-<VERSION>-<os>-<arch>.zip`), from the release's SHA256SUMS.
const BUILDS: [(&str, &str, &str); 6] = [
    ("linux", "amd64", "349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702"),
    ("linux", "arm64", "7e1e8d69654941b7b7df84ee74c5f7fb09cce7d5496947fb7bf16b55ff427d10"),
    ("osx", "amd64", "574e1f31cf1c464e5fb0325cb66fcbdeaf1efa1802bd9b17f13c256fd74b6561"),
    ("osx", "arm64", "610ab7d48615135dfc60b763858b11a426a0d3d1160c1bb76fb0ae6011b02edd"),
    ("windows", "amd64", "8a9db2bad1dc79a65b3348fd857069598e2aab7cf4d118b77b1df23055aff818"),
    ("windows", "arm64", "7eb9844c9e5b723b9ef28821891caa17f6d9d1a375c49365cbe04114cec8dcd9"),
];

/// Left out when the data folder is synced: secrets, the rclone folder itself, the live
/// Postgres cluster, models (downloaded again on demand), logs and partial downloads.
pub const DATA_EXCLUDES: [&str; 9] =
    ["/rclone/**", "/postgres/**", "/models/**", "/logs/**", "/backup.key", "/mail-password", "/keys.json", "*.part", "*.tmp"];

#[derive(Debug, thiserror::Error)]
pub enum RcloneError {
    #[error("{0}")]
    Config(String),
    #[error("rclone is not available for this computer ({0})")]
    Unsupported(String),
    #[error("download: {0}")]
    Download(String),
    #[error("rclone failed: {0}")]
    Failed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = RcloneError> = std::result::Result<T, E>;

/// Which way files go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Copy new and changed files up; nothing is deleted (`rclone copy`).
    #[default]
    Push,
    /// Copy new and changed files down; nothing is deleted (backups folder only).
    Pull,
    /// Make the remote an exact copy, deleting what is not here (`rclone sync`).
    Mirror,
    /// Two-way sync (`rclone bisync`; backups folder only). The first run sets the baseline.
    Bisync,
}

/// What is synced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Folder {
    /// The backups folder (`backup.folder`, or `<data>/backups`).
    #[default]
    Backups,
    /// The app's data folder, minus [`DATA_EXCLUDES`]; up only.
    Data,
}

/// Cloud sync settings (`rclone` in settings). Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RcloneConfig {
    pub enabled: bool,
    /// The remote's name in `rclone.conf`, e.g. `gdrive` (without the colon).
    pub remote: String,
    /// The folder on the remote (empty = its root).
    pub path: String,
    pub folder: Folder,
    pub direction: Direction,
    /// Sync every this many minutes (0 = only when asked; otherwise at least 5).
    pub interval_minutes: u32,
    /// rclone's `--bwlimit`: e.g. `1M`, `512k`, `10M:2M` (up:down); empty = no limit.
    pub bandwidth: String,
}

impl Default for RcloneConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            remote: String::new(),
            path: String::new(),
            folder: Folder::Backups,
            direction: Direction::Push,
            interval_minutes: 0,
            bandwidth: String::new(),
        }
    }
}

impl RcloneConfig {
    /// Trimmed, in range, and safe: the data folder only goes up, and a bandwidth rclone
    /// would not understand is dropped.
    pub fn validated(mut self) -> Self {
        self.remote = self.remote.trim().trim_end_matches(':').to_string();
        self.path = self.path.trim().trim_matches('/').to_string();
        self.bandwidth = self.bandwidth.trim().to_string();
        if !valid_bandwidth(&self.bandwidth) {
            self.bandwidth.clear();
        }
        if self.interval_minutes > 0 {
            self.interval_minutes = self.interval_minutes.clamp(5, 7 * 24 * 60);
        }
        if self.folder == Folder::Data && matches!(self.direction, Direction::Pull | Direction::Bisync) {
            self.direction = Direction::Push;
        }
        self
    }

    /// `remote:path`, after checking the remote's name.
    pub fn target(&self) -> Result<String> {
        if self.remote.is_empty() {
            return Err(RcloneError::Config("choose a remote first (its name in the rclone config)".into()));
        }
        let ok_name = self.remote.chars().all(|c| c.is_alphanumeric() || "_-.+@ ".contains(c));
        if !ok_name || self.remote.starts_with(['-', ' ']) {
            return Err(RcloneError::Config(format!("{:?} is not a remote name", self.remote)));
        }
        if self.path.chars().any(char::is_control) {
            return Err(RcloneError::Config("the remote path has a control character".into()));
        }
        Ok(format!("{}:{}", self.remote, self.path))
    }
}

/// Whether `s` is an `--bwlimit` value: empty, `off`, or `<rate>` / `<up>:<down>` where a
/// rate is a number with an optional `B`, `K`, `M`, `G`, `T` or `P` suffix.
pub fn valid_bandwidth(s: &str) -> bool {
    let rate = |r: &str| {
        let num = r.trim_end_matches(|c: char| "bBkKmMgGtTpP".contains(c));
        r.len() - num.len() <= 1 && !num.is_empty() && num.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.0)
    };
    s.is_empty() || s == "off" || (s.split(':').count() <= 2 && s.split(':').all(rate))
}

/// rclone's command line for one sync of `local` (no program name). `first_bisync` adds
/// `--resync`, which a bisync needs the first time (or after the target changes).
pub fn args(cfg: &RcloneConfig, local: &Path, conf: &Path, first_bisync: bool) -> Result<Vec<OsString>> {
    let target = cfg.target()?;
    let verb = match cfg.direction {
        Direction::Push | Direction::Pull => "copy",
        Direction::Mirror => "sync",
        Direction::Bisync => "bisync",
    };
    let mut a: Vec<OsString> = vec![verb.into(), "--config".into(), conf.into()];
    if !cfg.bandwidth.is_empty() {
        a.extend(["--bwlimit".into(), cfg.bandwidth.clone().into()]);
    }
    // One stats line every few seconds, at a level shown without -v.
    a.extend(["--stats", "5s", "--stats-one-line", "--stats-log-level", "NOTICE"].map(OsString::from));
    if cfg.folder == Folder::Data {
        for x in DATA_EXCLUDES {
            a.extend(["--exclude".into(), x.into()]);
        }
    }
    if cfg.direction == Direction::Bisync && first_bisync {
        a.push("--resync".into());
    }
    // Nothing after `--` is read as a flag.
    a.push("--".into());
    let (src, dst): (OsString, OsString) = match cfg.direction {
        Direction::Pull => (target.into(), local.into()),
        _ => (local.into(), target.into()),
    };
    a.extend([src, dst]);
    Ok(a)
}

/// `<data>/rclone`: the binary, its config, and the last run.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("rclone")
}

/// The downloaded binary's path (it may not be there yet).
pub fn binary(data_dir: &Path) -> PathBuf {
    dir(data_dir).join(VERSION).join(if cfg!(windows) { "rclone.exe" } else { "rclone" })
}

/// This computer's `(os, arch, sha256)` in rclone's naming, if rclone builds for it.
pub fn platform() -> Option<(&'static str, &'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "macos" => "osx",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    BUILDS.into_iter().find(|(o, a, _)| *o == os && *a == arch)
}

/// Download, check and unpack the pinned rclone (once); returns the binary's path.
/// `progress(bytes_so_far, total)` runs during the download.
pub fn install(data_dir: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    use sha2::Digest;
    let bin = binary(data_dir);
    if bin.is_file() {
        return Ok(bin);
    }
    let (os, arch, sha) =
        platform().ok_or_else(|| RcloneError::Unsupported(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)))?;
    let name = format!("rclone-{VERSION}-{os}-{arch}");
    let zip_path = dir(data_dir).join(format!("{name}.zip"));
    crate::net::download_to(&format!("{DOWNLOADS}/{VERSION}/{name}.zip"), &zip_path, progress).map_err(RcloneError::Download)?;
    let got = format!("{:x}", sha2::Sha256::digest(std::fs::read(&zip_path)?));
    if got != sha {
        let _ = std::fs::remove_file(&zip_path);
        return Err(RcloneError::Download(format!("{name}.zip failed its checksum")));
    }
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path)?).map_err(|e| RcloneError::Download(e.to_string()))?;
    let want = bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = bin.with_extension("unpack");
    std::fs::create_dir_all(bin.parent().unwrap_or(data_dir))?;
    let mut found = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| RcloneError::Download(e.to_string()))?;
        if entry.name().rsplit('/').next() == Some(want.as_str()) {
            std::io::copy(&mut entry, &mut std::fs::File::create(&tmp)?)?;
            found = true;
            break;
        }
    }
    let _ = std::fs::remove_file(&zip_path);
    if !found {
        return Err(RcloneError::Download(format!("{want} is not in {name}.zip")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &bin)?;
    Ok(bin)
}

/// The remote settings file (`rclone.conf`), readable by this user only.
pub struct ConfigFile(PathBuf);

impl ConfigFile {
    pub fn new(data_dir: &Path) -> Self {
        Self(dir(data_dir).join("rclone.conf"))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn exists(&self) -> bool {
        self.0.is_file()
    }
    /// Replace it with `text` (what `rclone config show` prints), or remove it when empty.
    pub fn set(&self, text: &str) -> std::io::Result<()> {
        if text.trim().is_empty() {
            return match std::fs::remove_file(&self.0) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        crate::jobs::write_private(&self.0, text.as_bytes())
    }
    /// The remotes it defines (its `[section]` names); never the secrets.
    pub fn remotes(&self) -> Vec<String> {
        std::fs::read_to_string(&self.0).map(|t| remotes_in(&t)).unwrap_or_default()
    }
}

/// The `[section]` names in an rclone config.
pub fn remotes_in(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix('[')?.strip_suffix(']').map(|n| n.trim().to_string())).filter(|n| !n.is_empty()).collect()
}

/// How the last sync went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastRun {
    /// RFC 3339, UTC.
    pub finished: String,
    pub ok: bool,
    /// rclone's last lines (its summary, or why it failed).
    pub message: String,
    pub target: String,
}

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub version: &'static str,
    pub installed: bool,
    /// rclone builds for this computer.
    pub supported: bool,
    pub configured: bool,
    pub remotes: Vec<String>,
    pub last: Option<LastRun>,
}

pub fn status(data_dir: &Path) -> Status {
    let conf = ConfigFile::new(data_dir);
    Status {
        version: VERSION,
        installed: binary(data_dir).is_file(),
        supported: platform().is_some(),
        configured: conf.exists(),
        remotes: conf.remotes(),
        last: std::fs::read(last_file(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()),
    }
}

fn last_file(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("last.json")
}

/// Whether a scheduled sync is due: on, with an interval, and the last one older than it.
pub fn due(data_dir: &Path, cfg: &RcloneConfig) -> bool {
    due_at(cfg, std::fs::metadata(last_file(data_dir)).and_then(|m| m.modified()).ok(), SystemTime::now())
}

fn due_at(cfg: &RcloneConfig, last: Option<SystemTime>, now: SystemTime) -> bool {
    let every = Duration::from_secs(u64::from(cfg.interval_minutes) * 60);
    cfg.enabled && cfg.interval_minutes > 0 && last.is_none_or(|t| now.duration_since(t).is_ok_and(|age| age >= every))
}

/// Run one sync of `local` now (the backups or data folder, as `cfg.folder` says),
/// installing rclone first if needed. rclone's output lines go to `progress`.
pub fn sync(data_dir: &Path, local: &Path, cfg: &RcloneConfig, progress: &mut dyn FnMut(&str)) -> Result<LastRun> {
    if crate::net::policy().offline {
        return Err(RcloneError::Config("offline mode is on (Settings → Network)".into()));
    }
    let conf = ConfigFile::new(data_dir);
    if !conf.exists() {
        return Err(RcloneError::Config("add the rclone config first (Settings → Cloud sync)".into()));
    }
    let target = cfg.target()?;
    std::fs::create_dir_all(local)?;
    let bin = install(data_dir, &mut |done, total| {
        progress(&match total {
            Some(t) => format!("Downloading rclone: {} of {} MB", done >> 20, t >> 20),
            None => format!("Downloading rclone: {} MB", done >> 20),
        })
    })?;
    let marker = dir(data_dir).join("bisync-baseline");
    let first = std::fs::read_to_string(&marker).ok().as_deref() != Some(target.as_str());
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args(cfg, local, conf.path(), first)?).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    crate::net::apply_env(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    log::info!("rclone {:?} {} -> {target}", cfg.direction, local.display());
    let mut child = cmd.spawn()?;
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    if let Some(err) = child.stderr.take() {
        for line in BufReader::new(err).lines() {
            let line = line?;
            progress(&line);
            tail.push_back(line);
            if tail.len() > 5 {
                tail.pop_front();
            }
        }
    }
    let ok = child.wait()?.success();
    let message = tail.into_iter().collect::<Vec<_>>().join("\n");
    let run = LastRun { finished: crate::logging::rfc3339(SystemTime::now()), ok, message: message.clone(), target: target.clone() };
    crate::jobs::write_atomic(&last_file(data_dir), &serde_json::to_vec_pretty(&run).map_err(std::io::Error::other)?)?;
    if !ok {
        return Err(RcloneError::Failed(if message.is_empty() { "no output".into() } else { message }));
    }
    if cfg.direction == Direction::Bisync && first {
        std::fs::write(&marker, &target)?;
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: Vec<OsString>) -> Vec<String> {
        a.into_iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    fn cfg(direction: Direction) -> RcloneConfig {
        RcloneConfig { enabled: true, remote: "s3".into(), path: "bucket/app".into(), direction, ..RcloneConfig::default() }
    }

    #[test]
    fn push_pull_mirror_and_bisync_commands() {
        let (local, conf) = (Path::new("/b"), Path::new("/c.conf"));
        let push = strings(args(&cfg(Direction::Push), local, conf, false).unwrap());
        assert_eq!(push[0], "copy");
        assert_eq!(push[push.len() - 3..], ["--", "/b", "s3:bucket/app"]);
        assert!(!push.contains(&"--bwlimit".into()));
        let pull = strings(args(&cfg(Direction::Pull), local, conf, false).unwrap());
        assert_eq!(pull[pull.len() - 3..], ["--", "s3:bucket/app", "/b"]);
        assert_eq!(strings(args(&cfg(Direction::Mirror), local, conf, false).unwrap())[0], "sync");
        let bisync = strings(args(&cfg(Direction::Bisync), local, conf, true).unwrap());
        assert_eq!(bisync[0], "bisync");
        assert!(bisync.contains(&"--resync".into()));
        assert!(!strings(args(&cfg(Direction::Bisync), local, conf, false).unwrap()).contains(&"--resync".into()));
    }

    #[test]
    fn bandwidth_and_data_excludes() {
        let c = RcloneConfig { folder: Folder::Data, bandwidth: " 10M:512k ".into(), ..cfg(Direction::Push) }.validated();
        let a = strings(args(&c, Path::new("/d"), Path::new("/c"), false).unwrap());
        let bw = a.iter().position(|x| x == "--bwlimit").unwrap();
        assert_eq!(a[bw + 1], "10M:512k");
        assert!(a.windows(2).any(|w| w == ["--exclude", "/backup.key"]));
        assert!(a.windows(2).any(|w| w == ["--exclude", "/rclone/**"]));
    }

    #[test]
    fn validation() {
        for ok in ["", "off", "1M", "0.5G", "512", "10M:2M", "1b"] {
            assert!(valid_bandwidth(ok), "{ok}");
        }
        for bad in ["fast", "1MB", "-1M", "1M:2M:3M", "M", "1M; rm -rf /", "nan"] {
            assert!(!valid_bandwidth(bad), "{bad}");
        }
        let c = RcloneConfig { remote: " gdrive: ".into(), path: "/a/b/".into(), bandwidth: "lots".into(), interval_minutes: 1, ..cfg(Direction::Push) }.validated();
        assert_eq!((c.remote.as_str(), c.path.as_str(), c.bandwidth.as_str(), c.interval_minutes), ("gdrive", "a/b", "", 5));
        assert_eq!(c.target().unwrap(), "gdrive:a/b");
        // The data folder never comes down.
        assert_eq!(RcloneConfig { folder: Folder::Data, ..cfg(Direction::Pull) }.validated().direction, Direction::Push);
        assert_eq!(RcloneConfig { folder: Folder::Data, ..cfg(Direction::Bisync) }.validated().direction, Direction::Push);
        assert_eq!(RcloneConfig { folder: Folder::Data, ..cfg(Direction::Mirror) }.validated().direction, Direction::Mirror);
    }

    #[test]
    fn remote_names_are_checked() {
        let bad = |remote: &str| RcloneConfig { remote: remote.into(), ..cfg(Direction::Push) }.validated().target().is_err();
        assert!(bad(""));
        assert!(bad("--config=/etc/x"));
        assert!(bad("/local/path"));
        assert!(bad("a:b"));
        assert!(!bad("my remote.2"));
        let ctl = RcloneConfig { path: "a\nb".into(), ..cfg(Direction::Push) };
        assert!(ctl.target().is_err());
    }

    #[test]
    fn schedule() {
        let now = SystemTime::now();
        let c = RcloneConfig { interval_minutes: 60, ..cfg(Direction::Push) };
        assert!(due_at(&c, None, now));
        assert!(!due_at(&c, Some(now - Duration::from_secs(600)), now));
        assert!(due_at(&c, Some(now - Duration::from_secs(3600)), now));
        assert!(!due_at(&RcloneConfig { enabled: false, ..c.clone() }, None, now));
        assert!(!due_at(&RcloneConfig { interval_minutes: 0, ..c }, None, now));
    }

    #[test]
    fn config_file_is_private_and_lists_remotes() {
        let tmp = tempfile::tempdir().unwrap();
        let f = ConfigFile::new(tmp.path());
        assert!(!f.exists() && f.remotes().is_empty());
        f.set("[gdrive]\ntype = drive\ntoken = {\"secret\":1}\n\n[ s3 ]\ntype = s3\n").unwrap();
        assert_eq!(f.remotes(), ["gdrive", "s3"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(f.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let st = status(tmp.path());
        assert!(st.configured && !st.installed && st.last.is_none());
        f.set("").unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn every_build_is_pinned() {
        for (os, arch, sha) in BUILDS {
            assert!(!os.is_empty() && !arch.is_empty());
            assert!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn sync_needs_a_config() {
        let tmp = tempfile::tempdir().unwrap();
        let e = sync(tmp.path(), &tmp.path().join("b"), &cfg(Direction::Push), &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("rclone config"), "{e}");
    }
}
