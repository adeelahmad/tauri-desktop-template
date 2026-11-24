//! Windows (WebView2): certificate pinning through `ServerCertificateErrorDetected`,
//! headers through `WebResourceRequested`, mute through `IsMuted`, feature switches through
//! browser arguments, and key capture with a low-level keyboard hook.
//!
//! - **TLS.** A certificate error is allowed only for a request on the site's origin whose
//!   certificate passes the pin; every other error keeps WebView2's error page.
//! - **Headers** are added to every request (pages, scripts, `fetch`, WebSockets' handshake)
//!   that a header rule matches, so the proxy is not needed here.
//! - **Key capture** is a `WH_KEYBOARD_LL` hook on its own thread with its own message loop,
//!   installed while capturing. It takes the Windows key, Alt+Tab, Ctrl+Esc, Alt+F4 and
//!   media keys. **Ctrl+Alt+Del cannot be captured**: Windows keeps the secure attention
//!   sequence for itself. Win+L also locks the computer regardless.

use super::Hooks;
use desktop_core::keymap::win_code;
use desktop_core::wrapper::url::WebUrl;
use desktop_core::wrapper::WebFeatures;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::AppHandle;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{ServerCertificateErrorDetectedEventHandler, WebResourceRequestedEventHandler};
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED,
    LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

/// Captured keys are taken before the web view sees them and sent to the page.
pub const KEYS_ARRIVE_IN_PAGE: bool = false;

/// A string WebView2 allocated, copied and freed.
fn take(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> Option<String> {
    let mut p = PWSTR::null();
    f(&mut p).ok()?;
    if p.is_null() {
        return None;
    }
    // SAFETY: WebView2 returned a NUL-terminated string allocated with CoTaskMemAlloc; it is
    // copied, then freed once.
    unsafe {
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Wire the hooks into a new web view.
pub fn install(pv: tauri::webview::PlatformWebview, hooks: Hooks) {
    // SAFETY: COM calls on the controller Tauri hands us, on the thread that owns it.
    let r = unsafe { install_inner(&pv.controller(), hooks) };
    if let Err(e) = r {
        log::warn!("web app hooks: {e}");
    }
}

/// # Safety
/// Must run on the web view's thread.
unsafe fn install_inner(controller: &ICoreWebView2Controller, hooks: Hooks) -> windows::core::Result<()> {
    // SAFETY: documented WebView2 calls; handlers live as long as the web view.
    unsafe {
        let wv = controller.CoreWebView2()?;
        if hooks.muted {
            wv.cast::<ICoreWebView2_8>()?.SetIsMuted(true)?;
        }
        if let Some(pins) = hooks.pins {
            let site = hooks.site.clone();
            let mut token = 0i64;
            wv.cast::<ICoreWebView2_14>()?.add_ServerCertificateErrorDetected(
                &ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let uri = take(|p| args.RequestUri(p)).unwrap_or_default();
                    let same = WebUrl::parse(&uri).is_ok_and(|u| u.origin() == site);
                    let cert = args.ServerCertificate()?;
                    let der = take(|p| cert.ToPemEncoding(p)).and_then(|pem| desktop_core::wrapper::pin::der_from_pem(&pem));
                    let ok = same && der.is_some_and(|d| pins.accept(&d));
                    args.SetAction(if ok {
                        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW
                    } else {
                        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL
                    })?;
                    Ok(())
                })),
                &mut token,
            )?;
        }
        if let Some((rules, service)) = hooks.headers.filter(|(c, _)| !c.headers.is_empty()) {
            wv.AddWebResourceRequestedFilter(&HSTRING::from("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
            let mut token = 0i64;
            wv.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let req = args.Request()?;
                    let uri = take(|p| req.Uri(p)).unwrap_or_default();
                    match rules.headers_for(&uri, &|s| super::resolve(&service, s)) {
                        Ok(list) if !list.is_empty() => {
                            let h = req.Headers()?;
                            for (k, v) in list {
                                h.SetHeader(&HSTRING::from(k), &HSTRING::from(v))?;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => log::warn!("{e}"),
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
    }
    Ok(())
}

/// Mute or unmute the page.
pub fn set_muted(pv: tauri::webview::PlatformWebview, muted: bool) {
    // SAFETY: COM calls on the live controller, on its thread (inside `with_webview`).
    let r = unsafe { pv.controller().CoreWebView2().and_then(|w| w.cast::<ICoreWebView2_8>()).and_then(|w| w.SetIsMuted(muted)) };
    if let Err(e) = r {
        log::warn!("mute: {e}");
    }
}

/// Chromium switches for the features switched off (the page script removes them too).
pub fn browser_args(features: &WebFeatures) -> Option<String> {
    // Tauri's own defaults, which `additional_browser_args` replaces.
    let mut disabled = vec!["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection"];
    if !features.webrtc {
        disabled.push("WebRtc");
    }
    let mut args = String::new();
    if !features.wasm {
        args.push_str("--js-flags=--noexpose_wasm ");
    }
    if !features.webrtc {
        args.push_str("--force-webrtc-ip-handling-policy=disable_non_proxied_udp ");
    }
    if features.webrtc && features.wasm {
        return None;
    }
    args.push_str(&format!("--disable-features={}", disabled.join(",")));
    Some(args)
}

// ---- key capture ----

static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if (k.flags & LLKHF_INJECTED).0 == 0 {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let ext = (k.flags & LLKHF_EXTENDED).0 != 0;
            if let Some(c) = win_code(k.vkCode, k.scanCode, ext) {
                if std::panic::catch_unwind(|| super::capture::key(c, down)).unwrap_or(false) {
                    return LRESULT(1);
                }
            }
        }
    }
    // SAFETY: passing the event on, as the hook contract asks.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Install the hook on its own thread.
pub fn capture_start(_app: &AppHandle) -> Result<(), String> {
    if HOOK_THREAD.load(Ordering::SeqCst) != 0 {
        return Ok(());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("key-capture".into())
        .spawn(move || {
            // SAFETY: the hook and message loop live on this thread; the hook is removed
            // before it ends.
            unsafe {
                let module = GetModuleHandleW(None).ok().map(|m| m.into());
                match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0) {
                    Ok(h) => {
                        HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
                        let _ = tx.send(Ok(()));
                        let mut msg = MSG::default();
                        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
                        let _ = UnhookWindowsHookEx(h);
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("could not install the keyboard hook: {e}")));
                    }
                }
                HOOK_THREAD.store(0, Ordering::SeqCst);
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

/// Remove the hook.
pub fn capture_stop(_app: &AppHandle) {
    let id = HOOK_THREAD.load(Ordering::SeqCst);
    if id != 0 {
        // SAFETY: asks the hook thread's message loop to end.
        let _ = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

/// Nothing to grant on Windows.
pub fn permissions() -> super::Permissions {
    super::Permissions { granted: true, items: json!([]) }
}

/// No privacy settings to open for this on Windows.
pub fn open_privacy(_pane: &str) -> std::io::Result<()> {
    Ok(())
}
