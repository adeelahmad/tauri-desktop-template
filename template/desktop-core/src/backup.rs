//! Backups: the whole index as one portable file, made on demand or on a schedule, kept in
//! the app data folder or a folder you choose (a synced cloud folder makes it off-site).
//!
//! A backup holds the chunks with their provenance, not the vectors: restoring into either
//! store (files or Postgres) embeds them again with the current model, so a backup survives
//! a model change, a store change or a move to another computer. The file is JSON lines:
//! line 1 is the manifest, every other line one chunk, and the manifest carries the SHA-256
//! of the chunk lines so a damaged or truncated file is refused before anything is replaced.
//!
//! With `encrypt` on, everything after the manifest is sealed with ChaCha20-Poly1305 (ring,
//! already in every build through rustls) under a random 256-bit key kept owner-only in
//! `<data>/backup.key`; the manifest is authenticated with it, and only counts, dates and
//! the key's id stay readable. Backups in a synced folder are then useless without the
//! key. Settings shows the key as a recovery key: keep it somewhere safe, because restoring
//! on another computer (or after losing the data folder) needs it.
//!
//! Main items: [`write()`], [`read()`], [`list`], [`prune`], [`due`], [`BackupKey`] and
//! [`BackupConfig`]. `Engine::backup`, `Engine::restore` and `Engine::backup_if_due` wrap them
//! for the app.
//!
//! # Example
//!
//! ```
//! use desktop_core::backup::{self, BackupKey};
//! use desktop_core::chunks_from_text;
//!
//! # let dir = std::env::temp_dir().join(format!("desktop-core-backup-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let key = BackupKey::from_text(&"ab".repeat(32)).expect("64 hex digits");
//! assert_eq!(BackupKey::from_text(&key.to_text()), Some(key.clone()));
//!
//! let chunks = chunks_from_text("notes.md", "Back me up.");
//! let info = backup::write(&dir, &chunks, "keyword-only", Some(&key))?;
//! assert!(info.encrypted);
//! let (manifest, restored) = backup::read(&info.path, &[key])?;
//! assert_eq!((manifest.chunks, restored[0].text.as_str()), (1, "Back me up."));
//! # let _ = std::fs::remove_dir_all(&dir);
//! # Ok::<(), std::io::Error>(())
//! ```

use crate::chunk::Chunk;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const FORMAT: &str = "desktop-core-backup";
const VERSION: u32 = 1;

/// Backup settings (`backup` in settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct BackupConfig {
    /// Back up on this schedule (0 = only when asked).
    pub interval_hours: u32,
    /// Backups to keep in the folder (older ones are deleted; 0 keeps all).
    pub keep: u32,
    /// Where backups go (default: `<data>/backups`).
    pub folder: Option<PathBuf>,
    /// Encrypt new backups with this computer's backup key.
    pub encrypt: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self { interval_hours: 0, keep: 5, folder: None, encrypt: false }
    }
}

/// A 256-bit backup key.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupKey([u8; 32]);

impl std::fmt::Debug for BackupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BackupKey({})", self.id())
    }
}

impl BackupKey {
    /// The key in `<data>/backup.key`, made (owner-only) the first time.
    pub fn load_or_create(data_dir: &Path) -> std::io::Result<Self> {
        let file = data_dir.join("backup.key");
        if let Ok(text) = std::fs::read_to_string(&file) {
            return Self::from_text(&text).ok_or_else(|| bad(format!("{} is damaged; restore it from your recovery key", file.display())));
        }
        let mut key = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key).map_err(|_| bad("no secure random numbers"))?;
        let key = Self(key);
        key.save(data_dir)?;
        Ok(key)
    }

    /// Keep this key as the computer's backup key (owner-only `<data>/backup.key`).
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        crate::jobs::write_private(&data_dir.join("backup.key"), self.to_text().as_bytes())
    }

    /// Read a recovery key: 64 hex digits; spaces and dashes are ignored.
    pub fn from_text(text: &str) -> Option<Self> {
        let hex: Vec<u8> = text.bytes().filter(|b| !matches!(b, b' ' | b'-' | b'\n' | b'\r' | b'\t')).collect();
        if hex.len() != 64 {
            return None;
        }
        let mut key = [0u8; 32];
        for (i, pair) in hex.chunks(2).enumerate() {
            key[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Self(key))
    }

    /// The recovery key: 8 groups of 8 hex digits.
    pub fn to_text(&self) -> String {
        let hex: String = self.0.iter().map(|b| format!("{b:02x}")).collect();
        hex.as_bytes().chunks(8).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
    }

    /// A short public id (so a backup can say which key it needs, without revealing it).
    pub fn id(&self) -> String {
        format!("{:x}", Sha256::digest([b"backup-key-id:".as_slice(), &self.0].concat()))[..12].to_string()
    }

    fn sealing(&self) -> ring::aead::LessSafeKey {
        ring::aead::LessSafeKey::new(ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, &self.0).expect("a 32-byte key"))
    }
}


