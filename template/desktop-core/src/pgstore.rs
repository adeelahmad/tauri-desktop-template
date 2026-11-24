//! Embedded Postgres 16 with pgvector and, with feature `bm25`, VectorChord-BM25.
//!
//! The server runs from a private data folder (`postgresql_embedded`), listens on
//! localhost only with a random password, and on first use installs one extension bundle
//! (pgvector, pg_tokenizer, VectorChord-BM25) built for this platform by the
//! `pg-extensions` workflow. The bundle's SHA-256 is checked before it is unpacked.
//! Vector search runs on an HNSW cosine index; keyword search uses BM25 when it is on and
//! installed, else Postgres full text; hybrid mode fuses both (see `fuse`).
//!
//! The server binaries come from their direct release download URL
//! (`github.com/theseus-rs/postgresql-binaries/releases/download/...`), checked against a
//! SHA-256 pinned here, so the first start makes no GitHub API call (that API allows only
//! 60 unauthenticated requests an hour per IP). They are kept in the data folder, so later
//! starts work offline. If the direct download fails and `GITHUB_TOKEN` is set, the crate's
//! own API-based download is tried instead.
//!
//! - `APP_PG_URL`: use an existing server instead (it needs pgvector; BM25 is used when
//!   `vchord_bm25` and `pg_tokenizer` are installed there).
//! - `APP_PG_EXT_DIR`: install the bundle from a local folder (`lib/`, `share/extension/`)
//!   instead of downloading it, for offline installers.
//!
//! A data folder made by an older major Postgres is upgraded with `pg_upgrade` as the
//! `postgres.upgrade` setting says ([`PostgresConfig`]); see [`crate::pgupgrade`] and
//! `<app> --pg-upgrade [--check]`.
//!
//! # Example
//!
//! The first open downloads Postgres and the extensions:
//!
//! ```no_run
//! use desktop_core::pgstore::PostgresStore;
//! use desktop_core::{chunks_from_text, SearchConfig, Store};
//!
//! let store = PostgresStore::open("data".as_ref())?;
//! store.add(&chunks_from_text("a.md", "Postgres keeps the chunks."), None)?;
//! println!("{:?}", store.search("chunks", None, &SearchConfig::default())?);
//! desktop_core::pgstore::shutdown();
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::config::{Config, KeywordEngine, SearchConfig};
use crate::fuse::{self, Candidate};
use crate::store::{plan, Hit, Plan, Store};
use crate::pgupgrade::{self, Job, Layout, Method, Need, PlanOptions, UpgradeMode, UPGRADE_NEEDED};
use crate::{Chunk, Error, Result};
use postgresql_embedded::blocking::PostgreSQL;
use postgresql_embedded::{Settings, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The exact server the extension bundle is built against.
pub const PG_VERSION: &str = "16.16.0";
/// Release asset with this platform's extensions; `{target}` becomes the target triple.
const EXT_URL: &str = env!("APP_PG_EXT_URL");
/// `sha256  file` lines (the release's SHA256SUMS) that pin each bundle.
const EXT_SHA256: &str = include_str!("../pg-ext.sha256");
const TARGET: &str = env!("APP_TARGET");

const DATABASE: &str = "app";

/// Where the server archives are published (no API: plain release downloads).
const SERVER_RELEASES: &str = "https://github.com/theseus-rs/postgresql-binaries/releases/download";
/// SHA-256 of each `postgresql-{PG_VERSION}-{target}.tar.gz` (from the release's `.sha256` files).
const SERVER_SHA256: &[(&str, &str)] = &[
    ("aarch64-apple-darwin", "b7f002d554cd8a916d189e24b8f885d088b7909bb6d2de092f82c10274cb8251"),
    ("x86_64-pc-windows-msvc", "4557b8251bb967c1c09eab57439fb2f1179d35c70debfc692fe637bb08d2f20c"),
    ("x86_64-unknown-linux-gnu", "f668fbc685cdcb71d6a82f57035bed7d97b059e2d8a1d3adf317ec36c5e861d2"),
];

static SERVERS: OnceLock<Mutex<HashMap<PathBuf, Running>>> = OnceLock::new();

fn pg_err(e: impl std::fmt::Display) -> Error {
    Error::Store(format!("Postgres: {e}"))
}

fn password(dir: &Path) -> Result<String> {
    let file = dir.join(".password");
    if let Ok(p) = std::fs::read_to_string(&file) {
        return Ok(p.trim().to_string());
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| Error::Store(format!("no secure random numbers: {e}")))?;
    let out: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    write_private(&file, out.as_bytes())?;
    Ok(out)
}

/// Write a file only this user can read (0600 on Unix; Windows keeps the per-user ACL of
/// the app data folder).
fn write_private(file: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(file)?;
        f.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    std::fs::write(file, bytes)?;
    Ok(())
}

/// Lock a folder to this user (0700 on Unix).
fn make_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The extension bundle is built for macOS arm64, Windows x86-64 and Linux x86-64.
/// pgvector there is compiled without `-march=native`, so any CPU of those kinds works.
pub fn embedded_supported() -> bool {
    cfg!(any(all(target_os = "macos", target_arch = "aarch64"), all(any(windows, target_os = "linux"), target_arch = "x86_64")))
}

/// The pinned SHA-256 for `file`, if `pg-ext.sha256` lists it.
fn pinned_sha256(file: &str) -> Option<&'static str> {
    EXT_SHA256
        .lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(_, f)| f.trim().trim_start_matches('*') == file)
        .map(|(h, _)| h)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Unpack the extension bundle into the server's install folder (once per bundle).
