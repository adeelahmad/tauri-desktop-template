//! Update channels, staged rollouts and rollback bookkeeping for the Tauri updater
//! (feature `release-channels`).
//!
//! - **Channels.** The updater endpoints in `tauri.conf.json` carry a `{channel}` token
//!   (`.../releases/download/channel-{channel}/latest.json`). [`channel_endpoints`] fills it
//!   in with the channel picked in Settings ([`Channel::Stable`] or [`Channel::Beta`]). The
//!   release workflow keeps one `latest.json` per channel; a stable release is offered on
//!   both channels, a beta release (`v1.2.0-beta.1`) only on `beta`.
//! - **Staged rollout.** `latest.json` may carry `"rollout": 25` (percent). Each install
//!   draws a random id once ([`install_id`]) and maps it to a stable bucket from 0 to 99
//!   ([`bucket`]); an update is offered when the bucket is below the rollout
//!   ([`admitted`]). Raising the percentage later reaches more installs; the ones already
//!   admitted stay admitted.
//! - **Rollback.** Before an update installs, the app writes a [`RollbackRecord`] with the
//!   version it is leaving and, where the platform allows, a copy of its own bundle
//!   ([`bundle_to_keep`], [`keep_bundle`]). Rolling back restores that copy
//!   ([`restore_bundle`]) or, where there is none, installs the previous release's signed
//!   update bundle again ([`release_manifest_url`]).
//!
//! What each OS allows:
//!
//! | OS | Kept for rollback | Rollback |
//! |---|---|---|
//! | macOS | the whole `.app` bundle | restores the copy, offline |
//! | Linux (AppImage) | the `.AppImage` file | restores the copy, offline |
//! | Linux (.deb) | nothing (the package manager owns the files) | not offered: use `apt` |
//! | Windows (NSIS) | nothing (the installer replaces the files in place) | reinstalls the previous signed release (online) |
//!
//! ```
//! use desktop_core::release::{admitted, bucket, channel_endpoints, Channel};
//!
//! let urls = channel_endpoints(&["https://example.com/channel-{channel}/latest.json".to_string()], Channel::Beta);
//! assert_eq!(urls, ["https://example.com/channel-beta/latest.json"]);
//!
//! let b = bucket("0123456789abcdef0123456789abcdef");
//! assert!(b < 100);
//! assert_eq!(b, bucket("0123456789abcdef0123456789abcdef"), "the same install keeps its bucket");
//! assert!(admitted(b, None), "no rollout field: everyone gets it");
//! assert!(admitted(b, Some(100.0)));
//! assert!(!admitted(b, Some(0.0)));
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};

/// The token in an updater endpoint that [`channel_endpoints`] replaces.
pub const CHANNEL_TOKEN: &str = "{channel}";

/// File in the data folder holding this install's random id.
const INSTALL_ID_FILE: &str = "install-id";
/// File in the data folder recording what a rollback goes back to.
const ROLLBACK_FILE: &str = "rollback.json";
/// Folder in the data folder holding the kept bundle.
const ROLLBACK_DIR: &str = "rollback";

/// Which releases this install follows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Full releases only (the default).
    #[default]
    Stable,
    /// Pre-releases (`v1.2.0-beta.1`) and full releases, whichever is newer.
    Beta,
}

impl Channel {
    /// The name used in URLs and tags: `stable` or `beta`.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }
}

/// Settings section `updates`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatesConfig {
    /// The channel to take updates from.
    pub channel: Channel,
}

impl UpdatesConfig {
    /// The config with every value in range (a channel is always valid).
    pub fn validated(self) -> Self {
        self
    }
}

/// The endpoints with [`CHANNEL_TOKEN`] (also in its percent-encoded form, as a parsed URL
/// prints it) replaced by `channel`. Endpoints without the token are kept as they are.
pub fn channel_endpoints(endpoints: &[String], channel: Channel) -> Vec<String> {
    endpoints.iter().map(|e| e.replace(CHANNEL_TOKEN, channel.as_str()).replace("%7Bchannel%7D", channel.as_str())).collect()
}

/// This install's random id (32 hex characters), created on first use in `data_dir`.
///
/// It never leaves the computer: only the bucket it maps to is used, to decide locally
/// whether a staged update is offered.
pub fn install_id(data_dir: &Path) -> io::Result<String> {
    let path = data_dir.join(INSTALL_ID_FILE);
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if id.len() >= 16 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(id.to_string());
        }
    }
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).map_err(|_| io::Error::other("no system randomness"))?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(&path, &id)?;
    Ok(id)
}

