//! A built-in key-value store (feature `kv`): embedded, persistent and pure Rust, in one
//! file in the data folder. No server and no Postgres.
//!
//! Built on [redb](https://docs.rs/redb) (ACID transactions, crash safe, one file, MIT OR
//! Apache-2.0, no dependencies beyond libc). On top of it this module adds what an app
//! needs from a "localStorage that is not a browser":
//!
//! - **namespaces** ([`Kv::ns`]), each its own table, so `settings`, `ui` and `secrets` never
//!   mix;
//! - bytes or typed JSON values ([`Namespace::get_json`], [`Namespace::set_json`]);
//! - a **TTL per key**: expired keys read as missing at once (lazy expiry) and are deleted
//!   by [`Kv::sweep`], which [`Kv::spawn_sweeper`] runs on a schedule. Time comes from a
//!   [`Clock`], so tests use a [`ManualClock`] instead of sleeping;
//! - **atomic operations**: [`Namespace::compare_and_swap`], [`Namespace::incr`] and
//!   [`Namespace::transaction`] for several keys at once (all or nothing);
//! - **size caps**: keys up to [`limits::KV_KEY_BYTES`], values up to
//!   [`KvOptions::max_value_bytes`] (at most [`limits::KV_VALUE_BYTES`]), and the file up to
//!   [`KvOptions::max_file_bytes`];
//! - optional **encryption at rest** per namespace ([`Kv::encrypt_namespace`]):
//!   ChaCha20-Poly1305 through ring (already in every build), with the key in an owner-only
//!   key file ([`KvKey::load_or_create`]){% if oauth %} or the OS keychain ([`KvKey::from_keychain`], with feature `oauth`){% endif %};
//! - [`Kv::export`] / [`Kv::import`] for backups (`Engine::backup` adds the store next to each
//!   backup when `kv.backup` is on).
//!
//! The window, the HTTP API and MCP tools reach only the namespaces listed in
//! [`KvConfig::ui_namespaces`] (default `ui`); Rust code can use any namespace.
//!
//! # Example
//!
//! ```
//! use desktop_core::kv::{Kv, KvOptions, ManualClock};
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! # let dir = tempfile::tempdir().unwrap();
//! let clock = Arc::new(ManualClock::new(0));
//! let kv = Kv::open_with(dir.path().join("kv.redb"), KvOptions { clock: clock.clone(), ..KvOptions::default() })?;
//! let settings = kv.ns("settings")?;
//!
//! settings.set_json("theme", &"dark")?;
//! assert_eq!(settings.get_json::<String>("theme")?.as_deref(), Some("dark"));
//!
//! // A key with a TTL reads as missing once it has expired.
//! settings.set_ttl("banner", b"hello", Duration::from_secs(60))?;
//! clock.advance(Duration::from_secs(61));
//! assert_eq!(settings.get("banner")?, None);
//!
//! // Atomic counters and compare-and-swap.
//! assert_eq!(settings.incr("launches", 1)?, 1);
//! assert!(settings.compare_and_swap("theme", Some(br#""dark""#), Some(br#""light""#))?);
//!
//! // Several keys at once: all or nothing.
//! settings.transaction(|tx| {
//!     tx.set_json("a", &1)?;
//!     tx.set_json("b", &2)
//! })?;
//! assert_eq!(settings.keys("")?, ["a", "b", "launches", "theme"]);
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::untrusted::limits;
use crate::{Error, Result};
use redb::{ReadableTable, ReadableTableMetadata, TableDefinition, TableHandle};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Every namespace is a redb table named `kv:<namespace>`.
const TABLE_PREFIX: &str = "kv:";
/// A stored value: `[kind][expires_at: u64 LE]` then the value (plain) or nonce + sealed value.
const PLAIN: u8 = 1;
const SEALED: u8 = 2;
const HEADER: usize = 9;
const NONCE: usize = 12;
/// The first bytes of an [`Kv::export`] dump.
const DUMP_MAGIC: &[u8] = b"desktop-core-kv 1\n";

type Def<'a> = TableDefinition<'a, &'static str, &'static [u8]>;
/// A key, its value and its expiry.
type Row = (String, Vec<u8>, Option<u64>);
type WriteTable<'t> = redb::Table<'t, &'static str, &'static [u8]>;

fn db(e: impl Into<redb::Error>) -> Error {
    match e.into() {
        redb::Error::DatabaseAlreadyOpen => {
            Error::Store("the key-value store is open in another process (with feature ipc-worker the app shares one)".into())
        }
        e => Error::Store(format!("key-value store: {e}")),
    }
}

// ---------------------------------------------------------------------------------------
// Time

/// Where the store gets the time for TTLs: [`SystemClock`], or [`ManualClock`] in tests.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

/// The computer's clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// A clock that only moves when told to, for tests of TTLs without sleeping.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    /// A clock at `ms` milliseconds since the epoch.
    pub fn new(ms: u64) -> Self {
        Self(AtomicU64::new(ms))
    }

    /// Move the clock forward.
    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(u64::try_from(by.as_millis()).unwrap_or(u64::MAX), Ordering::SeqCst);
    }

    /// Set the clock.
    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------------------
// Keys for encryption at rest

/// A 256-bit key that encrypts a namespace (ChaCha20-Poly1305).
#[derive(Clone, PartialEq, Eq)]
pub struct KvKey([u8; 32]);

