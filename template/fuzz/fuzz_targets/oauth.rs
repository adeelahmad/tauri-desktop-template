//! OAuth: the redirect the browser comes back with, and the provider's ID token.
#![no_main]

use desktop_core::oauth::{callback_state, parse_callback, verify_id_token, Expected};
use libfuzzer_sys::fuzz_target;
use serde_json::json;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let _ = callback_state(text);
    let _ = parse_callback(text, "state");
    let expected = Expected { issuer: "https://issuer.example", client_id: "app", nonce: Some("n") };
    let jwks = json!({"keys": [
        {"kty": "RSA", "kid": "r", "n": "AQAB", "e": "AQAB"},
        {"kty": "EC", "kid": "e", "crv": "P-256", "x": "AQAB", "y": "AQAB"}
    ]});
    // Never accepted: no key here can have signed it.
    assert!(verify_id_token(text, &expected, &jwks, 1_700_000_000).is_err());
});
