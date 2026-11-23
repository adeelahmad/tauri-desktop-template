//! Backups: the whole index as one portable file, made on demand or on a schedule, kept in
//! the app data folder or a folder you choose (a synced cloud folder makes it off-site).
//!
//! A backup holds the chunks with their provenance, not the vectors: restoring into either
//! store (files or Postgres) embeds them again with the current model, so a backup survives
//! a model change, a store change or a move to another computer. The file is JSON lines:
//! line 1 is the manifest, every other line one chunk, and the manifest carries the SHA-256
//! of the chunk lines so a damaged or truncated file is refused before anything is replaced.

use crate::chunk::Chunk;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const FORMAT: &str = "desktop-core-backup";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackupConfig {
    /// Back up on this schedule (0 = only when asked).
    pub interval_hours: u32,
    /// Backups to keep in the folder (older ones are deleted; 0 keeps all).
    pub keep: u32,
    /// Where backups go (default: `<data>/backups`).
    pub folder: Option<PathBuf>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self { interval_hours: 0, keep: 5, folder: None }
    }
}

impl BackupConfig {
    pub fn validated(mut self) -> Self {
        self.interval_hours = self.interval_hours.min(24 * 90);
        self.keep = self.keep.min(1000);
        self.folder = self.folder.filter(|f| !f.as_os_str().is_empty());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    /// The app version that wrote it.
    pub app_version: String,
    /// RFC 3339, UTC.
    pub created: String,
    pub chunks: usize,
    pub sources: usize,
    /// The embedding model of the store it came from (restores re-embed with the current one).
    pub model: String,
    /// SHA-256 of every line after the manifest.
    pub sha256: String,
}

/// One backup file, for lists and results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub path: PathBuf,
    pub created: String,
    pub chunks: usize,
    pub sources: usize,
    pub bytes: u64,
    pub model: String,
}

fn bad(msg: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

/// Write `chunks` to a new backup in `dir` and return it.
pub fn write(dir: &Path, chunks: &[Chunk], model: &str) -> std::io::Result<BackupInfo> {
    std::fs::create_dir_all(dir)?;
    let mut body = Vec::new();
    for c in chunks {
        serde_json::to_writer(&mut body, c).map_err(bad)?;
        body.push(b'\n');
    }
    let sources = chunks.iter().map(|c| c.source.as_str()).collect::<std::collections::HashSet<_>>().len();
    let created = crate::logging::rfc3339(SystemTime::now());
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created: created.clone(),
        chunks: chunks.len(),
        sources,
        model: model.into(),
        sha256: format!("{:x}", Sha256::digest(&body)),
    };
    // backup-2026-11-23T05-40-12-345Z.jsonl: sorts by time and is a valid name everywhere.
    let stamp = created.replace([':', '.'], "-");
    let mut path = dir.join(format!("backup-{stamp}.jsonl"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("backup-{stamp}-{n}.jsonl"));
        n += 1;
    }
    let part = path.with_extension("jsonl.part");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&part)?);
        serde_json::to_writer(&mut f, &manifest).map_err(bad)?;
        f.write_all(b"\n")?;
        f.write_all(&body)?;
        f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(&part, &path)?;
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupInfo { path, created, chunks: manifest.chunks, sources, bytes, model: manifest.model })
}

fn manifest_of(path: &Path) -> std::io::Result<Manifest> {
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(path)?).read_line(&mut line)?;
    let m: Manifest = serde_json::from_str(&line).map_err(|e| bad(format!("not a backup ({e})")))?;
    if m.format != FORMAT {
        return Err(bad("not a backup from this app"));
    }
    if m.version > VERSION {
        return Err(bad(format!("this backup was written by a newer version of the app (format {}); update the app", m.version)));
    }
    Ok(m)
}

/// Read and verify a backup: the format, the checksum and the chunk count.
pub fn read(path: &Path) -> std::io::Result<(Manifest, Vec<Chunk>)> {
    let m = manifest_of(path)?;
    let text = std::fs::read(path)?;
    let start = text.iter().position(|&b| b == b'\n').map_or(text.len(), |i| i + 1);
    let body = &text[start..];
    if format!("{:x}", Sha256::digest(body)) != m.sha256 {
        return Err(bad("the backup is damaged (checksum mismatch)"));
    }
    let chunks = body
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(serde_json::from_slice::<Chunk>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(bad)?;
    if chunks.len() != m.chunks {
        return Err(bad(format!("the backup should hold {} chunks but holds {}", m.chunks, chunks.len())));
    }
    Ok((m, chunks))
}

/// Backups in `dir`, newest first. Files that are not backups are left out.
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "jsonl") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("backup-"))
        })
        .filter_map(|path| {
            let m = manifest_of(&path).ok()?;
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Some(BackupInfo { path, created: m.created, chunks: m.chunks, sources: m.sources, bytes, model: m.model })
        })
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.path.cmp(&a.path)));
    out
}

/// Delete all but the newest `keep` backups (0 keeps all). Returns how many went.
pub fn prune(dir: &Path, keep: u32) -> usize {
    if keep == 0 {
        return 0;
    }
    list(dir).into_iter().skip(keep as usize).filter(|b| std::fs::remove_file(&b.path).is_ok()).count()
}

/// Whether a scheduled backup is due: on, and the newest is older than the interval.
pub fn due(dir: &Path, interval_hours: u32) -> bool {
    if interval_hours == 0 {
        return false;
    }
    let newest = list(dir).into_iter().next().and_then(|b| std::fs::metadata(&b.path).and_then(|m| m.modified()).ok());
    newest.is_none_or(|t| t.elapsed().unwrap_or(Duration::ZERO) >= Duration::from_secs(u64::from(interval_hours) * 3600))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(source: &str, text: &str) -> Chunk {
        Chunk { source: source.into(), page: None, text: text.into(), prov: None }
    }

    #[test]
    fn round_trip_list_prune_and_damage() {
        let dir = std::env::temp_dir().join(format!("backup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let chunks = vec![chunk("a.md", "alpha\nline"), chunk("a.md", "beta"), chunk("b.md", "gamma")];
        let first = write(&dir, &chunks, "bge").unwrap();
        assert_eq!((first.chunks, first.sources), (3, 2));
        let (m, back) = read(&first.path).unwrap();
        assert_eq!(back, chunks);
        assert_eq!(m.model, "bge");
        for _ in 0..3 {
            write(&dir, &chunks[..1], "bge").unwrap();
        }
        assert_eq!(list(&dir).len(), 4);
        assert!(!due(&dir, 0) && !due(&dir, 1));
        assert_eq!(prune(&dir, 2), 2);
        let left = list(&dir);
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|b| b.chunks == 1), "the newest are kept");

        // A changed byte is caught before anything is restored.
        let mut text = std::fs::read(&left[0].path).unwrap();
        let last = text.len() - 3;
        text[last] ^= 1;
        std::fs::write(&left[0].path, &text).unwrap();
        assert!(read(&left[0].path).unwrap_err().to_string().contains("damaged"));
        std::fs::write(dir.join("backup-x.jsonl"), "{\"format\":\"other\"}\n").unwrap();
        assert_eq!(list(&dir).len(), 2);
        assert!(due(&std::env::temp_dir().join("no-backups-here"), 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