impl std::fmt::Debug for KvKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KvKey({})", self.id())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl KvKey {
    /// A new random key.
    pub fn generate() -> Result<Self> {
        let mut key = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key).map_err(|_| Error::Store("no secure random numbers".into()))?;
        Ok(Self(key))
    }

    /// A key from its 32 bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// A key from 64 hex digits (spaces and dashes are ignored).
    pub fn from_text(text: &str) -> Option<Self> {
        let digits: Vec<u8> = text.bytes().filter(|b| !matches!(b, b' ' | b'-' | b'\n' | b'\r' | b'\t')).collect();
        if digits.len() != 64 {
            return None;
        }
        let mut key = [0u8; 32];
        for (i, pair) in digits.chunks(2).enumerate() {
            key[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Self(key))
    }

    /// The key as 64 hex digits.
    pub fn to_text(&self) -> String {
        hex(&self.0)
    }

    /// The key kept in `file` (owner-only on Unix), made the first time.
    pub fn load_or_create(file: &Path) -> Result<Self> {
        if let Ok(text) = std::fs::read_to_string(file) {
            return Self::from_text(&text).ok_or_else(|| Error::Store(format!("{} is damaged", file.display())));
        }
        let key = Self::generate()?;
        crate::jobs::write_private(file, key.to_text().as_bytes())?;
        Ok(key)
    }
{% if oauth %}
    /// The key kept in the OS keychain under `service` / `account`, made the first time
    /// (feature `oauth`, which brings the keyring crate).
    #[cfg(feature = "oauth")]
    pub fn from_keychain(service: &str, account: &str) -> Result<Self> {
        let failed = |e: keyring::Error| Error::Store(format!("keychain: {e}"));
        let entry = keyring::Entry::new(service, account).map_err(failed)?;
        match entry.get_password() {
            Ok(text) => Self::from_text(&text).ok_or_else(|| Error::Store("the key in the keychain is damaged".into())),
            Err(keyring::Error::NoEntry) => {
                let key = Self::generate()?;
                entry.set_password(&key.to_text()).map_err(failed)?;
                Ok(key)
            }
            Err(e) => Err(failed(e)),
        }
    }
{% endif %}
    /// A short public id of the key (never the key itself).
    pub fn id(&self) -> String {
        use sha2::Digest;
        hex(&sha2::Sha256::digest([b"kv-key-id:".as_slice(), &self.0].concat()))[..12].to_string()
    }

    fn aead(&self) -> ring::aead::LessSafeKey {
        ring::aead::LessSafeKey::new(ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, &self.0).expect("a 32-byte key"))
    }
}

// ---------------------------------------------------------------------------------------
// Settings

/// Where the encryption key of encrypted namespaces is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// `<data>/kv.key`, owner-only (like the backup key).
    #[default]
    File,
    /// The OS keychain (needs feature `oauth`, which brings the keyring crate).
    Keychain,
}

/// `kv` in the config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct KvConfig {
    /// Namespaces the window, the HTTP API and MCP tools may read and write (default `ui`).
    /// Rust code can use any namespace; keep secrets in one that is not listed here.
    pub ui_namespaces: Vec<String>,
    /// Namespaces encrypted at rest (none by default).
    pub encrypted_namespaces: Vec<String>,
    /// Where the key of encrypted namespaces is kept.
    pub key_source: KeySource,
    /// The largest the store's file may grow, in MB; writes fail beyond it (16 to 65536).
    pub max_size_mb: u64,
    /// The largest value, in KB (1 to 16384).
    pub max_value_kb: u64,
    /// Expired keys are deleted this often, in minutes (1 to 1440).
    pub sweep_minutes: u64,
    /// Put a copy of the store next to every backup (sealed with the backup key when backups
    /// are encrypted).
    pub backup: bool,
}

impl Default for KvConfig {
    fn default() -> Self {
        Self {
            ui_namespaces: vec!["ui".into()],
            encrypted_namespaces: Vec::new(),
            key_source: KeySource::File,
            max_size_mb: 1024,
            max_value_kb: 1024,
            sweep_minutes: 10,
            backup: true,
        }
    }
}

impl KvConfig {
    /// Numbers clamped into range; invalid namespace names dropped.
    pub fn validated(mut self) -> Self {
        self.max_size_mb = self.max_size_mb.clamp(16, 65_536);
        self.max_value_kb = self.max_value_kb.clamp(1, (limits::KV_VALUE_BYTES / 1024) as u64);
        self.sweep_minutes = self.sweep_minutes.clamp(1, 1440);
        let clean = |list: Vec<String>| list.into_iter().map(|n| n.trim().to_string()).filter(|n| valid_namespace(n)).collect::<BTreeSet<_>>().into_iter().collect();
        self.ui_namespaces = clean(self.ui_namespaces);
        self.encrypted_namespaces = clean(self.encrypted_namespaces);
        self
    }

    /// Whether the window, the HTTP API and MCP tools may use namespace `ns`.
    pub fn allows(&self, ns: &str) -> bool {
        self.ui_namespaces.iter().any(|n| n == ns)
    }

    /// An error unless [`KvConfig::allows`] `ns`. The message starts with "Not allowed", which
    /// the HTTP API turns into `403`.
    pub fn check_exposed(&self, ns: &str) -> Result<()> {
        if self.allows(ns) {
            return Ok(());
        }
        Err(Error::Source(format!("Not allowed: namespace {ns:?} is not in kv.uiNamespaces")))
    }

    /// The store's options for these settings.
    pub fn options(&self) -> KvOptions {
        KvOptions {
            max_value_bytes: usize::try_from(self.max_value_kb * 1024).unwrap_or(limits::KV_VALUE_BYTES),
            max_file_bytes: self.max_size_mb * 1024 * 1024,
            ..KvOptions::default()
        }
    }
}

/// Whether `name` can be a namespace: 1 to 64 letters, digits, `_`, `-` or `.`.
pub fn valid_namespace(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn check_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(Error::Source("a key-value key must not be empty".into()));
    }
    crate::untrusted::check_size("a key-value key", key.len(), limits::KV_KEY_BYTES)
}

/// How a store is opened: size caps and the clock.
#[derive(Clone)]
pub struct KvOptions {
    /// The largest value (at most [`limits::KV_VALUE_BYTES`]).
    pub max_value_bytes: usize,
    /// Writes fail once the file is this large (0: no cap).
    pub max_file_bytes: u64,
    /// Where TTLs get the time.
    pub clock: Arc<dyn Clock>,
}

impl Default for KvOptions {
    fn default() -> Self {
        Self { max_value_bytes: limits::MB, max_file_bytes: 0, clock: Arc::new(SystemClock) }
    }
}

// ---------------------------------------------------------------------------------------
// The store

struct Inner {
    db: redb::Database,
    path: PathBuf,
    opts: KvOptions,
    /// Namespace -> the key that seals its values.
    keys: RwLock<HashMap<String, Arc<ring::aead::LessSafeKey>>>,
}