fn install_extensions(root: &Path) -> Result<()> {
    let file = format!("pg16-extensions-{TARGET}.zip");
    let marker = root.join(format!(".app-extensions-{}", sha256_hex(EXT_URL.as_bytes())));
    if marker.exists() {
        return Ok(());
    }
    if let Some(dir) = std::env::var_os("APP_PG_EXT_DIR").filter(|d| !d.is_empty()) {
        let dir = PathBuf::from(dir);
        copy_tree(&dir.join("lib"), &root.join("lib"))?;
        copy_tree(&dir.join("share").join("extension"), &root.join("share").join("extension"))?;
    } else {
        let url = EXT_URL.replace("{target}", TARGET);
        let bytes = crate::net::get_bytes(&url, 200 * 1024 * 1024).map_err(|e| pg_err(format!("extension download ({url}): {e}")))?;
        let got = sha256_hex(&bytes);
        match pinned_sha256(&file) {
            Some(want) if want.eq_ignore_ascii_case(&got) => {}
            Some(want) => return Err(pg_err(format!("{file}: SHA-256 {got} does not match the pinned {want}"))),
            None => {
                // Not pinned in this build: at least check it against the release's own list.
                let sums_url = url.rsplit_once('/').map(|(base, _)| format!("{base}/SHA256SUMS")).unwrap_or_default();
                let sums = String::from_utf8(crate::net::get_bytes(&sums_url, 1024 * 1024).map_err(pg_err)?).unwrap_or_default();
                let listed = sums.lines().any(|l| l.split_whitespace().collect::<Vec<_>>() == [got.as_str(), file.as_str()]);
                if !listed {
                    return Err(pg_err(format!("{file}: SHA-256 {got} is not in {sums_url}")));
                }
            }
        }
        unzip_into(&bytes, root)?;
    }
    write_private(&marker, b"")?;
    Ok(())
}

/// Extract `lib/*` and `share/extension/*` of the zip in `bytes` into `root`, refusing paths
/// that climb out (zip slip), more than
/// [`limits::ZIP_ENTRIES`](crate::untrusted::limits::ZIP_ENTRIES) entries and more than
/// [`limits::ZIP_UNPACKED_BYTES`](crate::untrusted::limits::ZIP_UNPACKED_BYTES) unpacked
/// (a zip bomb). Public so the fuzz target can reach it.
pub fn unzip_into(bytes: &[u8], root: &Path) -> Result<()> {
    use crate::untrusted::limits::{ZIP_ENTRIES, ZIP_UNPACKED_BYTES};
    unzip_limited(bytes, root, ZIP_ENTRIES, ZIP_UNPACKED_BYTES)
}

/// [`unzip_into`] with chosen limits: at most `max_entries` entries and `max_bytes` unpacked.
pub fn unzip_limited(bytes: &[u8], root: &Path, max_entries: usize, max_bytes: u64) -> Result<()> {
    crate::untrusted::guard("extension bundle", || {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(pg_err)?;
        if zip.len() > max_entries {
            return Err(pg_err(format!("the extension bundle has more than {max_entries} files")));
        }
        let mut left = max_bytes;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(pg_err)?;
            let Some(rel) = entry.enclosed_name() else { return Err(pg_err("unsafe path in extension bundle")) };
            let allowed = rel.starts_with("lib") || rel.starts_with(Path::new("share").join("extension"));
            if !allowed || entry.is_dir() {
                continue;
            }
            let out = root.join(&rel);
            std::fs::create_dir_all(out.parent().unwrap_or(root))?;
            let mut f = std::fs::File::create(&out)?;
            // Count what is really inflated: the sizes in the zip's headers are the file's claim.
            let n = std::io::copy(&mut std::io::Read::take(&mut entry, left + 1), &mut f)?;
            if n > left {
                drop(f);
                let _ = std::fs::remove_file(&out);
                return Err(pg_err("the extension bundle unpacks to more than the size limit"));
            }
            left -= n;
        }
        Ok(())
    })
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// The server archive's file name for `version` and `target`.
fn server_asset(version: &str, target: &str) -> String {
    format!("postgresql-{version}-{target}.tar.gz")
}

/// The pinned SHA-256 of the server archive for `target`, if this build knows it.
fn server_sha256(target: &str) -> Option<&'static str> {
    SERVER_SHA256.iter().find(|(t, _)| *t == target).map(|(_, h)| *h)
}

/// The first SHA-256 in a `.sha256` file (`<hex>  <file>`).
fn parse_sha256_file(text: &str) -> Option<&str> {
    text.split_whitespace().next().filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Download the bundled server archive from its direct release URL (no GitHub API), check
/// its SHA-256 and unpack it into `dir` (the crate's versioned install folder).
fn install_server(dir: &Path) -> Result<()> {
    install_server_version(dir, PG_VERSION, server_sha256(postgresql_archive::matcher::target()))
}

/// Download server `version` into `dir`, checked against `pinned` (or, when this build does
/// not pin it, the release's own `.sha256` file).
fn install_server_version(dir: &Path, version: &str, pinned: Option<&str>) -> Result<()> {
    let target = postgresql_archive::matcher::target();
    let asset = server_asset(version, target);
    let url = format!("{SERVER_RELEASES}/{version}/{asset}");
    let bytes = crate::net::get_bytes(&url, 512 * 1024 * 1024).map_err(|e| pg_err(format!("server download ({url}): {e}")))?;
    let got = sha256_hex(&bytes);
    let want = match pinned {
        Some(h) => h.to_string(),
        None => {
            // Not pinned in this build: use the release's own checksum file.
            let sums = crate::net::get_bytes(&format!("{url}.sha256"), 4096).map_err(pg_err)?;
            parse_sha256_file(&String::from_utf8_lossy(&sums)).ok_or_else(|| pg_err(format!("no SHA-256 in {url}.sha256")))?.to_string()
        }
    };
    if !want.eq_ignore_ascii_case(&got) {
        return Err(pg_err(format!("{asset}: SHA-256 {got} does not match {want}")));
    }
    let mut dirs = postgresql_archive::extractor::ExtractDirectories::default();
    let all = regex_lite::Regex::new(".*").map_err(pg_err)?;
    dirs.add_mapping(all, dir.to_path_buf());
    postgresql_archive::configuration::theseus::extract(&bytes, &dirs).map_err(pg_err)?;
    Ok(())
}

/// Whether the embedded server can run here (or `APP_PG_URL` is set).
pub fn available() -> bool {
    std::env::var("APP_PG_URL").is_ok_and(|u| !u.is_empty()) || embedded_supported()
}

/// Postgres settings (`postgres` in the settings).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct PostgresConfig {
    /// What to do when the data folder was made by an older major Postgres: `auto`
    /// (upgrade at start), `ask` (the UI asks first) or `off` (keep the old server).
    pub upgrade: UpgradeMode,
    /// Upgrade with `pg_upgrade --copy` instead of `--link`: slower and twice the disk
    /// space, but the old folder stays usable whatever happens.
    pub upgrade_copy: bool,
}

/// The bundled major version (16).
pub fn bundled_major() -> u32 {
    pgupgrade::parse_major(PG_VERSION).unwrap_or(16)
}