/// The rollout bucket (0 to 99) of an install id: the same id always lands in the same
/// bucket, and ids spread evenly over the buckets.
pub fn bucket(install_id: &str) -> u8 {
    let digest = Sha256::digest(install_id.as_bytes());
    let n = u64::from_be_bytes([digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]]);
    (n % 100) as u8
}

/// Whether an install in `bucket` is offered a release rolled out to `rollout` percent.
/// No rollout (or 100 and above) means everyone; 0 and below means no one.
pub fn admitted(bucket: u8, rollout: Option<f64>) -> bool {
    match rollout {
        None => true,
        Some(p) if p.is_nan() => true,
        Some(p) => f64::from(bucket) < p.clamp(0.0, 100.0),
    }
}

/// The `rollout` percentage in an update manifest (`latest.json`), when it has one.
///
/// ```
/// let manifest = serde_json::json!({"version": "1.2.0", "rollout": 25});
/// assert_eq!(desktop_core::release::rollout_of(&manifest), Some(25.0));
/// assert_eq!(desktop_core::release::rollout_of(&serde_json::json!({})), None);
/// ```
pub fn rollout_of(manifest: &serde_json::Value) -> Option<f64> {
    manifest.get("rollout").and_then(serde_json::Value::as_f64)
}

/// The update manifest of one published release, used to go back to it:
/// `https://github.com/<repo>/releases/download/v<version>/latest.json`.
pub fn release_manifest_url(repo: &str, version: &str) -> String {
    format!("https://github.com/{repo}/releases/download/v{}/latest.json", version.trim_start_matches('v'))
}

/// What a rollback goes back to, written before an update installs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackRecord {
    /// The version that was running before the update.
    pub previous: String,
    /// A copy of its bundle (`.app` or `.AppImage`), where the platform allows keeping one.
    pub backup: Option<PathBuf>,
}

/// Save `record` in `data_dir`.
pub fn save_record(data_dir: &Path, record: &RollbackRecord) -> io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
    std::fs::write(data_dir.join(ROLLBACK_FILE), text)
}

/// The saved record, if an update was installed from this app before.
pub fn load_record(data_dir: &Path) -> Option<RollbackRecord> {
    serde_json::from_str(&std::fs::read_to_string(data_dir.join(ROLLBACK_FILE)).ok()?).ok()
}

/// Forget the record and the kept bundle (after a rollback, or to free the space).
pub fn clear(data_dir: &Path) -> io::Result<()> {
    let _ = std::fs::remove_file(data_dir.join(ROLLBACK_FILE));
    match std::fs::remove_dir_all(data_dir.join(ROLLBACK_DIR)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The bundle worth keeping for a rollback on `os` (`std::env::consts::OS`): on macOS the
/// `.app` folder around `exe`, on Linux the running AppImage (`$APPIMAGE`). Elsewhere
/// (Windows, a `.deb` install, a development build) there is none.
///
/// ```
/// use desktop_core::release::bundle_to_keep;
/// use std::path::Path;
///
/// let exe = Path::new("/Applications/Notes.app/Contents/MacOS/notes");
/// assert_eq!(bundle_to_keep(exe, None, "macos").as_deref(), Some(Path::new("/Applications/Notes.app")));
/// let image = Path::new("/home/me/Notes.AppImage");
/// assert_eq!(bundle_to_keep(Path::new("/tmp/.mount/notes"), Some(image), "linux").as_deref(), Some(image));
/// assert_eq!(bundle_to_keep(Path::new("C:/Notes/notes.exe"), None, "windows"), None);
/// ```
pub fn bundle_to_keep(exe: &Path, appimage: Option<&Path>, os: &str) -> Option<PathBuf> {
    match os {
        "macos" => exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf),
        "linux" => appimage.filter(|p| p.is_absolute()).map(Path::to_path_buf),
        _ => None,
    }
}

/// Copy `bundle` (a file or a folder) into `data_dir/rollback/`, replacing an older copy,
/// and return where the copy is.
pub fn keep_bundle(bundle: &Path, data_dir: &Path) -> io::Result<PathBuf> {
    let name = bundle.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the bundle has no name"))?;
    let dir = data_dir.join(ROLLBACK_DIR);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(name);
    copy_tree(bundle, &dest)?;
    Ok(dest)
}

/// Put `backup` in place of `target`. The current `target` is moved aside first and put
/// back if the copy fails, so a failed rollback leaves the app as it was.
pub fn restore_bundle(backup: &Path, target: &Path) -> io::Result<()> {
    if !backup.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is gone", backup.display())));
    }
    let mut aside = target.as_os_str().to_owned();
    aside.push(".rollback-old");
    let aside = PathBuf::from(aside);
    remove_any(&aside)?;
    std::fs::rename(target, &aside)?;
    if let Err(e) = copy_tree(backup, target) {
        let _ = remove_any(target);
        std::fs::rename(&aside, target)?;
        return Err(e);
    }
    remove_any(&aside)
}