/// An open store. Cheap to clone: clones share one database.
#[derive(Clone)]
pub struct Kv(Arc<Inner>);

/// A namespace and its key count, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NamespaceInfo {
    /// The namespace.
    pub name: String,
    /// Keys in it (expired ones not yet swept included).
    pub keys: u64,
    /// Values are encrypted at rest.
    pub encrypted: bool,
}

impl Kv {
    /// Open (or create) the store in `path` with the default options.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, KvOptions::default())
    }

    /// Open (or create) the store in `path`. A file left by a crash is repaired on open.
    pub fn open_with(path: impl AsRef<Path>, mut opts: KvOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        opts.max_value_bytes = opts.max_value_bytes.min(limits::KV_VALUE_BYTES);
        let db = redb::Database::create(&path).map_err(db)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self(Arc::new(Inner { db, path, opts, keys: RwLock::new(HashMap::new()) })))
    }

    /// The store's file.
    pub fn path(&self) -> &Path {
        &self.0.path
    }

    /// The file's size in bytes.
    pub fn size(&self) -> u64 {
        std::fs::metadata(&self.0.path).map_or(0, |m| m.len())
    }

    /// Namespace `name` (it is created on the first write).
    pub fn ns(&self, name: &str) -> Result<Namespace> {
        if !valid_namespace(name) {
            return Err(Error::Source(format!("{name:?} is not a namespace name (1 to 64 letters, digits, _ - .)")));
        }
        Ok(Namespace { kv: self.clone(), name: name.into(), table: format!("{TABLE_PREFIX}{name}") })
    }

    /// Encrypt values written to namespace `ns` from now on with `key`, and decrypt with it.
    /// Values written before stay readable; reading a value sealed with another key fails.
    pub fn encrypt_namespace(&self, ns: &str, key: &KvKey) {
        if let Ok(mut keys) = self.0.keys.write() {
            keys.insert(ns.to_string(), Arc::new(key.aead()));
        }
    }

    fn sealer(&self, ns: &str) -> Option<Arc<ring::aead::LessSafeKey>> {
        self.0.keys.read().ok()?.get(ns).cloned()
    }

    fn now(&self) -> u64 {
        self.0.opts.clock.now_ms()
    }

    /// Every namespace with keys, with its key count.
    pub fn namespaces(&self) -> Result<Vec<NamespaceInfo>> {
        let txn = self.0.db.begin_read().map_err(db)?;
        let names: Vec<String> = txn.list_tables().map_err(db)?.map(|t| t.name().to_string()).collect();
        let mut out = Vec::new();
        for table in names {
            let Some(name) = table.strip_prefix(TABLE_PREFIX) else { continue };
            let keys = txn.open_table(Def::new(&table)).map_err(db)?.len().map_err(db)?;
            out.push(NamespaceInfo { encrypted: self.sealer(name).is_some(), name: name.to_string(), keys });
        }
        Ok(out)
    }

    fn tables(txn: &redb::WriteTransaction) -> Result<Vec<String>> {
        Ok(txn.list_tables().map_err(db)?.map(|t| t.name().to_string()).filter(|n| n.starts_with(TABLE_PREFIX)).collect())
    }

    /// Delete every expired key now. Returns how many went.
    pub fn sweep(&self) -> Result<usize> {
        let now = self.now();
        let txn = self.0.db.begin_write().map_err(db)?;
        let mut removed = 0;
        for name in Self::tables(&txn)? {
            let mut table = txn.open_table(Def::new(&name)).map_err(db)?;
            let mut expired = Vec::new();
            for row in table.iter().map_err(db)? {
                let (k, v) = row.map_err(db)?;
                if expires_at(v.value()).is_some_and(|at| at <= now) {
                    expired.push(k.value().to_string());
                }
            }
            for k in &expired {
                table.remove(k.as_str()).map_err(db)?;
            }
            removed += expired.len();
        }
        txn.commit().map_err(db)?;
        if removed > 0 {
            log::debug!("kv: swept {removed} expired keys");
        }
        Ok(removed)
    }

    /// Run [`Kv::sweep`] every `every` on a background thread, until the store is dropped.
    pub fn spawn_sweeper(&self, every: Duration) -> std::io::Result<std::thread::JoinHandle<()>> {
        let weak = Arc::downgrade(&self.0);
        std::thread::Builder::new().name("kv-sweep".into()).spawn(move || loop {
            std::thread::sleep(every);
            let Some(inner) = weak.upgrade() else { break };
            if let Err(e) = Kv(inner).sweep() {
                log::warn!("kv sweep: {e}");
            }
        })
    }

    /// Every live key of every namespace, as stored (encrypted values stay encrypted), for a
    /// backup. [`Kv::import`] reads it back.
    pub fn export(&self) -> Result<Vec<u8>> {
        let now = self.now();
        let txn = self.0.db.begin_read().map_err(db)?;
        let names: Vec<String> = txn.list_tables().map_err(db)?.map(|t| t.name().to_string()).filter(|n| n.starts_with(TABLE_PREFIX)).collect();
        let mut out = DUMP_MAGIC.to_vec();
        for name in names {
            let table = txn.open_table(Def::new(&name)).map_err(db)?;
            for row in table.iter().map_err(db)? {
                let (k, v) = row.map_err(db)?;
                if expires_at(v.value()).is_some_and(|at| at <= now) {
                    continue;
                }
                for part in [&name.as_bytes()[TABLE_PREFIX.len()..], k.value().as_bytes(), v.value()] {
                    out.extend_from_slice(&u32::try_from(part.len()).map_err(|_| Error::Store("a value is too large to export".into()))?.to_le_bytes());
                    out.extend_from_slice(part);
                }
            }
        }
        Ok(out)
    }

    /// Replace everything in the store with a dump from [`Kv::export`], in one transaction
    /// (a damaged dump changes nothing). Returns the keys restored.
    pub fn import(&self, dump: &[u8]) -> Result<usize> {
        let damaged = || Error::Store("the key-value backup is damaged".into());
        let mut rest = dump.strip_prefix(DUMP_MAGIC).ok_or_else(damaged)?;
        let mut rows: Vec<(String, String, &[u8])> = Vec::new();
        fn take<'a>(rest: &mut &'a [u8], max: usize) -> Option<&'a [u8]> {
            let (len, tail) = rest.split_first_chunk::<4>()?;
            let len = u32::from_le_bytes(*len) as usize;
            if len > max || len > tail.len() {
                return None;
            }
            let (part, tail) = tail.split_at(len);
            *rest = tail;
            Some(part)
        }
        while !rest.is_empty() {
            let ns = std::str::from_utf8(take(&mut rest, 64).ok_or_else(damaged)?).map_err(|_| damaged())?.to_string();
            let key = std::str::from_utf8(take(&mut rest, limits::KV_KEY_BYTES).ok_or_else(damaged)?).map_err(|_| damaged())?.to_string();
            let value = take(&mut rest, limits::KV_VALUE_BYTES + HEADER + NONCE + 16).ok_or_else(damaged)?;
            if !valid_namespace(&ns) || key.is_empty() || value.len() < HEADER {
                return Err(damaged());
            }
            rows.push((ns, key, value));
        }
        let txn = self.0.db.begin_write().map_err(db)?;
        for name in Self::tables(&txn)? {
            txn.delete_table(Def::new(&name)).map_err(db)?;
        }
        for (ns, key, value) in &rows {
            let table = format!("{TABLE_PREFIX}{ns}");
            txn.open_table(Def::new(&table)).map_err(db)?.insert(key.as_str(), *value).map_err(db)?;
        }
        txn.commit().map_err(db)?;
        Ok(rows.len())
    }

    fn check_room(&self) -> Result<()> {
        let cap = self.0.opts.max_file_bytes;
        if cap > 0 && self.size() > cap {
            return Err(Error::Store(format!("the key-value store is full ({} MB; raise kv.maxSizeMb or delete keys)", cap >> 20)));
        }
        Ok(())
    }
}