/// One embedded server per data folder, started on first use; returns its URL. A data
/// folder from an older major Postgres is handled as `postgres.upgrade = "ask"` does (an
/// error starting with [`pgupgrade::UPGRADE_NEEDED`]); see [`server_url_with`].
pub fn server_url(data_dir: &Path) -> Result<String> {
    server_url_with(data_dir, &PostgresConfig::default(), &mut |_| {}).map(|(url, _)| url)
}

/// [`server_url`] with the Postgres settings, and progress lines for the setup screen
/// ("Upgrading database: ..."). Also returns a notice for the user when the server runs,
/// but not as it should (an upgrade that is off or failed, so the old major still runs).
pub fn server_url_with(data_dir: &Path, cfg: &PostgresConfig, progress: &mut dyn FnMut(&str)) -> Result<(String, Option<String>)> {
    if let Ok(url) = std::env::var("APP_PG_URL") {
        if !url.is_empty() {
            return Ok((url, None));
        }
    }
    if !embedded_supported() {
        return Err(Error::Store(
            "embedded Postgres is not built for this platform (macOS arm64, Windows x64 and Linux x64 are); use the file store or set APP_PG_URL"
                .into(),
        ));
    }
    make_private_dir(data_dir)?;
    let key = plain_path(std::fs::canonicalize(data_dir)?);
    let mut servers = SERVERS.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(pg_err)?;
    if let Some(s) = servers.get(&key) {
        return Ok((s.url.clone(), s.notice.clone()));
    }
    let layout = Layout { root: key.clone() };
    if let Some(done) = pgupgrade::recover(&layout)? {
        log::warn!("{done}");
    }
    let (pg, notice) = match pgupgrade::need(&layout.data(), bundled_major())? {
        Need::Fresh | Need::Current => (prepare(&key, PG_VERSION)?, None),
        Need::Newer { found, bundled } => {
            return Err(Error::Store(format!(
                "the database was made by Postgres {found}, newer than this app's Postgres {bundled}; install the newer version of the app"
            )))
        }
        Need::Upgrade { from, to } => {
            let failed = pgupgrade::last_failure(&layout);
            // After a failed automatic upgrade, ask again rather than retry at every start.
            let mode = match (cfg.upgrade, &failed) {
                (UpgradeMode::Auto, Some(_)) => UpgradeMode::Ask,
                (m, _) => m,
            };
            match mode {
                UpgradeMode::Ask => {
                    return Err(Error::Store(format!(
                        "{UPGRADE_NEEDED}: the database was made by Postgres {from} and this version of the app uses Postgres {to}. Upgrade it now? The old folder is kept until the new one works.{}",
                        failed.map(|f| format!(" (The last try failed: {f})")).unwrap_or_default()
                    )))
                }
                UpgradeMode::Off => {
                    let note = format!("The database still runs on Postgres {from}: upgrades are off (Settings, Database), and this app is made for Postgres {to}.");
                    old_server(&key, &layout, from, note)?
                }
                UpgradeMode::Auto => match upgrade(&key, &layout, from, to, cfg, false, progress) {
                    Ok(pg) => (pg, None),
                    Err(f) if f.rolled_back => {
                        progress(&f.message);
                        old_server(&key, &layout, from, f.message)?
                    }
                    Err(f) => return Err(Error::Store(f.message)),
                },
            }
        }
    };
    let mut pg = pg;
    launch(&mut pg)?;
    let url = pg.settings().url(DATABASE);
    if let Some(n) = &notice {
        log::warn!("{n}");
    }
    servers.insert(key, Running { pg, url: url.clone(), notice: notice.clone() });
    Ok((url, notice))
}

/// Run the upgrade now, whatever `postgres.upgrade` says (`<app> --pg-upgrade [--check]`).
/// With `check_only`, only `pg_upgrade --check` runs and nothing changes. Returns what
/// happened, for the terminal. The server it starts keeps running until [`shutdown`].
pub fn upgrade_now(data_dir: &Path, cfg: &PostgresConfig, check_only: bool, progress: &mut dyn FnMut(&str)) -> Result<String> {
    if std::env::var("APP_PG_URL").is_ok_and(|u| !u.is_empty()) {
        return Ok("APP_PG_URL is set: that server is upgraded by whoever runs it".into());
    }
    make_private_dir(data_dir)?;
    let key = plain_path(std::fs::canonicalize(data_dir)?);
    let layout = Layout { root: key.clone() };
    if let Some(done) = pgupgrade::recover(&layout)? {
        progress(&done);
    }
    let (from, to) = match pgupgrade::need(&layout.data(), bundled_major())? {
        Need::Upgrade { from, to } => (from, to),
        Need::Fresh => return Ok("there is no database yet; nothing to upgrade".into()),
        Need::Current => return Ok(format!("the database is already on Postgres {}", bundled_major())),
        Need::Newer { found, bundled } => return Err(Error::Store(format!("the database was made by Postgres {found}, newer than this app's {bundled}"))),
    };
    if check_only {
        let _servers = SERVERS.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(pg_err)?;
        upgrade(&key, &layout, from, to, cfg, true, progress).map_err(|f| Error::Store(f.message))?;
        return Ok(format!("pg_upgrade --check passed: Postgres {from} -> {to} can go ahead"));
    }
    pgupgrade::clear_failure(&layout)?;
    let (_, notice) = server_url_with(data_dir, &PostgresConfig { upgrade: UpgradeMode::Auto, ..cfg.clone() }, progress)?;
    match notice {
        Some(n) => Err(Error::Store(n)),
        None => Ok(format!("the database is now on Postgres {to}")),
    }
}

/// A running embedded server.
struct Running {
    pg: PostgreSQL,
    url: String,
    notice: Option<String>,
}

/// Whether `root/lib` has the shared library `name` (`vector.so`, `vector.dylib`, ...).
fn has_lib(root: &Path, name: &str) -> bool {
    std::fs::read_dir(root.join("lib")).is_ok_and(|rd| {
        rd.filter_map(|e| e.ok()).any(|e| {
            let p = e.path();
            p.file_stem().is_some_and(|s| s == name) && p.extension().is_some_and(|x| x == "so" || x == "dylib" || x == "dll")
        })
    })
}

