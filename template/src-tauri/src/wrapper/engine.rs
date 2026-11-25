//! The web engine under the site's window, and the few hooks that need its native web view.
//!
//! Tauri 3 picks the engine when the app starts (`tauri::Builder::runtime`, see
//! `crate::engine`), so the wrapper is generic over the runtime `R` and names no engine.
//! Almost everything it does goes through Tauri's own API, which every engine implements:
//! navigation and new windows ([`super::navigate`]: other origins open in the browser),
//! downloads, media permissions, page scripts, zoom and the menu.
//!
//! What Tauri does not cover goes through [`Engine`], implemented for every runtime on the
//! engine this app was built with:
//!
//! | Hook | System web view (wry) | Chromium (`cfg(cef)`, macOS) |
//! |---|---|---|
//! | Mute | per OS ([`super::platform`]): `_setPageMuted:`, WebView2, WebKitGTK | `CefBrowserHost::SetAudioMuted`: the whole page, iframes and WebRTC too |
//! | Pinned self-signed site | the OS's certificate hook in direct mode, or the loopback proxy | always the loopback proxy (`desktop_core::wrapper::proxy`, rustls and the pin); the window itself refuses certificate errors |
//! | Request headers | WebView2 adds them (Windows); elsewhere the proxy | the proxy |
//! | Browser features off | WebView2 switches; WebKitGTK settings; the page script | the page script |
//! | Permissions Tauri has no kind for | refused | Chromium's own handling (refusing them breaks storage access and sign-in flows) |
//! | Cookies and storage | the web view's persistent store (`<data>/wrapper`) | the Chromium profile under `<data>/cef` |
//!
//! # Example
//!
//! Code that only needs Tauri's API takes any runtime; code that needs the engine asks for
//! `R: Engine` instead:
//!
//! ```
//! use tauri::{AppHandle, Runtime};
//!
//! fn app_name<R: Runtime>(app: &AppHandle<R>) -> String {
//!     app.package_info().name.clone()
//! }
//! # let _ = app_name::<tauri::DynRuntime>;
//! ```

use super::Hooks;
use desktop_core::wrapper::{WebFeatures, WrapperConfig};
use tauri::webview::PermissionResponse;
use tauri::{Runtime, WebviewWindow};

/// A Tauri runtime whose web views the wrapper's engine hooks can reach.
///
/// `install` and `set_muted` run on the UI thread, after the window is made. With
/// `tauri::DynRuntime` they fail with Tauri's `RuntimeTypeMismatch` error when the app runs
/// another engine than the one it was built for.
pub trait Engine: Runtime {
    /// Wire the hooks ([`Hooks`]) into the window's web view.
    fn install(window: &WebviewWindow<Self>, hooks: Hooks) -> tauri::Result<()>;

    /// Mute or unmute the page's audio in the engine itself (the page script mutes the
    /// media elements as well).
    fn set_muted(window: &WebviewWindow<Self>, muted: bool) -> tauri::Result<()>;

    /// Extra browser arguments for the window (WebView2 only).
    fn browser_args(features: &WebFeatures) -> Option<String>;

    /// Whether the site must go through the loopback proxy although `cfg.uses_proxy()` says
    /// not: what this engine cannot do in direct mode (pinning, headers).
    fn needs_proxy(cfg: &WrapperConfig) -> bool;

    /// The answer to a permission request Tauri has no kind for (`PermissionKind::Other`).
    fn other_permission() -> PermissionResponse;
}

