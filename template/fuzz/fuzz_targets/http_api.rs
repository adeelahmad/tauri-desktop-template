//! The local REST API's request bodies and query strings, parsed and validated as the
//! handlers do (`ValidJson`, `ValidQuery`).
#![no_main]

use desktop_core::http_api::pagination::MAX_ITEMS;
use desktop_core::http_api::routes::{DocumentRequest, SearchRequest, SourceQuery};
use desktop_core::http_api::{Page, Validate};
use libfuzzer_sys::fuzz_target;

fn check<T: serde::de::DeserializeOwned + Validate>(data: &[u8]) -> Option<T> {
    let v: T = serde_json::from_slice(data).ok()?;
    v.validate().ok().map(|_| v)
}

fuzz_target!(|data: &[u8]| {
    if let Some(req) = check::<SearchRequest>(data) {
        assert!(req.page.fetch() <= MAX_ITEMS);
        let _ = req.page.paginate((0..req.page.fetch()).collect::<Vec<_>>());
    }
    let _ = check::<DocumentRequest>(data);
    let _ = check::<SourceQuery>(data);
    let _ = check::<desktop_core::Config>(data);
    if let Ok(page) = serde_json::from_slice::<Page>(data) {
        let _ = (page.check(), page.offset(), page.end(), page.fetch());
    }
});