/// Server options that preload BM25's libraries, when they are built in and in `root`.
fn preload(root: &Path) -> Option<String> {
    (cfg!(feature = "bm25") && has_lib(root, "vchord_bm25") && has_lib(root, "pg_tokenizer")).then(|| "pg_tokenizer,vchord_bm25".to_string())
}

/// The server for `version`, installed (the bundled one downloads on first use, with its
/// extensions) and with a cluster in `data`, but not started.
fn prepare(key: &Path, version: &str) -> Result<PostgreSQL> {
    let bundled = version == PG_VERSION;
    let mut configuration = HashMap::new();
    // Local connections only.
    configuration.insert("listen_addresses".to_string(), "localhost".to_string());
    let settings = Settings {
        version: VersionReq::parse(&format!("={version}")).map_err(pg_err)?,
        installation_dir: key.join("install"),
        data_dir: key.join("data"),
        password_file: key.join(".pgpass"),
        password: password(key)?,
        host: "localhost".into(),
        temporary: false,
        configuration,
        // The crate's 5 s default is too short for a first start (initdb, and macOS checking
        // the new binaries), which then fails with "deadline has elapsed".
        timeout: Some(std::time::Duration::from_secs(60)),
        ..Settings::default()
    };
    if bundled && !key.join("install").join(PG_VERSION).is_dir() {
        // Postgres downloads through its own client, so ask the network policy first.
        crate::net::check("https://github.com/").map_err(|e| pg_err(format!("Postgres is not installed yet: {e}")))?;
    }
    let mut pg = PostgreSQL::new(settings);
    // `new` appends the exact version, so this is `install/<version>`; once it exists the
    // crate uses it as is and never goes online.
    let install_dir = pg.settings().installation_dir.clone();
    if bundled && !install_dir.is_dir() {
        if let Err(e) = install_server(&install_dir) {
            // The crate's own download lists releases through the GitHub API, which only works
            // reliably with a token.
            if std::env::var("GITHUB_TOKEN").map_or(true, |t| t.is_empty()) {
                return Err(e);
            }
            log::warn!("{e}; trying the GitHub API with GITHUB_TOKEN");
        }
    }
    pg.setup().map_err(pg_err)?;
    let root = pg.settings().binary_dir().parent().map(Path::to_path_buf).ok_or_else(|| pg_err("no install folder"))?;
    if bundled {
        // The extension bundle is built for the bundled major only.
        install_extensions(&root)?;
    }
    // BM25's libraries are loaded at start when they are built in (and, on an old server,
    // only when that server has them).
    if let Some(libs) = preload(&root) {
        let mut settings = pg.settings().clone();
        settings.configuration.insert("shared_preload_libraries".to_string(), libs);
        // Not dropped: a dropped `PostgreSQL` stops a server running on the same folder.
        std::mem::forget(std::mem::replace(&mut pg, PostgreSQL::new(settings)));
    }
    Ok(pg)
}

/// Start a prepared server and make the app's database.
fn launch(pg: &mut PostgreSQL) -> Result<()> {
    if pg.status() != postgresql_embedded::Status::Started {
        pg.start().map_err(pg_err)?;
    }
    if !pg.database_exists(DATABASE).map_err(pg_err)? {
        pg.create_database(DATABASE).map_err(pg_err)?;
    }
    Ok(())
}

/// The old major's server version, from the install folder or else downloaded (pinned).
fn ensure_old(layout: &Layout, from: u32) -> Result<String> {
    if let Some(v) = pgupgrade::installed_version(&layout.install(), from) {
        return Ok(v);
    }
    let old = pgupgrade::old_server(from).ok_or_else(|| Error::Store(format!("this build cannot get a Postgres {from} server to read the old database")))?;
    fetch_old(layout, old.version)?;
    Ok(old.version.to_string())
}

fn fetch_old(layout: &Layout, version: &str) -> Result<()> {
    crate::net::check("https://github.com/").map_err(|e| pg_err(format!("the Postgres {version} server is not here: {e}")))?;
    let pinned = pgupgrade::parse_major(version).and_then(pgupgrade::old_server).filter(|s| s.version == version);
    install_server_version(&layout.install().join(version), version, pinned.and_then(|s| s.sha256_for(postgresql_archive::matcher::target())))
}

/// Run on the old major's server (upgrade off, or rolled back), with `note` for the user.
fn old_server(key: &Path, layout: &Layout, from: u32, note: String) -> Result<(PostgreSQL, Option<String>)> {
    let version = ensure_old(layout, from)?;
    let pg = prepare(key, &version)?;
    let root = layout.install().join(&version);
    let note = if has_lib(&root, "vector") { note } else { format!("{note} That server has no pgvector, so vector search will fail until the upgrade.") };
    Ok((pg, Some(note)))
}

/// Upgrade `data` from `from` to `to` (or only check). Returns the new server, started.
fn upgrade(
    key: &Path,
    layout: &Layout,
    from: u32,
    to: u32,
    cfg: &PostgresConfig,
    check_only: bool,
    progress: &mut dyn FnMut(&str),
) -> std::result::Result<PostgreSQL, pgupgrade::Failure> {
    let before = |e: Error| pgupgrade::Failure {
        rolled_back: true,
        message: format!("Could not get ready to upgrade the database from Postgres {from} to {to}: {e}. Nothing was changed."),
    };
    progress("Upgrading database");
    let cached = pgupgrade::installed_version(&layout.install(), from);
    let old_version = match (&cached, pgupgrade::old_server(from)) {
        (Some(v), _) => v.clone(),
        (None, Some(s)) => s.version.to_string(),
        (None, None) => return Err(before(Error::Store(format!("this build has no Postgres {from} server to upgrade from")))),
    };
    // The new server and its extensions must be there before `pg_upgrade --check`.
    let mut pg = prepare(key, PG_VERSION).map_err(before)?;
    let new_bin = pg.settings().binary_dir();
    let old_root = layout.install().join(&old_version);
    let method = if cfg.upgrade_copy { Method::Copy } else { Method::Link };
    let options = |libs: Option<String>| libs.map(|l| format!("-c shared_preload_libraries={l}")).unwrap_or_default();
    let job = Job {
        layout: layout.clone(),
        from,
        to,
        old_bin: old_root.join("bin"),
        new_options: options(new_bin.parent().and_then(preload)),
        new_bin,
        password: pg.settings().password.clone(),
        password_file: pg.settings().password_file.clone(),
        // The old server learns its libraries once it is here (it may be downloaded first).
        old_options: if old_root.is_dir() { options(preload(&old_root)) } else { String::new() },
        extensions: if cfg!(feature = "bm25") { vec!["vector", "pg_tokenizer", "vchord_bm25"] } else { vec!["vector"] },
        method,
        socket_dir: std::env::temp_dir().join(format!("pgu-{}", std::process::id())),
    };
    let steps = pgupgrade::plan(&PlanOptions { old_version, old_cached: cached.is_some(), method, check_only });
    let mut runner = Real { layout, pg: &mut pg, client: None };
    let result = pgupgrade::execute(&job, &steps, &mut runner, progress);
    let _ = std::fs::remove_dir_all(&job.socket_dir);
    drop(runner);
    result.map(|()| pg)
}