fn remove_any(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Copy a file or a folder tree, keeping symbolic links as links (app bundles use them)
/// and file permissions.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(std::fs::read_link(from)?, to);
        #[cfg(not(unix))]
        return std::fs::copy(from, to).map(|_| ());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    std::fs::copy(from, to).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_fill_the_token() {
        let eps = vec![
            "https://h/channel-{channel}/latest.json".to_string(),
            "https://h/channel-%7Bchannel%7D/latest.json".to_string(),
            "https://h/latest.json".to_string(),
        ];
        assert_eq!(channel_endpoints(&eps, Channel::Stable), ["https://h/channel-stable/latest.json", "https://h/channel-stable/latest.json", "https://h/latest.json"]);
        assert_eq!(serde_json::to_value(Channel::Beta).unwrap(), "beta");
        let cfg: UpdatesConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.channel, Channel::Stable);
    }

    #[test]
    fn install_id_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let a = install_id(dir.path()).unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(install_id(dir.path()).unwrap(), a);
        std::fs::write(dir.path().join(INSTALL_ID_FILE), "not hex").unwrap();
        assert_ne!(install_id(dir.path()).unwrap(), a, "a broken id is replaced");
    }

    #[test]
    fn buckets_spread_and_rollouts_admit_a_share() {
        let mut counts = [0u32; 100];
        for i in 0..10_000 {
            counts[usize::from(bucket(&format!("{i:032x}")))] += 1;
        }
        assert!(counts.iter().all(|&c| c > 50 && c < 150), "{counts:?}");
        let admitted_at = |p: f64| (0..100u8).filter(|&b| admitted(b, Some(p))).count();
        assert_eq!(admitted_at(25.0), 25);
        assert_eq!(admitted_at(0.0), 0);
        assert_eq!(admitted_at(150.0), 100);
        assert_eq!(admitted_at(-5.0), 0);
        // Raising the rollout keeps everyone admitted before.
        assert!((0..100u8).all(|b| !admitted(b, Some(10.0)) || admitted(b, Some(50.0))));
    }

    #[test]
    fn manifest_urls() {
        assert_eq!(release_manifest_url("o/r", "v1.2.0"), "https://github.com/o/r/releases/download/v1.2.0/latest.json");
        assert_eq!(release_manifest_url("o/r", "1.2.0"), "https://github.com/o/r/releases/download/v1.2.0/latest.json");
    }

    #[test]
    fn keeps_and_restores_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Notes.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/notes"), "v1").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("MacOS/notes", app.join("Contents/current")).unwrap();
        let data = dir.path().join("data");
        let kept = keep_bundle(&app, &data).unwrap();
        save_record(&data, &RollbackRecord { previous: "1.0.0".into(), backup: Some(kept.clone()) }).unwrap();

        // The update replaces the bundle; the rollback puts the old one back.
        std::fs::write(app.join("Contents/MacOS/notes"), "v2").unwrap();
        let record = load_record(&data).unwrap();
        assert_eq!(record.previous, "1.0.0");
        restore_bundle(record.backup.as_deref().unwrap(), &app).unwrap();
        assert_eq!(std::fs::read_to_string(app.join("Contents/MacOS/notes")).unwrap(), "v1");
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(app.join("Contents/current")).unwrap().file_type().is_symlink());
        assert!(!dir.path().join("Notes.app.rollback-old").exists());

        clear(&data).unwrap();
        assert!(load_record(&data).is_none());
        assert!(restore_bundle(&kept, &app).is_err(), "nothing left to restore");
        assert!(app.exists(), "a failed rollback leaves the app in place");
    }
}