fn expires_at(raw: &[u8]) -> Option<u64> {
    let at = u64::from_le_bytes(raw.get(1..HEADER)?.try_into().ok()?);
    (at != 0).then_some(at)
}

// ---------------------------------------------------------------------------------------
// Namespaces

/// One namespace of a [`Kv`] store: its own keys, optionally encrypted.
#[derive(Clone)]
pub struct Namespace {
    kv: Kv,
    name: String,
    table: String,
}

/// A key, its value and when it expires, for lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KvEntry {
    /// The key.
    pub key: String,
    /// The value as JSON (a value that is not JSON comes back as a string).
    pub value: serde_json::Value,
    /// When it expires (Unix milliseconds), if it has a TTL.
    pub expires_at: Option<u64>,
}

impl Namespace {
    /// The namespace's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn def(&self) -> Def<'_> {
        Def::new(&self.table)
    }

    fn aad(&self, key: &str, header: &[u8]) -> Vec<u8> {
        [header, self.name.as_bytes(), b"\0", key.as_bytes()].concat()
    }

    fn encode(&self, key: &str, value: &[u8], expires: u64) -> Result<Vec<u8>> {
        crate::untrusted::check_size("a key-value value", value.len(), self.kv.0.opts.max_value_bytes)?;
        let Some(sealer) = self.kv.sealer(&self.name) else {
            return Ok([&[PLAIN][..], &expires.to_le_bytes(), value].concat());
        };
        let header = [&[SEALED][..], &expires.to_le_bytes()].concat();
        let mut nonce = [0u8; NONCE];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce).map_err(|_| Error::Store("no secure random numbers".into()))?;
        let mut body = value.to_vec();
        sealer
            .seal_in_place_append_tag(ring::aead::Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(self.aad(key, &header)), &mut body)
            .map_err(|_| Error::Store("encryption failed".into()))?;
        Ok([header.as_slice(), &nonce, &body].concat())
    }

    /// The value in `raw`, or `None` when it has expired.
    fn decode(&self, key: &str, raw: &[u8], now: u64) -> Result<Option<Vec<u8>>> {
        if raw.len() < HEADER {
            return Err(Error::Store(format!("kv {}/{key}: damaged value", self.name)));
        }
        if expires_at(raw).is_some_and(|at| at <= now) {
            return Ok(None);
        }
        let (header, body) = raw.split_at(HEADER);
        match header[0] {
            PLAIN => Ok(Some(body.to_vec())),
            SEALED => {
                let sealer = self.kv.sealer(&self.name).ok_or_else(|| Error::Store(format!("kv {}/{key} is encrypted and no key is set", self.name)))?;
                if body.len() < NONCE {
                    return Err(Error::Store(format!("kv {}/{key}: damaged value", self.name)));
                }
                let (nonce, sealed) = body.split_at(NONCE);
                let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| Error::Store("damaged nonce".into()))?;
                let mut sealed = sealed.to_vec();
                let plain = sealer
                    .open_in_place(nonce, ring::aead::Aad::from(self.aad(key, header)), &mut sealed)
                    .map_err(|_| Error::Store(format!("kv {}/{key} does not decrypt (wrong key, or the value was changed)", self.name)))?;
                Ok(Some(plain.to_vec()))
            }
            other => Err(Error::Store(format!("kv {}/{key}: unknown value kind {other}", self.name))),
        }
    }

    /// Run `f` on this namespace's table in one write transaction: committed when it returns
    /// `Ok`, rolled back when it returns `Err` (or panics).
    fn write<R>(&self, grows: bool, f: impl FnOnce(&mut Txn<'_, '_>) -> Result<R>) -> Result<R> {
        if grows {
            self.kv.check_room()?;
        }
        let txn = self.kv.0.db.begin_write().map_err(db)?;
        let result = {
            let mut table = txn.open_table(self.def()).map_err(db)?;
            f(&mut Txn { ns: self, table: &mut table, now: self.kv.now() })
        };
        match result {
            Ok(r) => {
                txn.commit().map_err(db)?;
                Ok(r)
            }
            Err(e) => {
                txn.abort().map_err(db)?;
                Err(e)
            }
        }
    }

    /// The value of `key` (`None` when missing or expired).
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        check_key(key)?;
        let txn = self.kv.0.db.begin_read().map_err(db)?;
        let table = match txn.open_table(self.def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(db(e)),
        };
        match table.get(key).map_err(db)? {
            Some(raw) => self.decode(key, raw.value(), self.kv.now()),
            None => Ok(None),
        }
    }

    /// The value of `key` as `T` (`None` when missing or expired).
    pub fn get_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.get(key)?.map(|v| serde_json::from_slice(&v)).transpose().map_err(Error::from)
    }

    /// Set `key` to `value` (no TTL).
    pub fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.write(true, |tx| tx.set(key, value))
    }

    /// Set `key` to `value`; it expires after `ttl`.
    pub fn set_ttl(&self, key: &str, value: &[u8], ttl: Duration) -> Result<()> {
        self.write(true, |tx| tx.set_ttl(key, value, ttl))
    }

    /// Set `key` to `value` as JSON (no TTL).
    pub fn set_json<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        self.set(key, &serde_json::to_vec(value)?)
    }

    /// Set `key` to `value` as JSON; it expires after `ttl`.
    pub fn set_json_ttl<T: Serialize + ?Sized>(&self, key: &str, value: &T, ttl: Duration) -> Result<()> {
        self.set_ttl(key, &serde_json::to_vec(value)?, ttl)
    }

    /// Delete `key`. Returns whether it was there (and not expired).
    pub fn delete(&self, key: &str) -> Result<bool> {
        self.write(false, |tx| tx.delete(key))
    }

    /// Keys starting with `prefix` (all with `""`), in order, without expired ones.
    pub fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self.scan(prefix, false)?.into_iter().map(|(k, _, _)| k).collect())
    }

    /// Keys starting with `prefix` and their values, in key order, without expired ones.
    pub fn list(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        Ok(self.scan(prefix, true)?.into_iter().map(|(k, v, _)| (k, v)).collect())
    }

    /// [`Namespace::list`] as JSON entries with their expiry, for the UI.
    pub fn entries(&self, prefix: &str) -> Result<Vec<KvEntry>> {
        Ok(self
            .scan(prefix, true)?
            .into_iter()
            .map(|(key, v, expires_at)| {
                let value = serde_json::from_slice(&v).unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&v).into_owned()));
                KvEntry { key, value, expires_at }
            })
            .collect())
    }

    fn scan(&self, prefix: &str, values: bool) -> Result<Vec<Row>> {
        let now = self.kv.now();
        let txn = self.kv.0.db.begin_read().map_err(db)?;
        let table = match txn.open_table(self.def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(db(e)),
        };
        let mut out = Vec::new();
        for row in table.range::<&str>(prefix..).map_err(db)? {
            let (k, v) = row.map_err(db)?;
            let key = k.value();
            if !key.starts_with(prefix) {
                break;
            }
            let raw = v.value();
            if expires_at(raw).is_some_and(|at| at <= now) {
                continue;
            }
            let value = if values { self.decode(key, raw, now)?.unwrap_or_default() } else { Vec::new() };
            out.push((key.to_string(), value, expires_at(raw)));
        }
        Ok(out)
    }

    /// Set `key` to `new` only if its value is `expected` now (`None`: missing or expired).
    /// `new = None` deletes it. Returns whether it was swapped; atomic across threads and
    /// processes sharing the store.
    pub fn compare_and_swap(&self, key: &str, expected: Option<&[u8]>, new: Option<&[u8]>) -> Result<bool> {
        self.write(new.is_some(), |tx| {
            if tx.get(key)?.as_deref() != expected {
                return Ok(false);
            }
            match new {
                Some(v) => tx.set(key, v)?,
                None => {
                    tx.delete(key)?;
                }
            }
            Ok(true)
        })
    }

    /// Add `by` to the integer in `key` (a missing key counts as 0) and return the new value.
    /// A TTL on the key is kept.
    pub fn incr(&self, key: &str, by: i64) -> Result<i64> {
        self.write(true, |tx| tx.incr(key, by))
    }

    /// Several reads and writes as one: `f`'s writes are all committed when it returns `Ok`,
    /// and none are when it returns `Err` (or panics).
    pub fn transaction<R>(&self, f: impl FnOnce(&mut Txn<'_, '_>) -> Result<R>) -> Result<R> {
        self.write(true, f)
    }
}

