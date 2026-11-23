//! Embedded Postgres 16 with pgvector and, with feature `bm25`, VectorChord-BM25.
//!
//! The server runs from a private data folder (`postgresql_embedded`), listens on
//! localhost only with a random password, and on first use installs one extension bundle
//! (pgvector, pg_tokenizer, VectorChord-BM25) built for this platform by the
//! `pg-extensions` workflow. The bundle's SHA-256 is checked before it is unpacked.
//! Vector search runs on an HNSW cosine index; keyword search uses BM25 when it is on and
//! installed, else Postgres full text; hybrid mode fuses both (see `fuse`).
//!
//! - `APP_PG_URL`: use an existing server instead (it needs pgvector; BM25 is used when
//!   `vchord_bm25` and `pg_tokenizer` are installed there).
//! - `APP_PG_EXT_DIR`: install the bundle from a local folder (`lib/`, `share/extension/`)
//!   instead of downloading it, for offline installers.

use crate::config::{Config, KeywordEngine, SearchConfig};
use crate::fuse::{self, Candidate};
use crate::store::{plan, Hit, Plan, Store};
use crate::{Chunk, Error, Result};
use postgresql_embedded::blocking::PostgreSQL;
use postgresql_embedded::{Settings, VersionReq};
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

static SERVERS: OnceLock<Mutex<HashMap<PathBuf, (PostgreSQL, String)>>> = OnceLock::new();

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
    EXT_SHA256.lines().filter_map(|l| l.split_once(char::is_whitespace)).find(|(_, f)| f.trim().trim_start_matches('*') == file).map(|(h, _)| h)
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

/// Extract `lib/*` and `share/extension/*` only, refusing paths that climb out (zip slip).
fn unzip_into(bytes: &[u8], root: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(pg_err)?;
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
        std::io::copy(&mut entry, &mut f)?;
    }
    Ok(())
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

/// Whether the embedded server can run here (or `APP_PG_URL` is set).
pub fn available() -> bool {
    std::env::var("APP_PG_URL").is_ok_and(|u| !u.is_empty()) || embedded_supported()
}

/// One embedded server per data folder, started on first use; returns its URL.
pub fn server_url(data_dir: &Path) -> Result<String> {
    if let Ok(url) = std::env::var("APP_PG_URL") {
        if !url.is_empty() {
            return Ok(url);
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
    if let Some((_, url)) = servers.get(&key) {
        return Ok(url.clone());
    }
    let mut configuration = HashMap::new();
    // Local connections only, and BM25's libraries loaded at start when they are built in.
    configuration.insert("listen_addresses".to_string(), "localhost".to_string());
    if cfg!(feature = "bm25") {
        configuration.insert("shared_preload_libraries".to_string(), "pg_tokenizer,vchord_bm25".to_string());
    }
    let settings = Settings {
        version: VersionReq::parse(&format!("={PG_VERSION}")).map_err(pg_err)?,
        installation_dir: key.join("install"),
        data_dir: key.join("data"),
        password_file: key.join(".pgpass"),
        password: password(&key)?,
        host: "localhost".into(),
        temporary: false,
        configuration,
        ..Settings::default()
    };
    let mut pg = PostgreSQL::new(settings);
    pg.setup().map_err(pg_err)?;
    let root = pg.settings().binary_dir().parent().map(Path::to_path_buf).ok_or_else(|| pg_err("no install folder"))?;
    install_extensions(&root)?;
    pg.start().map_err(pg_err)?;
    if !pg.database_exists(DATABASE).map_err(pg_err)? {
        pg.create_database(DATABASE).map_err(pg_err)?;
    }
    let url = pg.settings().url(DATABASE);
    servers.insert(key, (pg, url.clone()));
    Ok(url)
}

/// Stop every server this process started (call on app exit).
pub fn shutdown() {
    if let Some(servers) = SERVERS.get() {
        if let Ok(mut servers) = servers.lock() {
            for (_, (pg, _)) in servers.drain() {
                let _ = pg.stop();
            }
        }
    }
}

pub struct PostgresStore {
    client: Mutex<postgres::Client>,
    /// The pg_tokenizer tokenizer BM25 uses, once BM25 is set up.
    bm25: Mutex<Option<String>>,
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
    pub fn open(data_dir: &Path) -> Result<Self> {
        let mut client = postgres::Client::connect(&server_url(data_dir)?, postgres::NoTls).map_err(pg_err)?;
        client
            .batch_execute(
                "create extension if not exists vector;
                 create table if not exists chunks (id bigserial primary key, source text not null, page integer,
                    text text not null, tsv tsvector generated always as (to_tsvector('english', text)) stored,
                    embedding vector);
                 create index if not exists chunks_source on chunks (source);
                 create index if not exists chunks_tsv on chunks using gin (tsv);
                 create table if not exists meta (key text primary key, value text);",
            )
            .map_err(pg_err)?;
        Ok(Self { client: Mutex::new(client), bm25: Mutex::new(None) })
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
            db.execute("insert into meta values ('bm25_tokenizer', $1) on conflict (key) do update set value = excluded.value", &[&tokenizer])
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
                    "select id, source, page, text, (1 - (embedding::{cast} <=> $1::text::{cast}))::float8 from chunks
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
                    "select id, source, page, text, (-(bm25 <&> to_bm25query('chunks_bm25', tokenize($1, '{tok}'))))::float8 as score
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
                        "select id, source, page, text, ts_rank_cd({doc}, q)::float8 as score
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
        },
    }
}

impl Store for PostgresStore {
    fn kind(&self) -> &'static str {
        "postgres"
    }

    fn add(&self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<usize> {
        let tokenizer = self.bm25_tokenizer();
        let mut db = self.db()?;
        let mut tx = db.transaction().map_err(pg_err)?;
        let sql = match &tokenizer {
            Some(t) => format!("insert into chunks (source, page, text, embedding, bm25) values ($1, $2, $3, $4::text::vector, tokenize($3, '{t}'))"),
            None => "insert into chunks (source, page, text, embedding) values ($1, $2, $3, $4::text::vector)".to_string(),
        };
        let stmt = tx.prepare(&sql).map_err(pg_err)?;
        for (i, c) in chunks.iter().enumerate() {
            let page = c.page.map(|p| p as i32);
            let vec = vectors.map(|v| vector_literal(&v[i]));
            tx.execute(&stmt, &[&c.source, &page, &c.text, &vec]).map_err(pg_err)?;
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
        let rows = self.db()?.query("select source, page, text from chunks order by id", &[]).map_err(pg_err)?;
        Ok(rows.iter().map(|r| Chunk { source: r.get(0), page: r.get::<_, Option<i32>>(1).map(|p| p as u32), text: r.get(2) }).collect())
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
mod path_tests {
    #[test]
    fn plain_path_has_no_verbatim_prefix() {
        let dir = std::env::temp_dir();
        let path = super::plain_path(std::fs::canonicalize(&dir).unwrap());
        assert!(!path.to_string_lossy().starts_with(r"\\?\"), "{}", path.display());
        assert!(path.join("x").parent().unwrap().is_dir());
    }
}