/// The real side effects of an upgrade.
struct Real<'a> {
    layout: &'a Layout,
    pg: &'a mut PostgreSQL,
    client: Option<postgres::Client>,
}

impl pgupgrade::Runner for Real<'_> {
    fn exec(&mut self, cmd: &pgupgrade::Cmd) -> Result<String> {
        pgupgrade::exec_command(cmd)
    }

    fn fetch(&mut self, version: &str) -> Result<()> {
        fetch_old(self.layout, version)
    }

    fn start(&mut self) -> Result<()> {
        launch(self.pg)
    }

    fn query(&mut self, sql: &str) -> Result<String> {
        if self.client.is_none() {
            self.client = Some(postgres::Client::connect(&self.pg.settings().url(DATABASE), postgres::NoTls).map_err(pg_err)?);
        }
        let db = self.client.as_mut().ok_or_else(|| pg_err("no connection"))?;
        let first = db.simple_query(sql).map_err(pg_err)?.into_iter().find_map(|m| match m {
            postgres::SimpleQueryMessage::Row(r) => Some(r.get(0).unwrap_or_default().to_string()),
            _ => None,
        });
        Ok(first.unwrap_or_default())
    }

    fn stop(&mut self) -> Result<()> {
        self.client = None;
        self.pg.stop().map_err(pg_err)
    }
}

/// Stop every server this process started (call on app exit).
pub fn shutdown() {
    if let Some(servers) = SERVERS.get() {
        if let Ok(mut servers) = servers.lock() {
            for (_, s) in servers.drain() {
                let _ = s.pg.stop();
            }
        }
    }
}

/// A [`Store`](crate::Store) in Postgres with pgvector (and BM25 when built in).
pub struct PostgresStore {
    client: Mutex<postgres::Client>,
    /// The pg_tokenizer tokenizer BM25 uses, once BM25 is set up.
    bm25: Mutex<Option<String>>,
    /// Why the server does not run as it should (an upgrade that is off or failed).
    notice: Option<String>,
}

/// Vector dimension of the stored embeddings (kept in `meta`), which the HNSW index needs.
fn dim(db: &mut postgres::Client) -> Result<Option<usize>> {
    meta(db, "dim").map(|d| d.and_then(|d| d.parse().ok()))
}

fn meta(db: &mut postgres::Client, key: &str) -> Result<Option<String>> {
    let row = db.query_opt("select value from meta where key = $1", &[&key]).map_err(pg_err)?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>(0)))
}

fn vector_literal(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    format!("[{}]", parts.join(","))
}

/// Whether this server has the BM25 extensions available to install.
fn bm25_installable(db: &mut postgres::Client) -> Result<bool> {
    let n: i64 = db
        .query_one("select count(*) from pg_available_extensions where name in ('pg_tokenizer', 'vchord_bm25')", &[])
        .map_err(pg_err)?
        .get(0);
    Ok(n == 2)
}

impl PostgresStore {
    /// Start (or reach) the server for `data_dir` and bring its schema up to date.
    /// The first call downloads Postgres and the extensions.
    pub fn open(data_dir: &Path) -> Result<Self> {
        Self::open_with(data_dir, &PostgresConfig::default(), &mut |_| {})
    }

    /// [`open`](Self::open) with the Postgres settings (what to do with a data folder from
    /// an older major Postgres) and progress lines for the setup screen.
    pub fn open_with(data_dir: &Path, cfg: &PostgresConfig, progress: &mut dyn FnMut(&str)) -> Result<Self> {
        let (url, notice) = server_url_with(data_dir, cfg, progress)?;
        let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(pg_err)?;
        migrate(&mut client)?;
        Ok(Self { client: Mutex::new(client), bm25: Mutex::new(None), notice })
    }

    fn db(&self) -> Result<std::sync::MutexGuard<'_, postgres::Client>> {
        self.client.lock().map_err(|_| Error::Store("lock poisoned".into()))
    }

    fn bm25_tokenizer(&self) -> Option<String> {
        self.bm25.lock().ok().and_then(|t| t.clone())
    }

    /// Install the BM25 extensions, make the tokenizer, add and fill the `bm25` column.
    /// A new tokenizer model re-tokenizes every chunk.
    fn setup_bm25(&self, model: &str) -> Result<String> {
        let tokenizer = format!("app_{model}");
        let mut db = self.db()?;
        db.batch_execute(&format!(
            "create extension if not exists pg_tokenizer cascade;
             create extension if not exists vchord_bm25 cascade;
             set search_path to public, tokenizer_catalog, bm25_catalog;
             do $$ begin
               perform create_tokenizer('{tokenizer}', 'model = \"{model}\"');
             exception when others then
               if sqlerrm not like '%exist%' then raise; end if;
             end $$;
             alter table chunks add column if not exists bm25 bm25vector;
             create index if not exists chunks_bm25 on chunks using bm25 (bm25 bm25_ops);"
        ))
        .map_err(pg_err)?;
        if meta(&mut db, "bm25_tokenizer")?.as_deref() != Some(tokenizer.as_str()) {
            db.execute("update chunks set bm25 = null", &[]).map_err(pg_err)?;
            db.execute(
                "insert into meta values ('bm25_tokenizer', $1) on conflict (key) do update set value = excluded.value",
                &[&tokenizer],
            )
            .map_err(pg_err)?;
        }
        db.execute(&format!("update chunks set bm25 = tokenize(text, '{tokenizer}') where bm25 is null"), &[]).map_err(pg_err)?;
        Ok(tokenizer)
    }

    fn vector_list(&self, db: &mut postgres::Client, v: &[f32], n: i64) -> Result<Vec<Candidate>> {
        let cast = dim(db)?.map_or("vector".to_string(), |d| format!("vector({d})"));
        let rows = db
            .query(
                &format!(
                    "select id, source, page, text, (1 - (embedding::{cast} <=> $1::text::{cast}))::float8, prov from chunks
                     where embedding is not null order by embedding::{cast} <=> $1::text::{cast} limit $2"
                ),
                &[&vector_literal(v), &n],
            )
            .map_err(pg_err)?;
        Ok(rows.iter().map(candidate).collect())
    }

    fn keyword_list(&self, db: &mut postgres::Client, query: &str, n: i64, cfg: &SearchConfig) -> Result<Vec<Candidate>> {
        let rows = match (cfg.keyword_engine, self.bm25_tokenizer()) {
            (KeywordEngine::Auto | KeywordEngine::Bm25, Some(tok)) => db.query(
                &format!(
                    "select id, source, page, text, (-(bm25 <&> to_bm25query('chunks_bm25', tokenize($1, '{tok}'))))::float8 as score, prov
                     from chunks where bm25 is not null
                     order by bm25 <&> to_bm25query('chunks_bm25', tokenize($1, '{tok}')) limit $2"
                ),
                &[&query, &n],
            ),
            _ => {
                // The stored tsv is English; another language is computed per row.
                let lang = &cfg.fulltext_language;
                let doc = if lang == "english" { "tsv".to_string() } else { format!("to_tsvector('{lang}', text)") };
                db.query(
                    &format!(
                        "select id, source, page, text, ts_rank_cd({doc}, q)::float8 as score, prov
                         from chunks, websearch_to_tsquery('{lang}', $1) q where {doc} @@ q order by score desc limit $2"
                    ),
                    &[&query, &n],
                )
            }
        }
        .map_err(pg_err)?;
        Ok(rows.iter().map(candidate).filter(|c| c.hit.score > 0.0).collect())
    }
}