/// A write transaction on one namespace, from [`Namespace::transaction`]. Reads see the
/// transaction's own writes.
pub struct Txn<'a, 't> {
    ns: &'a Namespace,
    table: &'a mut WriteTable<'t>,
    now: u64,
}

impl Txn<'_, '_> {
    /// The value of `key` (`None` when missing or expired).
    pub fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        check_key(key)?;
        match self.table.get(key).map_err(db)? {
            Some(raw) => self.ns.decode(key, raw.value(), self.now),
            None => Ok(None),
        }
    }

    /// The value of `key` as `T`.
    pub fn get_json<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        self.get(key)?.map(|v| serde_json::from_slice(&v)).transpose().map_err(Error::from)
    }

    fn put(&mut self, key: &str, value: &[u8], expires: u64) -> Result<()> {
        check_key(key)?;
        let raw = self.ns.encode(key, value, expires)?;
        self.table.insert(key, raw.as_slice()).map_err(db)?;
        Ok(())
    }

    /// Set `key` to `value` (no TTL).
    pub fn set(&mut self, key: &str, value: &[u8]) -> Result<()> {
        self.put(key, value, 0)
    }

    /// Set `key` to `value`; it expires after `ttl`.
    pub fn set_ttl(&mut self, key: &str, value: &[u8], ttl: Duration) -> Result<()> {
        let ms = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX).max(1);
        self.put(key, value, self.now.saturating_add(ms))
    }

    /// Set `key` to `value` as JSON.
    pub fn set_json<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> Result<()> {
        self.set(key, &serde_json::to_vec(value)?)
    }

    /// Delete `key`; whether it was there (and not expired).
    pub fn delete(&mut self, key: &str) -> Result<bool> {
        check_key(key)?;
        let Some(raw) = self.table.remove(key).map_err(db)? else { return Ok(false) };
        Ok(expires_at(raw.value()).is_none_or(|at| at > self.now))
    }

    /// Add `by` to the integer in `key`; see [`Namespace::incr`].
    pub fn incr(&mut self, key: &str, by: i64) -> Result<i64> {
        check_key(key)?;
        let (old, expires) = match self.table.get(key).map_err(db)? {
            Some(raw) => {
                let raw = raw.value().to_vec();
                match self.ns.decode(key, &raw, self.now)? {
                    Some(v) => (v, expires_at(&raw).unwrap_or(0)),
                    None => (Vec::new(), 0),
                }
            }
            None => (Vec::new(), 0),
        };
        let current: i64 = if old.is_empty() {
            0
        } else {
            serde_json::from_slice(&old).map_err(|_| Error::Source(format!("kv {}/{key} is not an integer", self.ns.name)))?
        };
        let next = current.checked_add(by).ok_or_else(|| Error::Source(format!("kv {}/{key} would overflow", self.ns.name)))?;
        self.put(key, next.to_string().as_bytes(), expires)?;
        Ok(next)
    }
}

