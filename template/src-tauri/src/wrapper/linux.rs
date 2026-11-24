//! Linux (WebKitGTK): TLS pinning through `load-failed-with-tls-errors`, mute, WebRTC off
//! through `WebKitSettings`, and key capture with a GDK keyboard grab.
//!
//! - **TLS.** When a page on the site fails with a certificate error, the certificate is
//!   checked against the pin ([`desktop_core::wrapper::pin`]); if it passes, it is allowed
//!   for that host only (`webkit_web_context_allow_tls_certificate_for_host`) and the page
//!   reloads. Other hosts keep WebKit's error page.
//! - **Key capture** grabs the keyboard for the window (`gdk_seat_grab`): on X11 that is
//!   `XGrabKeyboard`, on Wayland GTK asks the compositor to inhibit its shortcuts
//!   (`zwp_keyboard_shortcuts_inhibit_manager_v1`; GNOME asks the user once). Compositors
//!   without that protocol keep their own shortcuts. The keys then reach the page as real
//!   key events, so the release shortcut is caught by the page script.
//! - **Headers** need the proxy here (no per-request hook without a web extension).

use super::Hooks;
use desktop_core::wrapper::{WebFeatures, LABEL};
use gtk::gdk::prelude::*;
use serde_json::json;
use tauri::{AppHandle, Manager};
use webkit2gtk::gio::prelude::TlsCertificateExt;
use webkit2gtk::{SettingsExt, WebContextExt, WebViewExt};

/// Captured keys arrive in the page as the web view's own key events.
pub const KEYS_ARRIVE_IN_PAGE: bool = true;

/// Wire the hooks into a new web view.
pub fn install(pv: tauri::webview::PlatformWebview, hooks: Hooks) {
    let wv = pv.inner();
    wv.set_is_muted(hooks.muted);
    if let Some(s) = WebViewExt::settings(&wv) {
        if !hooks.features.webrtc {
            s.set_enable_webrtc(false);
        }
        s.set_enable_media_stream(true);
    }
    if let Some(pins) = hooks.pins {
        let site = hooks.site;
        wv.connect_load_failed_with_tls_errors(move |wv, uri, cert, _errors| {
            let same = desktop_core::wrapper::url::WebUrl::parse(uri).is_ok_and(|u| u.origin() == site);
            let der = cert.certificate().map(|b| b.to_vec()).unwrap_or_default();
            if !same || der.is_empty() || !pins.accept(&der) {
                return false; // WebKit's own error page
            }
            if let Some(ctx) = wv.context() {
                ctx.allow_tls_certificate_for_host(cert, site.host.trim_start_matches('[').trim_end_matches(']'));
            }
            wv.load_uri(uri);
            true
        });
    }
}

/// Mute or unmute the page.
pub fn set_muted(pv: tauri::webview::PlatformWebview, muted: bool) {
    pv.inner().set_is_muted(muted);
}

/// No extra browser arguments on Linux.
pub fn browser_args(_features: &WebFeatures) -> Option<String> {
    None
}

/// Grab the keyboard for the site's window (on the main thread).
pub fn capture_start(app: &AppHandle) -> Result<(), String> {
    let w = app.get_webview_window(LABEL).ok_or("no window")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let win = w.clone();
    w.run_on_main_thread(move || {
        let r = (|| -> Result<(), String> {
            let gtk_win = win.gtk_window().map_err(|e| e.to_string())?;
            let gdk_win = gtk::prelude::WidgetExt::window(&gtk_win).ok_or("the window is not shown")?;
            let seat = gdk_win.display().default_seat().ok_or("no keyboard")?;
            match seat.grab(&gdk_win, gtk::gdk::SeatCapabilities::KEYBOARD, false, None, None, None) {
                gtk::gdk::GrabStatus::Success => Ok(()),
                other => Err(format!("the keyboard grab was refused ({other:?})")),
            }
        })();
        let _ = tx.send(r);
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(2)).map_err(|_| "the keyboard grab timed out".to_string())?
}

/// Let the keyboard go.
pub fn capture_stop(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let win = w.clone();
        let _ = w.run_on_main_thread(move || {
            if let Some(seat) =
                win.gtk_window().ok().and_then(|g| gtk::prelude::WidgetExt::window(&g)).and_then(|g| g.display().default_seat())
            {
                seat.ungrab();
            }
        });
    }
}

/// Nothing to grant on Linux (Wayland compositors may ask once themselves).
pub fn permissions() -> super::Permissions {
    super::Permissions { granted: true, items: json!([]) }
}

/// No privacy settings to open on Linux.
pub fn open_privacy(_pane: &str) -> std::io::Result<()> {
    Ok(())
}
