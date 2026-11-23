//! Embedded Postgres 17 + pgvector.
//!
//! The server runs from a private data folder (`postgresql_embedded`), listens on
//! localhost with a random password, and installs prebuilt pgvector on first use. Vector
//! search runs in SQL on an HNSW cosine index; keyword search uses Postgres full text.
//! `APP_PG_URL` points at an existing server with pgvector instead.

use crate::store::{Hit, Store};
use crate::{Chunk, Error, Result};
use postgresql_embedded::blocking::PostgreSQL;
use postgresql_embedded::{Settings, VersionReq};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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
    // Random enough for a localhost-only server: time, pid and a stack address hashed.
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..3u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    std::fs::write(&file, &out)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(out)
}

/// The prebuilt x86-64 pgvector is compiled for AVX-512 and crashes (SIGILL) on CPUs
/// without it (many Intel Core and older AMD chips, Intel Macs). ARM is fine.
pub fn cpu_supports_bundled_pgvector() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512vl")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512dq")
            && std::arch::is_x86_feature_detected!("avx512vpopcntdq")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

/// Whether the embedded server can run here (or `APP_PG_URL` is set).
pub fn available() -> bool {
    std::env::var("APP_PG_URL").is_ok_and(|u| !u.is_empty()) || cpu_supports_bundled_pgvector()
}

/// One embedded server per data folder, started on first use; returns its URL.
pub fn server_url(data_dir: &Path) -> Result<String> {
    if let Ok(url) = std::env::var("APP_PG_URL") {
        if !url.is_empty() {
            return Ok(url);
        }
    }
    if !cpu_supports_bundled_pgvector() {
        return Err(Error::Store(
            "embedded Postgres is off on this CPU (the bundled pgvector needs AVX-512); use the file store or set APP_PG_URL".into(),
        ));
    }
    std::fs::create_dir_all(data_dir)?;
    let key = std::fs::canonicalize(data_dir)?;
    let mut servers = SERVERS.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(pg_err)?;
    if let Some((_, url)) = servers.get(&key) {
        return Ok(url.clone());
    }
    let settings = Settings {
        version: VersionReq::parse("=17").map_err(pg_err)?,
        installation_dir: key.join("install"),
        data_dir: key.join("data"),
        password_file: key.join(".pgpass"),
        password: password(&key)?,
        host: "localhost".into(),
        temporary: false,
        ..Settings::default()
    };
    let mut pg = PostgreSQL::new(settings);
    pg.setup().map_err(pg_err)?;
    let installed = postgresql_extensions::blocking::get_installed_extensions(pg.settings()).unwrap_or_default();
    if !installed.iter().any(|e| e.name().contains("pgvector")) {
        postgresql_extensions::blocking::install(pg.settings(), "portal-corp", "pgvector_compiled", &postgresql_extensions::VersionReq::STAR)
            .map_err(|e| pg_err(format!("could not install pgvector ({e})")))?;
    }
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
}

/// Vector dimension of the stored embeddings (kept in `meta`), which the HNSW index needs.
fn dim(db: &mut postgres::Client) -> Result<Option<usize>> {
    let row = db.query_opt("select value from meta where key = 'dim'", &[]).map_err(pg_err)?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>(0)).and_then(|d| d.parse().ok()))
}

fn vector_literal(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    format!("[{}]", parts.join(","))
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
        Ok(Self { client: Mutex::new(client) })
    }

    fn db(&self) -> Result<std::sync::MutexGuard<'_, postgres::Client>> {
        self.client.lock().map_err(|_| Error::Store("lock poisoned".into()))
    }
}

impl Store for PostgresStore {
    fn kind(&self) -> &'static str {
        "postgres"
    }

    fn add(&self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<usize> {
        let mut db = self.db()?;
        let mut tx = db.transaction().map_err(pg_err)?;
        let stmt = tx
            .prepare("insert into chunks (source, page, text, embedding) values ($1, $2, $3, $4::text::vector)")
            .map_err(pg_err)?;
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

    fn search(&self, query: &str, query_vector: Option<&[f32]>, k: usize) -> Result<Vec<Hit>> {
        let mut db = self.db()?;
        let k = k as i64;
        let rows = match query_vector {
            Some(v) => {
                let cast = dim(&mut db)?.map_or("vector".to_string(), |d| format!("vector({d})"));
                db.query(
                    &format!(
                        "select source, page, text, (1 - (embedding::{cast} <=> $1::text::{cast}))::float8 from chunks
                         where embedding is not null order by embedding::{cast} <=> $1::text::{cast} limit $2"
                    ),
                    &[&vector_literal(v), &k],
                )
            }
            None => db.query(
                "select source, page, text, ts_rank(tsv, q)::float8 as score from chunks, plainto_tsquery('english', $1) q
                 where tsv @@ q order by score desc limit $2",
                &[&query, &k],
            ),
        }
        .map_err(pg_err)?;
        Ok(rows
            .iter()
            .map(|r| Hit {
                source: r.get(0),
                page: r.get::<_, Option<i32>>(1).map(|p| p as u32),
                text: r.get(2),
                score: r.get::<_, f64>(3) as f32,
            })
            .collect())
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
        let row = self.db()?.query_opt("select value from meta where key = 'embedder'", &[]).map_err(pg_err)?;
        Ok(row.and_then(|r| r.get::<_, Option<String>>(0)))
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
