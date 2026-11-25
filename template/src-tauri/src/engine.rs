//! The web engine the app runs on, chosen when it was generated (`web_engine`).
//!
//! Tauri 3 ships no engine of its own: `run()` hands [`runtime`] to `Builder::runtime`, and
//! every Tauri type in this crate is the type-erased `tauri::DynRuntime`, so nothing else
//! names the engine.
//!
//! | `web_engine` | macOS | Windows | Linux |
//! |---|---|---|---|
//! | `system` (default) | WKWebView (wry) | WebView2 (wry) | WebKitGTK (wry) |
//! | `chromium` (feature `chromium`) | Chromium, CEF through `tauri-runtime-cef` (`cfg(cef)`) | WebView2 | WebKitGTK |
//!
//! With Chromium the app keeps one Chromium profile under `<data>/cef` (cookies, local
//! storage, IndexedDB, granted permissions), keeps session cookies across restarts, never
//! offers a click-through on a certificate error (a pinned self-signed site goes through
//! the web wrapper's loopback proxy instead), and blocks notifications, geolocation, USB,
//! serial and HID for web pages. Bundling, signing and the helper apps are in
//! docs/GUIDE.md, "Chromium engine".
//!
//! # Example
//!
//! ```
//! assert!(["system", "chromium"].contains(&app_lib::engine::NAME));
//! ```

/// The engine this build runs on: `"system"` or `"chromium"`.
#[cfg(not(cef))]
pub const NAME: &str = "system";
/// The engine this build runs on: `"system"` or `"chromium"`.
#[cfg(cef)]
pub const NAME: &str = "chromium";

/// The system web view, through wry.
#[cfg(not(cef))]
pub fn runtime() -> tauri_runtime_wry::Wry {
    tauri_runtime_wry::Wry::default()
}

/// Chromium (CEF), with a persistent profile of the app's own.
#[cfg(cef)]
pub fn runtime() -> tauri_runtime_cef::Cef {
    use tauri_runtime_cef::cef::{ContentSettingTypes as Kind, ContentSettingValues as Value};
    use tauri_runtime_cef::{Cef, CertificateErrorPolicy};
    let blocked = [Kind::NOTIFICATIONS, Kind::GEOLOCATION, Kind::USB_GUARD, Kind::SERIAL_GUARD, Kind::HID_GUARD];
    blocked.into_iter().fold(
        Cef::default()
            // The data folder, not CEF's default under the cache folder: macOS may purge
            // `~/Library/Caches`, and this holds the sign-ins.
            .root_cache_path(profile_dir(&crate::paths::app_dirs().1))
            .persist_session_cookies(true)
            .certificate_errors(CertificateErrorPolicy::Cancel),
        |cef, kind| cef.default_content_setting(kind, Value::BLOCK),
    )
}

/// The Chromium profile folder inside the app's data folder.
#[cfg_attr(not(cef), allow(dead_code))]
pub fn profile_dir(data: &std::path::Path) -> std::path::PathBuf {
    data.join("cef")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chromium_profile_lives_in_the_data_folder() {
        let data = std::path::Path::new("/data/com.example.app");
        assert_eq!(profile_dir(data), data.join("cef"));
        assert!(profile_dir(data).starts_with(data));
    }

    #[test]
    fn the_name_says_which_engine_was_built() {
        assert_eq!(NAME, if cfg!(cef) { "chromium" } else { "system" });
    }
}