impl BackupConfig {
    /// Interval and count clamped; an empty folder means the default.
    pub fn validated(mut self) -> Self {
        self.interval_hours = self.interval_hours.min(24 * 90);
        self.keep = self.keep.min(1000);
        self.folder = self.folder.filter(|f| !f.as_os_str().is_empty());
        self
    }
}

/// Line 1 of a backup file: what it holds and how to check it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Always `desktop-core-backup`.
    pub format: String,
    /// The file format version (newer files are refused).
    pub version: u32,
    /// The app version that wrote it.
    pub app_version: String,
    /// RFC 3339, UTC.
    pub created: String,
    /// Chunks in the file.
    pub chunks: usize,
    /// Distinct sources (files or names) in the file.
    pub sources: usize,
    /// The embedding model of the store it came from (restores re-embed with the current one).
    pub model: String,
    /// SHA-256 of every line after the manifest (before encryption).
    pub sha256: String,
    /// The chunks are encrypted with the key whose id is `key_id`.
    #[serde(default)]
    pub encrypted: bool,
    /// The id of the key that encrypted it (`BackupKey::id`), empty when not encrypted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key_id: String,
}

/// One backup file, for lists and results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    /// The backup file.
    pub path: PathBuf,
    /// When it was made (RFC 3339).
    pub created: String,
    /// Chunks in it.
    pub chunks: usize,
    /// Distinct sources in it.
    pub sources: usize,
    /// File size.
    pub bytes: u64,
    /// The embedding model of the store it came from.
    pub model: String,
    /// Whether restoring needs the backup key.
    pub encrypted: bool,
}

fn bad(msg: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

/// Write `chunks` to a new backup in `dir` (encrypted when a key is given) and return it.
pub fn write(dir: &Path, chunks: &[Chunk], model: &str, key: Option<&BackupKey>) -> std::io::Result<BackupInfo> {
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
        encrypted: key.is_some(),
        key_id: key.map(BackupKey::id).unwrap_or_default(),
    };
    let mut head = serde_json::to_vec(&manifest).map_err(bad)?;
    head.push(b'\n');
    if let Some(key) = key {
        // A fresh random nonce per file; the manifest line is the associated data.
        let mut nonce = [0u8; 12];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce).map_err(|_| bad("no secure random numbers"))?;
        key.sealing()
            .seal_in_place_append_tag(ring::aead::Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(&head), &mut body)
            .map_err(|_| bad("encryption failed"))?;
        body = [nonce.as_slice(), &body].concat();
    }
    // backup-2026-11-23T05-40-12-345Z.jsonl: sorts by time and is a valid name everywhere.
    let stamp = created.replace([':', '.'], "-");
    let ext = if key.is_some() { "enc" } else { "jsonl" };
    let mut path = dir.join(format!("backup-{stamp}.{ext}"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("backup-{stamp}-{n}.{ext}"));
        n += 1;
    }
    let part = path.with_extension(format!("{ext}.part"));
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&part)?);
        f.write_all(&head)?;
        f.write_all(&body)?;
        f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(&part, &path)?;
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupInfo { path, created, chunks: manifest.chunks, sources, bytes, model: manifest.model, encrypted: manifest.encrypted })
}

fn manifest_of(path: &Path) -> std::io::Result<Manifest> {
    // Only the first line, and only so much of it: a file without newlines is not read whole.
    let mut line = Vec::new();
    let max = crate::untrusted::limits::BACKUP_HEADER_BYTES as u64;
    std::io::BufReader::new(std::fs::File::open(path)?).take(max).read_until(b'\n', &mut line)?;
    let m: Manifest = serde_json::from_slice(&line).map_err(|e| bad(format!("not a backup ({e})")))?;
    if m.format != FORMAT {
        return Err(bad("not a backup from this app"));
    }
    if m.version > VERSION {
        return Err(bad(format!("this backup was written by a newer version of the app (format {}); update the app", m.version)));
    }
    Ok(m)
}

