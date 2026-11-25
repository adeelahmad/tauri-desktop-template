//! The web engine under the site's window, and the few hooks that need its native web view.
//!
//! Tauri 3 picks the engine when the app starts (`tauri::Builder::runtime`), so the wrapper
//! is generic over the runtime `R` and names no engine. Almost everything it does goes
//! through Tauri's own API, which every engine implements: navigation and new windows
//! ([`super::navigate`]), downloads, media permissions, page scripts, zoom and the menu.
//!
//! What Tauri does not cover needs the engine's own web view object: muting the page, the
//! certificate hook for a pinned self-signed site in direct mode, WebView2's request headers
//! and WebKitGTK's settings. Those are the per-OS functions in [`super::platform`], and
//! [`Engine`] hands them the native web view. It is implemented for every runtime that runs
//! on the engine this app was built with: the system web view through wry (WKWebView,
//! WebView2, WebKitGTK), whether the app names it (`tauri_runtime_wry::WryRuntime`) or uses
//! the type-erased `tauri::DynRuntime` that runs wry, as `run()` does.
//!
//! # Example
//!
//! Code that only needs Tauri's API takes any runtime; code that needs the native web view
//! asks for `R: Engine` instead:
//!
//! ```
//! use tauri::{AppHandle, Runtime};
//!
//! fn app_name<R: Runtime>(app: &AppHandle<R>) -> String {
//!     app.package_info().name.clone()
//! }
//! # let _ = app_name::<tauri::DynRuntime>;
//! ```

use super::{platform, Hooks};
use tauri::{Runtime, WebviewWindow};
use tauri_runtime_wry::{AsWryHandle, WebviewWryExt};

/// A Tauri runtime whose web views the wrapper's per-OS hooks can reach.
///
/// Both calls run on the UI thread, after the window is made. With `tauri::DynRuntime`
/// they fail with Tauri's `RuntimeTypeMismatch` error when the app runs another engine.
pub trait Engine: Runtime {
    /// Wire the per-OS hooks ([`Hooks`]) into the window's web view.
    fn install(window: &WebviewWindow<Self>, hooks: Hooks) -> tauri::Result<()>;

    /// Mute or unmute the page's audio in the engine itself (the page script mutes the
    /// media elements as well).
    fn set_muted(window: &WebviewWindow<Self>, muted: bool) -> tauri::Result<()>;
}

/// The system web view, through wry.
impl<R> Engine for R
where
    R: Runtime,
    R::Handle: AsWryHandle,
{
    fn install(window: &WebviewWindow<R>, hooks: Hooks) -> tauri::Result<()> {
        window.with_wry_webview(move |pv| platform::install(pv, hooks))
    }

    fn set_muted(window: &WebviewWindow<R>, muted: bool) -> tauri::Result<()> {
        window.with_wry_webview(move |pv| platform::set_muted(pv, muted))
    }
}

#[cfg(test)]
mod tests {
    use super::Engine;

    fn is_engine<R: Engine>() {}

    #[test]
    fn the_runtimes_the_app_can_start_with_reach_the_web_view() {
        // `run()` uses the type-erased runtime; naming wry's runtime works too.
        is_engine::<tauri::DynRuntime>();
        is_engine::<tauri_runtime_wry::WryRuntime>();
    }
}
