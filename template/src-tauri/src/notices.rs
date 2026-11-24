//! Third-party licence notices for Settings → About → Licences (feature `supply-chain`).
//!
//! `build.rs` writes the notices for exactly what this build links (see
//! `desktop_core::notices`); this module compiles them in and serves them to the UI with
//! the `third_party_notices` command.
//!
//! # Example
//!
//! `third_party_notices` returns:
//!
//! ```
//! let reply = serde_json::json!({
//!     "packages": [{"ecosystem": "rust", "name": "serde", "version": "1.0.228", "license": "MIT OR Apache-2.0", "repository": null}],
//!     "text": "# Third-party notices\n...",
//! });
//! assert_eq!(reply["packages"][0]["license"], "MIT OR Apache-2.0");
//! ```

use serde_json::{json, Value};

/// The full notices (Markdown), as written by the build script.
pub const TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/THIRD_PARTY_NOTICES.md"));
/// The package list (JSON array), as written by the build script.
const PACKAGES: &str = include_str!(concat!(env!("OUT_DIR"), "/third_party.json"));

/// The package list and the full notices.
pub fn notices() -> Value {
    json!({
        "packages": serde_json::from_str::<Value>(PACKAGES).unwrap_or_else(|_| json!([])),
        "text": TEXT,
    })
}

/// Tauri command: the licences of the software this app ships.
#[tauri::command]
pub fn third_party_notices() -> Value {
    notices()
}

#[cfg(test)]
mod tests {
    #[test]
    fn notices_are_compiled_in() {
        let n = super::notices();
        assert!(n["text"].as_str().unwrap().starts_with("# Third-party notices"));
        assert!(n["packages"].is_array());
    }
}
