//! Caches{% if cache %}: the app's general in-memory cache ([`memory`], feature `cache`){% endif %}{% if ai and cache %} and{% endif %}{% if ai %} the
//! semantic answer cache of `Engine::ask` ([`semantic`], feature `ai`){% endif %}.
//!
//! They answer different questions, so neither sits on the other:
{% if cache %}//! - [`memory::Cache`] is a bounded, in-memory map for any value the app can rebuild (W-TinyLFU
//!   eviction, TTL and time-to-idle, weights, hit and miss counts, single-flight loading),
//!   optionally backed by the key-value store (feature `kv`) so entries survive a restart;
{% endif %}{% if ai %}//! - [`semantic::SemanticCache`] keeps LLM answers on disk and finds them by the closest
//!   question embedding (a nearest-neighbour scan, not a key lookup), so it keeps its own file.
{% endif %}{% if cache %}//!
//! Web-dev mapping: an in-process Redis (or `lru-cache` with a TTL) is [`memory::Cache`];
//! a persistent `localStorage` is the key-value store, `desktop_core::kv` (feature `kv`).
{% endif %}{% if ai %}
#[cfg(feature = "ai")]
pub mod semantic;
#[cfg(feature = "ai")]
pub use semantic::{normalise_question, CacheConfig, SemanticCache};
{% endif %}{% if cache %}
#[cfg(feature = "cache")]
pub mod memory;
#[cfg(feature = "cache")]
pub use memory::{AppCache, Backing, Cache, CacheOptions, CacheStats, JsonCache, MemoryCacheConfig};
{% endif %}
