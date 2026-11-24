//! A bounded in-memory cache (feature `cache`), for anything the app can rebuild: results of
//! slow calls, parsed files, remote lookups.
//!
//! [`Cache`] wraps [moka](https://docs.rs/moka)'s concurrent cache (W-TinyLFU admission and
//! eviction, as in Caffeine) and adds:
//! - a time to live and a time to idle ([`CacheOptions`]);
//! - a size cap by weight (bytes for [`JsonCache`], or your own weigher);
//! - hit and miss counts ([`Cache::stats`]);
//! - single-flight loading: [`Cache::get_or_insert_with`] runs the loader once per key even
//!   when many threads ask at the same time, and they all get its value;
//! - an optional [`Backing`] store, written through and read on a miss, so entries survive a
//!   restart{% if kv %} (a `desktop_core::kv` namespace is one, with feature `kv`; `cache.persist` in
//!   the settings uses namespace `cache`){% endif %}.
//!
//! [`AppCache`] is the app's own [`JsonCache`], sized by `cache` in the settings; Settings
//! shows its stats and can clear it (`Engine::cache`).
//!
//! # Example
//!
//! ```
//! use desktop_core::cache::{Cache, CacheOptions};
//! use std::time::Duration;
//!
//! let cache: Cache<String, String> = Cache::new(CacheOptions { max_weight: 1000, ttl: Some(Duration::from_secs(60)), tti: None });
//! let v = cache.get_or_insert_with("user:1".into(), || "Ann".to_string()); // loads: a miss
//! assert_eq!(v, "Ann");
//! assert_eq!(cache.get(&"user:1".into()).as_deref(), Some("Ann")); // a hit
//! let s = cache.stats();
//! assert_eq!((s.hits, s.misses, s.entries), (1, 1, 1));
//! cache.clear();
//! assert_eq!(cache.get(&"user:1".into()), None);
//! ```

use serde::{Deserialize, Serialize};
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A store behind a [`Cache`]: written through on insert, read on a miss. Failures are the
/// backing's to log; the cache works without it.
pub trait Backing<K, V>: Send + Sync {
    /// The stored value for `key`, if any.
    fn load(&self, key: &K) -> Option<V>;
    /// Keep `value` for `key`, for `ttl` when given.
    fn store(&self, key: &K, value: &V, ttl: Option<Duration>);
    /// Forget `key`.
    fn remove(&self, key: &K);
    /// Forget everything.
    fn clear(&self);
}

/// How big a [`Cache`] may get and how long entries live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheOptions {
    /// The most total weight kept (entries weigh 1 each unless a weigher says otherwise).
    pub max_weight: u64,
    /// An entry goes this long after it was written.
    pub ttl: Option<Duration>,
    /// An entry goes this long after it was last read or written.
    pub tti: Option<Duration>,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self { max_weight: 10_000, ttl: None, tti: None }
    }
}

/// Counts for Settings and logs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    /// Lookups answered from memory or the backing store.
    pub hits: u64,
    /// Lookups that found nothing (or ran the loader).
    pub misses: u64,
    /// Of the hits, those answered by the backing store.
    pub backing_hits: u64,
    /// Entries now in memory (approximate while evictions are pending).
    pub entries: u64,
    /// Their total weight.
    pub weight: u64,
    /// The cap on the weight.
    pub max_weight: u64,
    /// `hits / (hits + misses)`, 0 before any lookup.
    pub hit_rate: f64,
}

/// A concurrent, bounded in-memory cache. Share it with `Arc`; every method takes `&self`.
pub struct Cache<K, V> {
    inner: moka::sync::Cache<K, V>,
    opts: CacheOptions,
    hits: AtomicU64,
    misses: AtomicU64,
    backing_hits: AtomicU64,
    backing: Option<Arc<dyn Backing<K, V>>>,
}

type Weigher<K, V> = Box<dyn Fn(&K, &V) -> u32 + Send + Sync + 'static>;

