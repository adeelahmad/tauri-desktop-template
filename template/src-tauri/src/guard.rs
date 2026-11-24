//! Security defaults for the window, always on:
//! - the web view may only show the app's own pages: navigation anywhere else is refused
//!   (and logged), so a link or injected script cannot load a remote page that would
//!   then reach the app's commands;
//! - developer tools exist in debug builds only (Tauri's `devtools` feature is not
//!   enabled), and `APP_DEVTOOLS=1` opens them at start in a debug build.
//!
//! The content security policy is in `tauri.conf.json` (`app.security.csp`).
{% if webview or web_wrapper %}//!
//! Windows labelled `web-*` (features `webview` and `web-wrapper`) are not the app's pages:
//! their navigations are checked by their own hooks in `webview.rs` and `wrapper/` instead.
{% endif %}
//!
//! # Example
//!
//! The app's own pages are `tauri://localhost` (macOS, Linux) and `http(s)://tauri.localhost`
//! (Windows); anything else is refused:
//!
//! ```
//! let page = tauri::Url::parse("https://tauri.localhost/#/settings").unwrap();
//! let other = tauri::Url::parse("https://tauri.localhost.evil.com/").unwrap();
//! assert_eq!(page.host_str(), Some("tauri.localhost"));
//! assert_ne!(other.host_str(), Some("tauri.localhost"));
//! ```

use tauri::{Manager, Runtime};

/// Whether `url` is one of the app's own pages.
pub fn allowed(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" | "asset" | "ipc" => true,
        "http" | "https" => {
            let host = url.host_str().unwrap_or_default();
            host == "tauri.localhost" || host == "ipc.localhost" || (cfg!(debug_assertions) && matches!(host, "localhost" | "127.0.0.1"))
        }
        "about" => url.as_str() == "about:blank",
        _ => false,
    }
}

/// The Tauri plugin that enforces this: register it with `Builder::plugin`.
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("guard")
        .on_navigation(|webview, url| {
{% if webview or web_wrapper %}            // Embedded web views and the web app's window have their own hooks, and no IPC.
            #[cfg(any(feature = "webview", feature = "web-wrapper"))]
            if desktop_core::webview::is_webview_label(webview.label()) {
                return true;
            }
{% endif %}            // A debug build may also show the dev server, which on iOS and Android is this
            // computer's address on the local network rather than `localhost`.
            let dev = cfg!(debug_assertions) && webview.config().build.dev_url.as_ref().is_some_and(|d| d.origin() == url.origin());
            let ok = allowed(url) || dev;
            if !ok {
                log::warn!("blocked navigation to {}", url.origin().ascii_serialization());
            }
            ok
        })
        .on_webview_ready(|_webview| {
            #[cfg(debug_assertions)]
            if std::env::var_os("APP_DEVTOOLS").is_some_and(|v| v == "1") {
                _webview.open_devtools();
            }
        })
        .setup(|app, _| {
            log::info!("{} {} starting", app.package_info().name, app.package_info().version);
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn only_the_apps_own_pages() {
        let u = |s: &str| tauri::Url::parse(s).unwrap();
        assert!(allowed(&u("tauri://localhost/index.html")));
        assert!(allowed(&u("http://tauri.localhost/")));
        assert!(allowed(&u("https://tauri.localhost/#/settings")));
        assert!(!allowed(&u("https://example.com/")));
        assert!(!allowed(&u("https://tauri.localhost.evil.com/")));
        assert!(!allowed(&u("file:///etc/passwd")));
        assert!(!allowed(&u("javascript:alert(1)")));
        assert_eq!(allowed(&u("http://localhost:5173/")), cfg!(debug_assertions));
    }
}