/// Schema changes in order; `meta.schema` records how many have run. Each runs in one
/// transaction with its version bump, so a crash never leaves half a migration.
const MIGRATIONS: [&str; 2] = [
    "create extension if not exists vector;
     create table if not exists chunks (id bigserial primary key, source text not null, page integer,
        text text not null, tsv tsvector generated always as (to_tsvector('english', text)) stored,
        embedding vector);
     create index if not exists chunks_source on chunks (source);
     create index if not exists chunks_tsv on chunks using gin (tsv);
     create table if not exists meta (key text primary key, value text);",
    // 2: provenance (JSON) for every chunk.
    "alter table chunks add column if not exists prov text;",
];

/// Bring the schema up to date, or refuse a database from a newer app version.
fn migrate(db: &mut postgres::Client) -> Result<u32> {
    db.batch_execute("create table if not exists meta (key text primary key, value text)").map_err(pg_err)?;
    let have: u32 = meta(db, "schema")?.and_then(|v| v.parse().ok()).unwrap_or(0);
    let want = MIGRATIONS.len() as u32;
    if have > want {
        return Err(Error::Store(format!(
            "the database was set up by a newer version of this app (schema {have}, this one knows {want}); update the app"
        )));
    }
    let existing: bool = db.query_one("select to_regclass('chunks') is not null", &[]).map_err(pg_err)?.get(0);
    if existing && have < want {
        // Keep the rows as they were, in the same database, until the next migration.
        db.batch_execute("drop table if exists chunks_before_migration; create table chunks_before_migration as table chunks")
            .map_err(pg_err)?;
        log::info!("kept a copy of the chunks table (schema {have}) as chunks_before_migration");
    }
    for (n, sql) in MIGRATIONS.iter().enumerate().skip(have as usize) {
        let mut tx = db.transaction().map_err(pg_err)?;
        tx.batch_execute(sql).map_err(pg_err)?;
        tx.execute(
            "insert into meta values ('schema', $1) on conflict (key) do update set value = excluded.value",
            &[&(n + 1).to_string()],
        )
        .map_err(pg_err)?;
        tx.commit().map_err(pg_err)?;
        log::info!("database schema {} applied", n + 1);
    }
    Ok(want)
}

fn prov(r: &postgres::Row, i: usize) -> Option<crate::chunk::Provenance> {
    r.get::<_, Option<String>>(i).and_then(|s| serde_json::from_str(&s).ok())
}

fn candidate(r: &postgres::Row) -> Candidate {
    Candidate {
        key: r.get::<_, i64>(0) as u64,
        hit: Hit {
            source: r.get(1),
            page: r.get::<_, Option<i32>>(2).map(|p| p as u32),
            text: r.get(3),
            score: r.get::<_, f64>(4) as f32,
            vector_score: None,
            keyword_score: None,
            prov: prov(r, 5),
        },
    }
}

