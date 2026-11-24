//! `dotenv::parse`: `.env` files (development builds only, but still files from disk).
#![no_main]

use desktop_core::dotenv::{merge, parse, valid_key};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let parsed = parse(text);
    for (k, _) in &parsed.vars {
        assert!(valid_key(k), "{k:?}");
    }
    let _ = merge(&[parsed], |_| false);
});