// ---------------------------------------------------------------------------------------
// The app's store, opened on first use

/// The app's store in `<data>/kv.redb`, opened on first use with the settings in force
/// (`Engine::kv` uses it). Opening late means an app that never uses the store never
/// creates the file, and a second process (`--mcp` without the worker) only fails when it
/// really uses the store.
pub struct Lazy {
    data_dir: PathBuf,
    open: Mutex<Option<(Kv, Vec<String>)>>,
}

/// The store's file, size, namespaces and exposure, for Settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KvInfo {
    /// The store's file.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// Every namespace with keys.
    pub namespaces: Vec<NamespaceInfo>,
    /// Namespaces the window, the HTTP API and MCP tools may use.
    pub ui_namespaces: Vec<String>,
}

impl Lazy {
    /// The store for the data folder `data_dir` (nothing is opened yet).
    pub fn new(data_dir: &Path) -> Self {
        Self { data_dir: data_dir.to_path_buf(), open: Mutex::new(None) }
    }

    /// The store's file.
    pub fn path(&self) -> PathBuf {
        self.data_dir.join("kv.redb")
    }

    /// Whether the store has been opened in this process.
    pub fn is_open(&self) -> bool {
        self.open.lock().is_ok_and(|o| o.is_some())
    }

    /// The store, opened now if it is not open yet; encrypted namespaces get their key.
    pub fn get(&self, cfg: &KvConfig) -> Result<Kv> {
        let mut open = self.open.lock().map_err(|_| Error::Store("lock poisoned".into()))?;
        if open.is_none() {
            let kv = Kv::open_with(self.path(), cfg.options())?;
            kv.spawn_sweeper(Duration::from_secs(cfg.sweep_minutes * 60))?;
            *open = Some((kv, Vec::new()));
        }
        let (kv, encrypted) = open.as_mut().expect("opened above");
        if *encrypted != cfg.encrypted_namespaces {
            if !cfg.encrypted_namespaces.is_empty() {
                let key = self.key(cfg.key_source)?;
                for ns in &cfg.encrypted_namespaces {
                    kv.encrypt_namespace(ns, &key);
                }
            }
            encrypted.clone_from(&cfg.encrypted_namespaces);
        }
        Ok(kv.clone())
    }

    fn key(&self, source: KeySource) -> Result<KvKey> {
        match source {
            KeySource::File => KvKey::load_or_create(&self.data_dir.join("kv.key")),
{% if oauth %}            #[cfg(feature = "oauth")]
            KeySource::Keychain => KvKey::from_keychain("desktop-core-kv", &self.data_dir.to_string_lossy()),
            #[cfg(not(feature = "oauth"))]
            KeySource::Keychain => Err(Error::NotBuilt("the OS keychain (feature oauth)")),
{% else %}            KeySource::Keychain => Err(Error::NotBuilt("the OS keychain (feature oauth)")),
{% endif %}        }
    }