/// Read and verify a backup: the format, the key (for an encrypted one), the checksum and
/// the chunk count. `keys` are tried in order; the one whose id matches is used.
pub fn read(path: &Path, keys: &[BackupKey]) -> std::io::Result<(Manifest, Vec<Chunk>)> {
    let m = manifest_of(path)?;
    let mut text = std::fs::read(path)?;
    let start = text.iter().position(|&b| b == b'\n').map_or(text.len(), |i| i + 1);
    let (head, rest) = text.split_at_mut(start);
    let body: &[u8] = if m.encrypted {
        let key = keys.iter().find(|k| k.id() == m.key_id).ok_or_else(|| {
            bad(format!("this backup is encrypted with key {}; enter the recovery key from the computer that made it", m.key_id))
        })?;
        if rest.len() < 12 + ring::aead::CHACHA20_POLY1305.tag_len() {
            return Err(bad("the backup is damaged (too short)"));
        }
        let (nonce, sealed) = rest.split_at_mut(12);
        let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| bad("the backup is damaged"))?;
        key.sealing()
            .open_in_place(nonce, ring::aead::Aad::from(&*head), sealed)
            .map_err(|_| bad("the backup is damaged or was changed (it does not decrypt)"))?
    } else {
        rest
    };
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
            p.extension().is_some_and(|x| x == "jsonl" || x == "enc") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("backup-"))
        })
        .filter_map(|path| {
            let m = manifest_of(&path).ok()?;
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Some(BackupInfo { path, created: m.created, chunks: m.chunks, sources: m.sources, bytes, model: m.model, encrypted: m.encrypted })
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
        let first = write(&dir, &chunks, "bge", None).unwrap();
        assert_eq!((first.chunks, first.sources), (3, 2));
        let (m, back) = read(&first.path, &[]).unwrap();
        assert_eq!(back, chunks);
        assert_eq!(m.model, "bge");
        for _ in 0..3 {
            write(&dir, &chunks[..1], "bge", None).unwrap();
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
        assert!(read(&left[0].path, &[]).unwrap_err().to_string().contains("damaged"));
        std::fs::write(dir.join("backup-x.jsonl"), "{\"format\":\"other\"}\n").unwrap();
        assert_eq!(list(&dir).len(), 2);
        assert!(due(&std::env::temp_dir().join("no-backups-here"), 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypted_backups_need_the_right_key() {
        let dir = std::env::temp_dir().join(format!("backup-enc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = BackupKey::load_or_create(&dir).unwrap();
        assert_eq!(BackupKey::load_or_create(&dir).unwrap(), key, "made once, then reused");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("backup.key")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let text = key.to_text();
        assert_eq!(text.len(), 71);
        assert_eq!(BackupKey::from_text(&text.to_uppercase().replace('-', " ")), Some(key.clone()));
        assert!(BackupKey::from_text("abc").is_none());

        let chunks = vec![chunk("secret.md", "the launch code is 0000")];
        let info = write(&dir.join("out"), &chunks, "bge", Some(&key)).unwrap();
        assert!(info.encrypted && info.path.extension().unwrap() == "enc");
        let raw = std::fs::read(&info.path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("launch code"), "the text is not readable");
        assert_eq!(list(&dir.join("out")).len(), 1);
        assert_eq!(read(&info.path, std::slice::from_ref(&key)).unwrap().1, chunks);

        let other = BackupKey([7; 32]);
        assert!(read(&info.path, &[]).unwrap_err().to_string().contains(&key.id()));
        assert!(read(&info.path, std::slice::from_ref(&other)).is_err());
        assert_eq!(read(&info.path, &[other, key.clone()]).unwrap().1, chunks, "the matching key is picked");

        // A changed manifest (it is authenticated) or body is refused.
        let mut tampered = raw.clone();
        let pos = String::from_utf8_lossy(&raw).find("\"chunks\":1").unwrap() + 9;
        tampered[pos] = b'2';
        std::fs::write(&info.path, &tampered).unwrap();
        assert!(read(&info.path, std::slice::from_ref(&key)).is_err());
        let mut tampered = raw;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        std::fs::write(&info.path, &tampered).unwrap();
        assert!(read(&info.path, &[key]).unwrap_err().to_string().contains("decrypt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_huge_first_line_is_not_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.backup");
        std::fs::write(&file, vec![b'{'; 4 * crate::untrusted::limits::BACKUP_HEADER_BYTES]).unwrap();
        assert!(read(&file, &[]).unwrap_err().to_string().contains("not a backup"));
    }
}
