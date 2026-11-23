//! Auto-ingest from a drop folder and any other folders the user adds (feature `watch`).
//!
//! Changes are debounced (`WatchConfig::debounce_ms`), so a file is ingested once it
//! stops changing; new and changed files go through `Engine::ingest_paths` (with the
//! configured worker count), deleted ones are removed from the index. Folders are checked
//! first: no filesystem roots, system folders or the app's own data folder.

use crate::config::WatchConfig;
use crate::{Engine, Error, Result};
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// What the watcher did, for the UI (`watch-event`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WatchEvent {
    Started { folders: Vec<PathBuf> },
    Ingested { files: usize, chunks: usize, errors: Vec<String> },
    Removed { path: PathBuf, chunks: usize },
    Error { message: String },
}

/// `<Documents>/<app> Inbox`, or `<data_dir>/inbox` when there is no Documents folder.
pub fn default_drop_folder(app_name: &str, data_dir: &Path) -> PathBuf {
    let docs = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|h| PathBuf::from(h).join("Documents"));
    match docs.filter(|d| d.is_dir()) {
        Some(d) => d.join(format!("{app_name} Inbox")),
        None => data_dir.join("inbox"),
    }
}

/// The canonical folder, or why it may not be watched.
pub fn check_folder(path: &Path, data_dir: &Path) -> Result<PathBuf> {
    let p = std::fs::canonicalize(path).map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
    if !p.is_dir() {
        return Err(Error::Source(format!("{} is not a folder", p.display())));
    }
    if p.parent().is_none() {
        return Err(Error::Source(format!("{} is a whole drive; pick a folder on it", p.display())));
    }
    let system: &[&str] = if cfg!(windows) {
        &["\\Windows", "\\Program Files", "\\Program Files (x86)", "\\ProgramData"]
    } else {
        &["/System", "/Library", "/usr", "/bin", "/sbin", "/etc", "/private/etc", "/dev", "/proc", "/sys", "/boot"]
    };
    let s = p.to_string_lossy();
    let rest = if cfg!(windows) { s.get(s.find(':').map_or(0, |i| i + 1)..).unwrap_or(&s) } else { &s };
    if system.iter().any(|d| rest.eq_ignore_ascii_case(d) || rest.to_lowercase().starts_with(&format!("{}{}", d.to_lowercase(), std::path::MAIN_SEPARATOR))) {
        return Err(Error::Source(format!("{} is a system folder", p.display())));
    }
    if let Ok(data) = std::fs::canonicalize(data_dir) {
        if p.starts_with(&data) || data.starts_with(&p) {
            return Err(Error::Source(format!("{} holds the app's own data", p.display())));
        }
    }
    Ok(p)
}

/// The folders `cfg` asks for, created (the drop folder) and checked. Bad ones are
/// reported, not fatal.
pub fn folders(cfg: &WatchConfig, drop_default: &Path, data_dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut want: Vec<PathBuf> = Vec::new();
    if cfg.drop_folder {
        let drop = cfg.drop_folder_path.clone().unwrap_or_else(|| drop_default.to_path_buf());
        if let Err(e) = std::fs::create_dir_all(&drop) {
            return (Vec::new(), vec![format!("{}: {e}", drop.display())]);
        }
        want.push(drop);
    }
    want.extend(cfg.paths.iter().cloned());
    let mut ok = BTreeSet::new();
    let mut errors = Vec::new();
    for p in want {
        match check_folder(&p, data_dir) {
            Ok(p) => {
                ok.insert(p);
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    (ok.into_iter().collect(), errors)
}

/// A running watcher; dropping it stops watching.
pub struct Watcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    pub folders: Vec<PathBuf>,
}

/// Start watching per `cfg` (does nothing useful when `cfg.enabled` is false: callers
/// should not start it). `on_event` runs on the watcher's thread.
pub fn start(
    engine: Arc<Engine>,
    cfg: &WatchConfig,
    drop_default: &Path,
    data_dir: &Path,
    on_event: impl Fn(WatchEvent) + Send + 'static,
) -> Result<Watcher> {
    let (folders, errors) = folders(cfg, drop_default, data_dir);
    for e in errors {
        on_event(WatchEvent::Error { message: e });
    }
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(Duration::from_millis(cfg.debounce_ms), None, tx).map_err(|e| Error::Source(e.to_string()))?;
    let mode = if cfg.recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    for f in &folders {
        debouncer.watch(f, mode).map_err(|e| Error::Source(format!("{}: {e}", f.display())))?;
    }
    on_event(WatchEvent::Started { folders: folders.clone() });
    let existing: Vec<PathBuf> = if cfg.ingest_existing { folders.iter().flat_map(|f| engine.files_in(f, cfg.recursive)).collect() } else { Vec::new() };
    let remove_deleted = cfg.remove_deleted;
    let roots = folders.clone();
    std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || {
            let ingest = |paths: Vec<PathBuf>| {
                if paths.is_empty() {
                    return;
                }
                let r = engine.ingest_paths(&paths, &|_, _, _| {});
                if r.files > 0 || !r.errors.is_empty() {
                    on_event(WatchEvent::Ingested { files: r.files, chunks: r.chunks, errors: r.errors });
                }
            };
            ingest(existing);
            for result in rx {
                let events = match result {
                    Ok(events) => events,
                    Err(errors) => {
                        for e in errors {
                            on_event(WatchEvent::Error { message: e.to_string() });
                        }
                        continue;
                    }
                };
                let mut changed = BTreeSet::new();
                for ev in events {
                    for p in &ev.paths {
                        // Only files under a watched root (renames can report the old place).
                        if !roots.iter().any(|r| p.starts_with(r)) {
                            continue;
                        }
                        if p.is_file() {
                            if engine.skip_reason(p).is_none() {
                                changed.insert(p.clone());
                            }
                        } else if remove_deleted && !p.exists() {
                            match engine.remove_source(&p.to_string_lossy()) {
                                Ok(0) => {}
                                Ok(n) => on_event(WatchEvent::Removed { path: p.clone(), chunks: n }),
                                Err(e) => on_event(WatchEvent::Error { message: e.to_string() }),
                            }
                        }
                    }
                }
                ingest(changed.into_iter().collect());
            }
        })
        .map_err(Error::Io)?;
    Ok(Watcher { _debouncer: debouncer, folders })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_roots_system_and_data_folders() {
        let data = tempfile::tempdir().unwrap();
        let root = if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") };
        assert!(check_folder(&root, data.path()).is_err());
        if !cfg!(windows) {
            assert!(check_folder(Path::new("/etc"), data.path()).is_err());
        }
        assert!(check_folder(data.path(), data.path()).is_err());
        let other = tempfile::tempdir().unwrap();
        assert!(check_folder(other.path(), data.path()).is_ok());
    }
}