    /// The store's file, size and namespaces.
    pub fn info(&self, cfg: &KvConfig) -> Result<KvInfo> {
        let kv = self.get(cfg)?;
        Ok(KvInfo { path: kv.path().display().to_string(), bytes: kv.size(), namespaces: kv.namespaces()?, ui_namespaces: cfg.ui_namespaces.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> (Kv, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000));
        let kv = Kv::open_with(dir.join("kv.redb"), KvOptions { clock: clock.clone(), ..KvOptions::default() }).unwrap();
        (kv, clock)
    }

    #[test]
    fn get_set_delete_list_and_namespaces_are_apart() {
        let d = tempfile::tempdir().unwrap();
        let (kv, _) = store(d.path());
        let a = kv.ns("a").unwrap();
        let b = kv.ns("b").unwrap();
        a.set("user:1", b"ann").unwrap();
        a.set("user:2", b"bob").unwrap();
        a.set("zed", b"z").unwrap();
        b.set("user:1", b"other").unwrap();
        assert_eq!(a.get("user:1").unwrap().as_deref(), Some(&b"ann"[..]));
        assert_eq!(b.get("user:1").unwrap().as_deref(), Some(&b"other"[..]));
        assert_eq!(a.keys("user:").unwrap(), ["user:1", "user:2"]);
        assert_eq!(a.list("").unwrap().len(), 3);
        assert!(a.delete("user:1").unwrap());
        assert!(!a.delete("user:1").unwrap());
        assert_eq!(a.get("user:1").unwrap(), None);
        assert_eq!(kv.ns("never").unwrap().get("x").unwrap(), None, "a namespace with no table reads as empty");
        let names: Vec<_> = kv.namespaces().unwrap().into_iter().map(|n| (n.name, n.keys)).collect();
        assert_eq!(names, [("a".to_string(), 2), ("b".to_string(), 1)]);
        assert!(kv.ns("bad name").is_err());
        assert!(a.set("", b"x").is_err());
    }

    #[test]
    fn ttl_expires_lazily_and_the_sweep_deletes() {
        let d = tempfile::tempdir().unwrap();
        let (kv, clock) = store(d.path());
        let ns = kv.ns("s").unwrap();
        ns.set_ttl("short", b"1", Duration::from_secs(10)).unwrap();
        ns.set_json_ttl("long", &2, Duration::from_secs(100)).unwrap();
        ns.set("forever", b"3").unwrap();
        clock.advance(Duration::from_secs(9));
        assert!(ns.get("short").unwrap().is_some());
        clock.advance(Duration::from_secs(1));
        assert_eq!(ns.get("short").unwrap(), None, "expired at exactly its TTL");
        assert_eq!(ns.keys("").unwrap(), ["forever", "long"]);
        assert_eq!(kv.namespaces().unwrap()[0].keys, 3, "still stored until swept");
        assert_eq!(kv.sweep().unwrap(), 1);
        assert_eq!(kv.namespaces().unwrap()[0].keys, 2);
        clock.advance(Duration::from_secs(1000));
        assert_eq!(kv.sweep().unwrap(), 1);
        assert_eq!(ns.get_json::<i32>("long").unwrap(), None);
        assert_eq!(ns.get("forever").unwrap().as_deref(), Some(&b"3"[..]));
    }

    #[test]
    fn incr_keeps_the_ttl_and_refuses_non_integers() {
        let d = tempfile::tempdir().unwrap();
        let (kv, clock) = store(d.path());
        let ns = kv.ns("n").unwrap();
        assert_eq!(ns.incr("hits", 5).unwrap(), 5);
        assert_eq!(ns.incr("hits", -2).unwrap(), 3);
        ns.set_json_ttl("rate", &1, Duration::from_secs(60)).unwrap();
        assert_eq!(ns.incr("rate", 1).unwrap(), 2);
        clock.advance(Duration::from_secs(61));
        assert_eq!(ns.get("rate").unwrap(), None, "the TTL survived incr");
        assert_eq!(ns.incr("rate", 1).unwrap(), 1, "an expired counter starts again");
        ns.set("text", b"\"x\"").unwrap();
        assert!(ns.incr("text", 1).is_err());
        ns.set_json("big", &i64::MAX).unwrap();
        assert!(ns.incr("big", 1).is_err());
    }

    #[test]
    fn compare_and_swap_is_atomic_across_threads() {
        let d = tempfile::tempdir().unwrap();
        let (kv, _) = store(d.path());
        let ns = kv.ns("c").unwrap();
        assert!(ns.compare_and_swap("n", None, Some(b"0")).unwrap());
        assert!(!ns.compare_and_swap("n", None, Some(b"9")).unwrap(), "it exists now");
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let ns = ns.clone();
                std::thread::spawn(move || {
                    let mut wins = 0;
                    while wins < 25 {
                        let cur = ns.get("n").unwrap().unwrap();
                        let next = (String::from_utf8_lossy(&cur).parse::<u32>().unwrap() + 1).to_string();
                        if ns.compare_and_swap("n", Some(&cur), Some(next.as_bytes())).unwrap() {
                            wins += 1;
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(ns.get("n").unwrap().as_deref(), Some(&b"200"[..]), "no update was lost");
        assert!(ns.compare_and_swap("n", Some(b"200"), None).unwrap());
        assert_eq!(ns.get("n").unwrap(), None);
    }

    #[test]
    fn a_failed_transaction_rolls_back() {
        let d = tempfile::tempdir().unwrap();
        let (kv, _) = store(d.path());
        let ns = kv.ns("t").unwrap();
        ns.set("a", b"old").unwrap();
        let r: Result<()> = ns.transaction(|tx| {
            tx.set("a", b"new")?;
            tx.set("b", b"new")?;
            assert_eq!(tx.get("a")?.as_deref(), Some(&b"new"[..]), "reads see the transaction's writes");
            Err(Error::Source("stop".into()))
        });
        assert!(r.is_err());
        assert_eq!(ns.get("a").unwrap().as_deref(), Some(&b"old"[..]));
        assert_eq!(ns.get("b").unwrap(), None);
        // A panic inside rolls back too.
        let ns2 = ns.clone();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = ns2.transaction(|tx| -> Result<()> {
                tx.set("c", b"x")?;
                panic!("boom")
            });
        }));
        assert!(panicked.is_err());
        assert_eq!(ns.get("c").unwrap(), None);
        ns.transaction(|tx| {
            tx.set_json("x", &1)?;
            tx.delete("a").map(|_| ())
        })
        .unwrap();
        assert_eq!(ns.keys("").unwrap(), ["x"]);
    }

    #[test]
    fn encryption_round_trips_and_a_wrong_key_fails() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("kv.redb");
        let key = KvKey::load_or_create(&d.path().join("kv.key")).unwrap();
        assert_eq!(KvKey::load_or_create(&d.path().join("kv.key")).unwrap(), key, "the key file is reused");
        {
            let kv = Kv::open(&path).unwrap();
            kv.encrypt_namespace("secrets", &key);
            let s = kv.ns("secrets").unwrap();
            s.set_json("token", &"hunter2").unwrap();
            assert_eq!(s.get_json::<String>("token").unwrap().as_deref(), Some("hunter2"));
            kv.ns("plain").unwrap().set("x", b"hunter3").unwrap();
        }
        let file = std::fs::read(&path).unwrap();
        assert!(!file.windows(7).any(|w| w == b"hunter2"), "the secret is not in the file");
        assert!(file.windows(7).any(|w| w == b"hunter3"), "other namespaces are plain");
        let kv = Kv::open(&path).unwrap();
        assert!(kv.ns("secrets").unwrap().get("token").is_err(), "no key: refused");
        kv.encrypt_namespace("secrets", &KvKey::generate().unwrap());
        let err = kv.ns("secrets").unwrap().get("token").unwrap_err().to_string();
        assert!(err.contains("does not decrypt"), "{err}");
        kv.encrypt_namespace("secrets", &key);
        assert_eq!(kv.ns("secrets").unwrap().get_json::<String>("token").unwrap().as_deref(), Some("hunter2"));
        assert_eq!(KvKey::from_text(&key.to_text()), Some(key));
    }

    #[test]
    fn a_value_moved_to_another_key_does_not_decrypt() {
        let d = tempfile::tempdir().unwrap();
        let kv = Kv::open(d.path().join("kv.redb")).unwrap();
        kv.encrypt_namespace("s", &KvKey::generate().unwrap());
        kv.ns("s").unwrap().set("a", b"secret").unwrap();
        // Copy the sealed bytes of "a" under "b", as someone with the file could.
        let dump = kv.export().unwrap();
        let raw = dump[dump.len() - (HEADER + NONCE + 6 + 16)..].to_vec();
        let txn = kv.0.db.begin_write().unwrap();
        txn.open_table(Def::new("kv:s")).unwrap().insert("b", raw.as_slice()).unwrap();
        txn.commit().unwrap();
        assert!(kv.ns("s").unwrap().get("a").unwrap().is_some());
        assert!(kv.ns("s").unwrap().get("b").is_err());
    }

    #[test]
    fn size_caps() {
        let d = tempfile::tempdir().unwrap();
        let kv = Kv::open_with(d.path().join("kv.redb"), KvOptions { max_value_bytes: 8, ..KvOptions::default() }).unwrap();
        let ns = kv.ns("s").unwrap();
        assert!(ns.set("k", b"12345678").is_ok());
        assert!(ns.set("k", b"123456789").is_err());
        assert!(ns.set(&"k".repeat(limits::KV_KEY_BYTES + 1), b"1").is_err());
        let full = Kv::open_with(d.path().join("full.redb"), KvOptions { max_file_bytes: 1, ..KvOptions::default() }).unwrap();
        let err = full.ns("s").unwrap().set("k", b"v").unwrap_err().to_string();
        assert!(err.contains("full"), "{err}");
        assert!(full.ns("s").unwrap().delete("k").is_ok(), "deleting still works when full");
    }

    #[test]
    fn reopening_after_drop_keeps_committed_writes() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("kv.redb");
        {
            let kv = Kv::open(&path).unwrap();
            kv.ns("s").unwrap().set("kept", b"1").unwrap();
            let _ = kv.ns("s").unwrap().transaction(|tx| -> Result<()> {
                tx.set("lost", b"1")?;
                Err(Error::Source("abandoned".into()))
            });
            // Dropped without any close call, as when the process ends.
        }
        let kv = Kv::open(&path).unwrap();
        assert_eq!(kv.ns("s").unwrap().keys("").unwrap(), ["kept"]);
        assert!(Kv::open(&path).is_err(), "one open store per file");
    }