/// The system web view, through wry.
#[cfg(not(cef))]
impl<R> Engine for R
where
    R: Runtime,
    R::Handle: tauri_runtime_wry::AsWryHandle,
{
    fn install(window: &WebviewWindow<R>, hooks: Hooks) -> tauri::Result<()> {
        use tauri_runtime_wry::WebviewWryExt;
        window.with_wry_webview(move |pv| super::platform::install(pv, hooks))
    }

    fn set_muted(window: &WebviewWindow<R>, muted: bool) -> tauri::Result<()> {
        use tauri_runtime_wry::WebviewWryExt;
        window.with_wry_webview(move |pv| super::platform::set_muted(pv, muted))
    }

    fn browser_args(features: &WebFeatures) -> Option<String> {
        super::platform::browser_args(features)
    }

    fn needs_proxy(_cfg: &WrapperConfig) -> bool {
        false
    }

    fn other_permission() -> PermissionResponse {
        PermissionResponse::Deny
    }
}

/// Chromium (CEF), through `tauri-runtime-cef` (feature `chromium`, macOS).
#[cfg(cef)]
impl<R> Engine for R
where
    R: Runtime,
    R::WebviewDispatcher: tauri_runtime_cef::AsCefWebviewDispatcher,
{
    fn install(window: &WebviewWindow<R>, hooks: Hooks) -> tauri::Result<()> {
        if hooks.pins.is_some() {
            // `proxy: "off"` with `insecureTls`: CEF has no per-certificate hook.
            log::warn!("certificate pinning on Chromium needs the proxy (wrapper.proxy is off): certificate errors are refused");
        }
        if hooks.headers.is_some_and(|(c, _)| !c.headers.is_empty()) {
            log::warn!("request headers on Chromium need the proxy (wrapper.proxy is off): they are not sent");
        }
        if hooks.muted {
            Self::set_muted(window, true)?;
        }
        Ok(())
    }

    fn set_muted(window: &WebviewWindow<R>, muted: bool) -> tauri::Result<()> {
        use tauri_runtime_cef::cef::{ImplBrowser, ImplBrowserHost};
        use tauri_runtime_cef::WebviewCefExt;
        window.with_cef_webview(move |cw| {
            if let Some(host) = cw.browser().host() {
                host.set_audio_muted(i32::from(muted));
                log::info!("page audio {}", if muted { "muted" } else { "unmuted" });
            }
        })
    }

    fn browser_args(_features: &WebFeatures) -> Option<String> {
        // Chromium's switches are per process (`Cef::command_line_arg`); the page script
        // switches the features off.
        None
    }

    fn needs_proxy(cfg: &WrapperConfig) -> bool {
        !matches!(cfg.proxy, desktop_core::wrapper::ProxyMode::Off) && (cfg.insecure_tls || !cfg.headers.is_empty())
    }

    fn other_permission() -> PermissionResponse {
        PermissionResponse::Default
    }
}

/// Whether the site goes through the loopback proxy with engine `R`.
pub fn uses_proxy<R: Engine>(cfg: &WrapperConfig) -> bool {
    cfg.uses_proxy() || R::needs_proxy(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::wrapper::ProxyMode;

    fn is_engine<R: Engine>() {}

    #[test]
    fn the_runtime_the_app_starts_with_reaches_the_web_view() {
        // `run()` uses the type-erased runtime.
        is_engine::<tauri::DynRuntime>();
    }

    #[test]
    fn the_proxy_is_used_when_the_settings_ask_for_it() {
        let on = WrapperConfig { proxy: ProxyMode::On, ..WrapperConfig::default() };
        assert!(uses_proxy::<tauri::DynRuntime>(&on));
        let off = WrapperConfig { proxy: ProxyMode::Off, insecure_tls: true, ..WrapperConfig::default() };
        assert!(!uses_proxy::<tauri::DynRuntime>(&off));
    }

    #[test]
    fn pinning_and_headers_take_the_proxy_on_chromium_only() {
        let pinned = WrapperConfig { insecure_tls: true, ..WrapperConfig::default() };
        assert_eq!(uses_proxy::<tauri::DynRuntime>(&pinned), cfg!(cef) || pinned.uses_proxy());
        assert_eq!(matches!(<tauri::DynRuntime as Engine>::other_permission(), PermissionResponse::Default), cfg!(cef));
    }
}