impl Store for PostgresStore {
    fn kind(&self) -> &'static str {
        "postgres"
    }

    fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    fn add(&self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<usize> {
        let tokenizer = self.bm25_tokenizer();
        let mut db = self.db()?;
        let mut tx = db.transaction().map_err(pg_err)?;
        let sql = match &tokenizer {
            Some(t) => format!("insert into chunks (source, page, text, embedding, prov, bm25) values ($1, $2, $3, $4::text::vector, $5, tokenize($3, '{t}'))"),
            None => "insert into chunks (source, page, text, embedding, prov) values ($1, $2, $3, $4::text::vector, $5)".to_string(),
        };
        let stmt = tx.prepare(&sql).map_err(pg_err)?;
        for (i, c) in chunks.iter().enumerate() {
            let page = c.page.map(|p| p as i32);
            let vec = vectors.map(|v| vector_literal(&v[i]));
            let prov = c.prov.as_ref().and_then(|p| serde_json::to_string(p).ok());
            tx.execute(&stmt, &[&c.source, &page, &c.text, &vec, &prov]).map_err(pg_err)?;
        }
        if let Some(d) = vectors.and_then(|v| v.first()).map(Vec::len) {
            // The column takes any dimension; the index is on a typed cast, built once per model.
            tx.batch_execute(&format!(
                "insert into meta values ('dim', '{d}') on conflict (key) do nothing;
                 create index if not exists chunks_hnsw on chunks using hnsw ((embedding::vector({d})) vector_cosine_ops);"
            ))
            .map_err(pg_err)?;
        }
        tx.commit().map_err(pg_err)?;
        Ok(chunks.len())
    }

    fn search(&self, query: &str, query_vector: Option<&[f32]>, cfg: &SearchConfig) -> Result<Vec<Hit>> {
        let plan = plan(cfg.mode, query_vector.is_some());
        let n = if plan == Plan::Hybrid { cfg.candidates } else { cfg.top_k } as i64;
        let mut db = self.db()?;
        let hits = |c: Vec<Candidate>| c.into_iter().map(|c| c.hit).collect::<Vec<_>>();
        Ok(match (plan, query_vector) {
            (Plan::Vector, Some(v)) => fuse::single(hits(self.vector_list(&mut db, v, n)?), cfg.min_vector_score, cfg),
            (Plan::Hybrid, Some(v)) => {
                let vector = self.vector_list(&mut db, v, n)?;
                let keyword = self.keyword_list(&mut db, query, n, cfg)?;
                fuse::rrf(vector, keyword, cfg)
            }
            _ => fuse::single(hits(self.keyword_list(&mut db, query, n, cfg)?), cfg.min_keyword_score, cfg),
        })
    }

    /// BM25 is set up the first time keyword search may use it (feature `bm25`, and the
    /// extensions present on the server); otherwise keyword search stays on full text.
    fn configure(&self, config: &Config) -> Result<()> {
        let s = &config.search;
        let wants = cfg!(feature = "bm25") && s.keyword_engine != KeywordEngine::Fulltext;
        let tokenizer = if wants && bm25_installable(&mut *self.db()?)? {
            Some(self.setup_bm25(&s.bm25_tokenizer)?)
        } else if s.keyword_engine == KeywordEngine::Bm25 {
            return Err(Error::Store("BM25 search needs the vchord_bm25 and pg_tokenizer extensions on this server".into()));
        } else {
            None
        };
        if tokenizer.is_some() {
            self.db()?.batch_execute("set search_path to public, tokenizer_catalog, bm25_catalog").map_err(pg_err)?;
        }
        *self.bm25.lock().map_err(|_| Error::Store("lock poisoned".into()))? = tokenizer;
        Ok(())
    }

    fn count(&self) -> Result<usize> {
        Ok(self.db()?.query_one("select count(*) from chunks", &[]).map_err(pg_err)?.get::<_, i64>(0) as usize)
    }

    fn all(&self) -> Result<Vec<Chunk>> {
        let rows = self.db()?.query("select source, page, text, prov from chunks order by id", &[]).map_err(pg_err)?;
        Ok(rows
            .iter()
            .map(|r| Chunk { source: r.get(0), page: r.get::<_, Option<i32>>(1).map(|p| p as u32), text: r.get(2), prov: prov(r, 3) })
            .collect())
    }

    fn remove_source(&self, source: &str) -> Result<usize> {
        Ok(self.db()?.execute("delete from chunks where source = $1", &[&source]).map_err(pg_err)? as usize)
    }

    fn clear(&self) -> Result<()> {
        self.db()?.batch_execute("truncate chunks").map_err(pg_err)
    }

    fn embedder(&self) -> Result<Option<String>> {
        meta(&mut *self.db()?, "embedder")
    }

    /// A new model means a new dimension: the HNSW index is rebuilt for it.
    fn set_embedder(&self, name: Option<&str>) -> Result<()> {
        let mut db = self.db()?;
        db.execute("insert into meta values ('embedder', $1) on conflict (key) do update set value = excluded.value", &[&name])
            .map_err(pg_err)?;
        db.batch_execute("drop index if exists chunks_hnsw; delete from meta where key = 'dim';").map_err(pg_err)?;
        Ok(())
    }
}

/// `canonicalize` on Windows returns a verbatim `\\?\C:\...` path. postgresql_embedded turns it
/// into `//?/C:/...`, which Windows can't resolve, so initdb can't find postgres.exe. Strip the
/// prefix for ordinary drive paths.
fn plain_path(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\").filter(|r| !r.starts_with("UNC\\")) {
            return std::path::PathBuf::from(rest.to_string());
        }
    }
    path
}

#[cfg(test)]
mod server_tests {
    use super::*;

    #[test]
    fn every_supported_target_is_pinned() {
        for target in ["aarch64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"] {
            let h = server_sha256(target).unwrap();
            assert_eq!(parse_sha256_file(h), Some(h));
        }
        assert_eq!(server_sha256("riscv64gc-unknown-linux-gnu"), None);
    }

    #[test]
    fn asset_name_matches_the_release() {
        assert_eq!(server_asset(PG_VERSION, "x86_64-unknown-linux-gnu"), format!("postgresql-{PG_VERSION}-x86_64-unknown-linux-gnu.tar.gz"));
    }

    #[test]
    fn sha256_file_is_parsed() {
        let h = "f668fbc685cdcb71d6a82f57035bed7d97b059e2d8a1d3adf317ec36c5e861d2";
        assert_eq!(parse_sha256_file(&format!("{h}  postgresql.tar.gz\n")), Some(h));
        assert_eq!(parse_sha256_file("not a hash  file"), None);
        assert_eq!(parse_sha256_file(""), None);
    }
}

