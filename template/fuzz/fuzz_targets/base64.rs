//! `audio::base64_decode`: base64 audio from the web view (and JWT parts in `oauth`).
#![no_main]

use desktop_core::audio::{base64, base64_decode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    if let Ok(bytes) = base64_decode(text) {
        assert!(bytes.len() <= text.len());
        // Encoding what was decoded must decode to the same bytes.
        assert_eq!(base64_decode(&base64(&bytes)).unwrap(), bytes);
    }
    // And every byte string survives the round trip.
    assert_eq!(base64_decode(&base64(data)).unwrap(), data);
});