impl<K, V> Cache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// A cache where every entry weighs 1, so `max_weight` is the most entries.
    pub fn new(opts: CacheOptions) -> Self {
        Self::build(opts, None)
    }

    /// A cache where `weigher` says what each entry weighs (bytes, say).
    pub fn weighted(opts: CacheOptions, weigher: impl Fn(&K, &V) -> u32 + Send + Sync + 'static) -> Self {
        Self::build(opts, Some(Box::new(weigher)))
    }

    fn build(opts: CacheOptions, weigher: Option<Weigher<K, V>>) -> Self {
        let mut b = moka::sync::Cache::builder().max_capacity(opts.max_weight);
        if let Some(w) = weigher {
            b = b.weigher(move |k: &K, v: &V| w(k, v));
        }
        if let Some(ttl) = opts.ttl {
            b = b.time_to_live(ttl);
        }
        if let Some(tti) = opts.tti {
            b = b.time_to_idle(tti);
        }
        Self { inner: b.build(), opts, hits: AtomicU64::new(0), misses: AtomicU64::new(0), backing_hits: AtomicU64::new(0), backing: None }
    }

    /// Write entries through to `backing` and read it on a miss.
    pub fn with_backing(mut self, backing: Arc<dyn Backing<K, V>>) -> Self {
        self.backing = Some(backing);
        self
    }

    /// The options it was made with.
    pub fn options(&self) -> CacheOptions {
        self.opts
    }

    fn count(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// The value for `key`, from memory or the backing store.
    pub fn get(&self, key: &K) -> Option<V> {
        if let Some(v) = self.inner.get(key) {
            self.count(&self.hits);
            return Some(v);
        }
        if let Some(v) = self.backing.as_ref().and_then(|b| b.load(key)) {
            self.count(&self.hits);
            self.count(&self.backing_hits);
            self.inner.insert(key.clone(), v.clone());
            return Some(v);
        }
        self.count(&self.misses);
        None
    }

    /// Keep `value` for `key`.
    pub fn insert(&self, key: K, value: V) {
        if let Some(b) = &self.backing {
            b.store(&key, &value, self.opts.ttl);
        }
        self.inner.insert(key, value);
    }

    /// Forget `key`.
    pub fn remove(&self, key: &K) {
        if let Some(b) = &self.backing {
            b.remove(key);
        }
        self.inner.invalidate(key);
    }

    /// The value for `key`; on a miss, `init` makes it. Single-flight: while one caller runs
    /// `init` for a key, others asking for the same key wait and get its value.
    pub fn get_or_insert_with(&self, key: K, init: impl FnOnce() -> V) -> V {
        let mut from_backing = false;
        let entry = self.inner.entry(key.clone()).or_insert_with(|| {
            if let Some(v) = self.backing.as_ref().and_then(|b| b.load(&key)) {
                from_backing = true;
                return v;
            }
            let v = init();
            if let Some(b) = &self.backing {
                b.store(&key, &v, self.opts.ttl);
            }
            v
        });
        self.tally(entry.is_fresh(), from_backing);
        entry.into_value()
    }

    /// [`Cache::get_or_insert_with`] with a loader that can fail; a failure is not cached,
    /// and every caller waiting on it gets the same error.
    pub fn try_get_or_insert_with<E>(&self, key: K, init: impl FnOnce() -> Result<V, E>) -> Result<V, Arc<E>>
    where
        E: Send + Sync + 'static,
    {
        let mut from_backing = false;
        let entry = self.inner.entry(key.clone()).or_try_insert_with(|| {
            if let Some(v) = self.backing.as_ref().and_then(|b| b.load(&key)) {
                from_backing = true;
                return Ok(v);
            }
            let v = init()?;
            if let Some(b) = &self.backing {
                b.store(&key, &v, self.opts.ttl);
            }
            Ok(v)
        })?;
        self.tally(entry.is_fresh(), from_backing);
        Ok(entry.into_value())
    }

    fn tally(&self, fresh: bool, from_backing: bool) {
        if !fresh {
            self.count(&self.hits);
        } else if from_backing {
            self.count(&self.hits);
            self.count(&self.backing_hits);
        } else {
            self.count(&self.misses);
        }
    }

    /// Forget everything (in memory and in the backing store); the counts start again.
    pub fn clear(&self) {
        if let Some(b) = &self.backing {
            b.clear();
        }
        self.inner.invalidate_all();
        self.inner.run_pending_tasks();
        for c in [&self.hits, &self.misses, &self.backing_hits] {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// Run evictions and expiries that are pending now (they otherwise run a little later).
    pub fn run_pending_tasks(&self) {
        self.inner.run_pending_tasks();
    }

    /// Hits, misses, entries and weight.
    pub fn stats(&self) -> CacheStats {
        self.inner.run_pending_tasks();
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let lookups = hits + misses;
        CacheStats {
            hits,
            misses,
            backing_hits: self.backing_hits.load(Ordering::Relaxed),
            entries: self.inner.entry_count(),
            weight: self.inner.weighted_size(),
            max_weight: self.opts.max_weight,
            #[allow(clippy::cast_precision_loss)]
            hit_rate: if lookups == 0 { 0.0 } else { hits as f64 / lookups as f64 },
        }
    }
}

/// A cache of JSON values by string key, weighed in (approximate) bytes.
pub type JsonCache = Cache<String, serde_json::Value>;

/// About how many bytes `v` takes as JSON text (cheap: nothing is serialised).
pub fn json_weight(v: &serde_json::Value) -> usize {
    use serde_json::Value;
    match v {
        Value::Null | Value::Bool(_) => 5,
        Value::Number(_) => 12,
        Value::String(s) => s.len() + 2,
        Value::Array(a) => 2 + a.iter().map(|x| json_weight(x) + 1).sum::<usize>(),
        Value::Object(o) => 2 + o.iter().map(|(k, x)| k.len() + 4 + json_weight(x)).sum::<usize>(),
    }
}

impl JsonCache {
    /// A JSON cache whose `max_weight` is in bytes.
    pub fn json(opts: CacheOptions) -> Self {
        Self::weighted(opts, |k: &String, v: &serde_json::Value| u32::try_from(k.len() + json_weight(v)).unwrap_or(u32::MAX))
    }
}

/// `cache` in the config: the app's own cache ([`AppCache`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct MemoryCacheConfig {
    /// The most memory the cache may hold, in MB (1 to 4096).
    pub max_mb: u64,
    /// Entries go this long after they were written, in seconds (0: never).
    pub ttl_secs: u64,
    /// Entries go this long after they were last used, in seconds (0: never).
    pub tti_secs: u64,
    /// Also keep entries in the key-value store (namespace `cache`, needs feature `kv`) so
    /// they survive a restart.
    pub persist: bool,
}

impl Default for MemoryCacheConfig {
    fn default() -> Self {
        Self { max_mb: 64, ttl_secs: 3600, tti_secs: 0, persist: false }
    }
}

impl MemoryCacheConfig {
    /// Numbers clamped into range.
    pub fn validated(mut self) -> Self {
        self.max_mb = self.max_mb.clamp(1, 4096);
        self.ttl_secs = self.ttl_secs.min(365 * 24 * 3600);
        self.tti_secs = self.tti_secs.min(365 * 24 * 3600);
        self
    }

    /// The cache options for these settings.
    pub fn options(&self) -> CacheOptions {
        let secs = |s: u64| (s > 0).then(|| Duration::from_secs(s));
        CacheOptions { max_weight: self.max_mb * 1024 * 1024, ttl: secs(self.ttl_secs), tti: secs(self.tti_secs) }
    }
}

/// The app's [`JsonCache`], made again when its settings change.
#[derive(Default)]
pub struct AppCache {
    current: Mutex<Option<(MemoryCacheConfig, Arc<JsonCache>)>>,
}

impl AppCache {
    /// No cache yet; [`AppCache::get`] makes it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cache for `cfg` (a new, empty one when `cfg` changed). `backing` is asked for a
    /// store only when `cfg.persist` is on and a cache is made.
    pub fn get(&self, cfg: &MemoryCacheConfig, backing: impl FnOnce() -> Option<Arc<dyn Backing<String, serde_json::Value>>>) -> Arc<JsonCache> {
        let mut current = self.current.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((c, cache)) = current.as_ref() {
            if c == cfg {
                return cache.clone();
            }
        }
        let mut cache = JsonCache::json(cfg.options());
        if cfg.persist {
            if let Some(b) = backing() {
                cache = cache.with_backing(b);
            }
        }
        let cache = Arc::new(cache);
        *current = Some((cfg.clone(), cache.clone()));
        cache
    }
}
{% if kv %}
/// A key-value namespace behind a cache: values as JSON, the cache's TTL on each key.
#[cfg(feature = "kv")]
impl<V: Serialize + serde::de::DeserializeOwned> Backing<String, V> for crate::kv::Namespace {
    fn load(&self, key: &String) -> Option<V> {
        self.get_json(key).unwrap_or_else(|e| {
            log::debug!("cache backing {}: {e}", self.name());
            None
        })
    }

    fn store(&self, key: &String, value: &V, ttl: Option<Duration>) {
        let r = match ttl {
            Some(ttl) => self.set_json_ttl(key, value, ttl),
            None => self.set_json(key, value),
        };
        if let Err(e) = r {
            log::debug!("cache backing {}: {e}", self.name());
        }
    }

    fn remove(&self, key: &String) {
        let _ = self.delete(key);
    }

    fn clear(&self) {
        let r = self.keys("").and_then(|keys| {
            self.transaction(|tx| {
                for k in &keys {
                    tx.delete(k)?;
                }
                Ok(())
            })
        });
        if let Err(e) = r {
            log::warn!("cache backing {}: {e}", self.name());
        }
    }
}
{% endif %}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn evicts_down_to_the_weight_cap() {
        let c: Cache<u32, u32> = Cache::new(CacheOptions { max_weight: 10, ..CacheOptions::default() });
        for i in 0..200 {
            c.insert(i, i);
        }
        let s = c.stats();
        assert!(s.entries <= 10 && s.weight <= 10, "{s:?}");
        let w: Cache<u32, String> = Cache::weighted(CacheOptions { max_weight: 100, ..CacheOptions::default() }, |_, v: &String| u32::try_from(v.len()).unwrap());
        for i in 0..50 {
            w.insert(i, "x".repeat(30));
        }
        assert!(w.stats().weight <= 100);
    }

    #[test]
    fn the_loader_runs_once_for_many_callers() {
        let c: Arc<Cache<String, u64>> = Arc::new(Cache::new(CacheOptions::default()));
        let runs = Arc::new(AtomicU64::new(0));
        let start = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (c, runs, start) = (c.clone(), runs.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    c.get_or_insert_with("k".into(), || {
                        runs.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        42
                    })
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), 42);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let s = c.stats();
        assert_eq!((s.misses, s.hits), (1, 7));
    }

    #[test]
    fn failures_are_not_cached() {
        let c: Cache<&'static str, u8> = Cache::new(CacheOptions::default());
        let e = c.try_get_or_insert_with("k", || Err::<u8, _>("down")).unwrap_err();
        assert_eq!(*e, "down");
        assert_eq!(c.try_get_or_insert_with("k", || Ok::<_, &str>(7)).unwrap(), 7);
        assert_eq!(c.try_get_or_insert_with("k", || Err::<u8, _>("unused")).unwrap(), 7);
    }

    #[derive(Default)]
    struct MapBacking(Mutex<HashMap<String, serde_json::Value>>);

    impl Backing<String, serde_json::Value> for MapBacking {
        fn load(&self, key: &String) -> Option<serde_json::Value> {
            self.0.lock().unwrap().get(key).cloned()
        }
        fn store(&self, key: &String, value: &serde_json::Value, _: Option<Duration>) {
            self.0.lock().unwrap().insert(key.clone(), value.clone());
        }
        fn remove(&self, key: &String) {
            self.0.lock().unwrap().remove(key);
        }
        fn clear(&self) {
            self.0.lock().unwrap().clear();
        }
    }

    #[test]
    fn a_backing_store_outlives_the_cache() {
        let backing = Arc::new(MapBacking::default());
        let first = JsonCache::json(CacheOptions::default()).with_backing(backing.clone());
        first.insert("a".into(), serde_json::json!({"n": 1}));
        drop(first);
        let second = JsonCache::json(CacheOptions::default()).with_backing(backing.clone());
        assert_eq!(second.get(&"a".into()), Some(serde_json::json!({"n": 1})));
        assert_eq!(second.get_or_insert_with("a".into(), || serde_json::json!("unused")), serde_json::json!({"n": 1}));
        assert_eq!(second.stats().backing_hits, 1);
        second.clear();
        assert!(backing.0.lock().unwrap().is_empty());
        assert_eq!(second.stats(), CacheStats { max_weight: 10_000, ..CacheStats::default() });
    }

    #[test]
    fn the_app_cache_follows_its_settings() {
        let app = AppCache::new();
        let cfg = MemoryCacheConfig::default();
        let a = app.get(&cfg, || None);
        a.insert("k".into(), serde_json::json!(1));
        assert!(Arc::ptr_eq(&a, &app.get(&cfg, || None)));
        let b = app.get(&MemoryCacheConfig { max_mb: 1, ..cfg.clone() }, || None);
        assert_eq!(b.get(&"k".into()), None, "new settings, new cache");
        assert_eq!(b.options().max_weight, 1024 * 1024);
        let v = MemoryCacheConfig { max_mb: 0, ttl_secs: u64::MAX, ..cfg }.validated();
        assert_eq!((v.max_mb, v.ttl_secs), (1, 365 * 24 * 3600));
        assert!(json_weight(&serde_json::json!({"ab": "cd"})) >= 8);
    }
{% if kv %}
    #[cfg(feature = "kv")]
    #[test]
    fn a_kv_namespace_can_back_the_cache() {
        let d = tempfile::tempdir().unwrap();
        let kv = crate::kv::Kv::open(d.path().join("kv.redb")).unwrap();
        let ns: Arc<dyn Backing<String, serde_json::Value>> = Arc::new(kv.ns("cache").unwrap());
        let c = JsonCache::json(CacheOptions { ttl: Some(Duration::from_secs(60)), ..CacheOptions::default() }).with_backing(ns.clone());
        c.insert("a".into(), serde_json::json!([1, 2]));
        assert_eq!(kv.ns("cache").unwrap().get_json::<Vec<i32>>("a").unwrap(), Some(vec![1, 2]));
        let again = JsonCache::json(CacheOptions::default()).with_backing(ns);
        assert_eq!(again.get(&"a".into()), Some(serde_json::json!([1, 2])));
        again.clear();
        assert!(kv.ns("cache").unwrap().keys("").unwrap().is_empty());
    }
{% endif %}}
