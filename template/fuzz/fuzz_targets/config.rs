//! `Config`: settings JSON (settings.json, the Settings screen, `PUT /v1/settings`, the worker).
#![no_main]

use desktop_core::Config;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(cfg) = serde_json::from_slice::<Config>(data) else { return };
    let cfg = cfg.validated();
    let s = &cfg.search;
    assert!((1..=1000).contains(&s.top_k) && s.candidates >= s.top_k && s.rrf_k.is_finite());
    let i = &cfg.ingest;
    assert!(i.chunk_overlap <= i.chunk_chars / 2 && (1..=4096).contains(&i.max_file_mb));
    // What is saved is read back the same.
    let again: Config = serde_json::from_slice(&serde_json::to_vec(&cfg).unwrap()).unwrap();
    assert_eq!(again.validated().search.top_k, cfg.search.top_k);
});