#[cfg(test)]
mod path_tests {
    #[test]
    fn plain_path_has_no_verbatim_prefix() {
        let dir = std::env::temp_dir();
        let path = super::plain_path(std::fs::canonicalize(&dir).unwrap());
        assert!(!path.to_string_lossy().starts_with(r"\\?\"), "{}", path.display());
        assert!(path.join("x").parent().unwrap().is_dir());
    }
}

#[cfg(test)]
mod unzip_tests {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn bundle(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn keeps_only_lib_and_share_extension() {
        let dir = tempfile::tempdir().unwrap();
        let zip = bundle(&[("lib/vector.so", b"so"), ("share/extension/vector.control", b"ctl"), ("bin/evil", b"x")]);
        super::unzip_into(&zip, dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("lib/vector.so")).unwrap(), b"so");
        assert_eq!(std::fs::read(dir.path().join("share/extension/vector.control")).unwrap(), b"ctl");
        assert!(!dir.path().join("bin").exists());
    }

    #[test]
    fn refuses_more_than_the_unpacked_limit() {
        // A highly compressed entry (a zip bomb) is cut off at the limit, whatever its header says.
        let dir = tempfile::tempdir().unwrap();
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("lib/bomb.so", opts).unwrap();
        w.write_all(&vec![0u8; 1 << 20]).unwrap();
        let bytes = w.finish().unwrap().into_inner();
        assert!(bytes.len() < 10_000);
        let e = super::unzip_limited(&bytes, dir.path(), 10, 64 * 1024).unwrap_err();
        assert!(e.to_string().contains("size limit"), "{e}");
        assert!(!dir.path().join("lib/bomb.so").exists());
        let many = bundle(&[("lib/a", b"1"), ("lib/b", b"2")]);
        assert!(super::unzip_limited(&many, dir.path(), 1, 1024).is_err());
        assert!(super::unzip_limited(&many, dir.path(), 2, 1024).is_ok());
    }

    #[test]
    fn garbage_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        for bytes in [&b""[..], b"PK\x03\x04", b"PK\x05\x06\0\0\0\0\xff\xff\xff\xff\0\0\0\0\0\0\0\0\0\0"] {
            assert!(super::unzip_into(bytes, dir.path()).is_err());
        }
    }

    #[test]
    fn refuses_paths_that_climb_out() {
        let dir = tempfile::tempdir().unwrap();
        let zip = bundle(&[("lib/../../escape.so", b"x")]);
        assert!(super::unzip_into(&zip, dir.path()).is_err());
        assert!(!dir.path().parent().unwrap().join("escape.so").exists());
    }
}

/// The real upgrade, end to end: a Postgres 15 cluster (pinned download) with rows in it is
/// upgraded to the bundled major by `server_url_with` in `auto` mode. It downloads two
/// servers, and initdb and pg_upgrade refuse to run as root, so it is ignored by default:
/// `APP_PG_UPGRADE_E2E=1 cargo test -p desktop-core --features postgres -- --ignored upgrade_e2e`
/// as a normal user. Without `APP_PG_EXT_DIR` it installs an empty extension bundle (the
/// cluster has no extensions), so it tests the upgrade itself.
#[cfg(test)]
mod upgrade_e2e {
    use super::*;
    use crate::pgupgrade::{exec_command, Cmd};

    fn run(program: PathBuf, args: &[&str], root: &Path, password: &str) -> String {
        let cmd = Cmd {
            program,
            args: args.iter().map(Into::into).collect(),
            env: vec![("PGPASSWORD".into(), password.into())],
            cwd: root.to_path_buf(),
        };
        exec_command(&cmd).unwrap_or_else(|e| panic!("{e}"))
    }

    fn copy_all(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.path().is_dir() {
                copy_all(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "downloads Postgres 15 and 16 and must not run as root; set APP_PG_UPGRADE_E2E=1"]
    fn upgrade_e2e() {
        if std::env::var("APP_PG_UPGRADE_E2E").map_or(true, |v| v != "1") {
            eprintln!("skipped: set APP_PG_UPGRADE_E2E=1");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        if std::env::var_os("APP_PG_EXT_DIR").is_none() {
            let ext = tmp.path().join("ext");
            std::fs::create_dir_all(ext.join("lib")).unwrap();
            std::fs::create_dir_all(ext.join("share/extension")).unwrap();
            std::env::set_var("APP_PG_EXT_DIR", &ext);
        }
        let root = plain_path(std::fs::canonicalize(tmp.path()).unwrap()).join("pg");
        make_private_dir(&root).unwrap();
        let layout = Layout { root: root.clone() };
        let old = pgupgrade::old_server(15).unwrap().version;
        if let Some(cache) = std::env::var_os("APP_PG_UPGRADE_E2E_CACHE") {
            // Servers unpacked beforehand (`<cache>/15.20.0/bin/...`, `<cache>/16.16.0/...`),
            // for machines where this user cannot download.
            for v in [old, PG_VERSION] {
                copy_all(&Path::new(&cache).join(v), &layout.install().join(v));
            }
        } else {
            fetch_old(&layout, old).unwrap();
        }
        let bin = layout.install().join(old).join("bin");
        let pw = password(&root).unwrap();
        std::fs::write(root.join(".pgpass"), &pw).unwrap();
        let data = layout.data();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        run(bin.join("initdb"), &["-D", &s(&data), "-U", "postgres", "-A", "password", "--pwfile", &s(&root.join(".pgpass")), "-E", "UTF8"], &root, &pw);
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port().to_string();
        let opts = format!("-c listen_addresses=localhost -p {port}");
        run(bin.join("pg_ctl"), &["start", "-w", "-D", &s(&data), "-l", &s(&root.join("old.log")), "-o", &opts], &root, &pw);
        let psql = |db: &str, sql: &str| run(bin.join("psql"), &["-h", "localhost", "-p", &port, "-U", "postgres", "-d", db, "-At", "-c", sql], &root, &pw);
        psql("postgres", "create database app");
        psql("app", "create table chunks (id bigserial primary key, text text); insert into chunks (text) select 'row ' || g from generate_series(1, 500) g");
        run(bin.join("pg_ctl"), &["stop", "-w", "-m", "fast", "-D", &s(&data)], &root, &pw);
        assert_eq!(pgupgrade::data_major(&data).unwrap(), Some(15));

        // `ask` stops with the prompt's error and changes nothing.
        let ask = server_url_with(&root, &PostgresConfig::default(), &mut |_| {}).unwrap_err().to_string();
        assert!(ask.contains(UPGRADE_NEEDED), "{ask}");
        // A check passes and changes nothing.
        let mut lines = Vec::new();
        let checked = upgrade_now(&root, &PostgresConfig::default(), true, &mut |l| lines.push(l.to_string())).unwrap();
        assert!(checked.contains("--check passed"), "{checked}");
        assert_eq!(pgupgrade::data_major(&data).unwrap(), Some(15));

        let cfg = PostgresConfig { upgrade: UpgradeMode::Auto, ..PostgresConfig::default() };
        let (url, notice) = server_url_with(&root, &cfg, &mut |l| lines.push(l.to_string())).unwrap();
        assert_eq!(notice, None, "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("Upgrading database")));
        let mut db = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        let version: String = db.query_one("show server_version_num", &[]).unwrap().get(0);
        assert_eq!(version[..2].parse::<u32>().unwrap(), bundled_major());
        let n: i64 = db.query_one("select count(*) from chunks", &[]).unwrap().get(0);
        assert_eq!(n, 500);
        assert!(!layout.backup(15).exists(), "the old folder goes once the new one passed");
        assert_eq!(pgupgrade::last_failure(&layout), None);
        drop(db);
        shutdown();
    }
}