    #[test]
    fn export_and_import_replace_everything() {
        let d = tempfile::tempdir().unwrap();
        let (kv, clock) = store(d.path());
        kv.ns("a").unwrap().set("1", b"one").unwrap();
        kv.ns("b").unwrap().set_ttl("2", b"two", Duration::from_secs(5)).unwrap();
        kv.ns("b").unwrap().set_ttl("gone", b"x", Duration::from_secs(1)).unwrap();
        clock.advance(Duration::from_secs(2));
        let dump = kv.export().unwrap();
        kv.ns("a").unwrap().set("extra", b"x").unwrap();
        kv.ns("c").unwrap().set("x", b"x").unwrap();
        assert_eq!(kv.import(&dump).unwrap(), 2, "the expired key is not exported");
        assert_eq!(kv.ns("a").unwrap().keys("").unwrap(), ["1"]);
        assert!(kv.ns("c").unwrap().keys("").unwrap().is_empty());
        clock.advance(Duration::from_secs(5));
        assert_eq!(kv.ns("b").unwrap().get("2").unwrap(), None, "the TTL came back with it");
        assert!(kv.import(b"nonsense").is_err());
        assert!(kv.import(&dump[..dump.len() - 1]).is_err(), "a truncated dump changes nothing");
        assert_eq!(kv.ns("a").unwrap().keys("").unwrap(), ["1"]);
    }

    #[test]
    fn only_listed_namespaces_are_exposed() {
        let cfg = KvConfig::default();
        assert!(cfg.allows("ui"));
        assert!(!cfg.allows("secrets"));
        assert!(cfg.check_exposed("secrets").unwrap_err().to_string().starts_with("Not allowed"));
        let cfg = KvConfig { ui_namespaces: vec!["ui".into(), " prefs ".into(), "bad name".into(), "ui".into()], ..KvConfig::default() }.validated();
        assert_eq!(cfg.ui_namespaces, ["prefs", "ui"]);
        let cfg = KvConfig { ui_namespaces: Vec::new(), ..KvConfig::default() };
        assert!(!cfg.allows("ui"), "an empty list exposes nothing");
    }

    #[test]
    fn the_engine_backs_up_and_restores_the_store() {
        let d = tempfile::tempdir().unwrap();
        let store = crate::FileStore::open(&d.path().join("store")).unwrap();
        let e = crate::Engine::with_parts(Box::new(store), None, d.path(), &mut |_| {}).unwrap();
        e.kv().unwrap().ns("settings").unwrap().set_json("a", &1).unwrap();
        assert!(e.kv_exposed("settings").is_err(), "not in kv.uiNamespaces");
        e.kv_exposed("ui").unwrap().set_json("b", &2).unwrap();
        let info = e.backup().unwrap();
        assert!(crate::backup::part_path(&info.path, "kv").exists());
        e.kv().unwrap().ns("settings").unwrap().set_json("a", &9).unwrap();
        e.restore(&info.path, None).unwrap();
        assert_eq!(e.kv().unwrap().ns("settings").unwrap().get_json::<i32>("a").unwrap(), Some(1));
        assert_eq!(e.kv_info().unwrap().namespaces.len(), 2);
    }

    #[test]
    fn the_lazy_store_opens_once_and_applies_encryption() {
        let d = tempfile::tempdir().unwrap();
        let lazy = Lazy::new(d.path());
        assert!(!lazy.is_open() && !lazy.path().exists());
        let cfg = KvConfig { encrypted_namespaces: vec!["secrets".into()], ..KvConfig::default() };
        lazy.get(&cfg).unwrap().ns("secrets").unwrap().set("k", b"secret").unwrap();
        assert!(lazy.is_open() && d.path().join("kv.key").exists());
        let info = lazy.info(&cfg).unwrap();
        assert_eq!(info.namespaces, [NamespaceInfo { name: "secrets".into(), keys: 1, encrypted: true }]);
        assert_eq!(info.ui_namespaces, ["ui"]);
    }
}
