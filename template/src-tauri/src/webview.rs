//! Embedded web views (feature `webview`, desktop only): remote pages in the app's own
//! windows, behind the allowlist in `desktop_core::webview`.
//!
//! Commands: `webview_open {url, name?, title?}` opens (or reuses) the window `web-<name>`
//! and returns its label, `webview_close {name}` closes it, and `webview_post {name,
//! message}` sends a JSON message to a page that may use the bridge. Each window is a Tauri 2
//! `WebviewWindowBuilder` window. Child webviews inside the main window need Tauri's
//! `unstable` feature, so this module does not use them.
//!
//! # Security
//!
//! - **No IPC for remote pages.** `capabilities/default.json` names only the `main`
//!   window, and `capabilities/webview.json` gives `web-*` windows no permissions and no
//!   `remote` URLs, so Tauri's ACL refuses every plugin and app command from them. On top
//!   of that, [`guard_invoke`] turns away any command that comes from a `web-*` window
//!   before it runs.
//! - **The app's own window is unchanged:** the guard plugin keeps it on the app's pages,
//!   and the content security policy in `tauri.conf.json` (`app.security.csp`) still
//!   applies to it. Remote pages keep their own sites' policies.
//! - **Every navigation** (links, redirects, `window.open`, and on macOS and Linux also
//!   frames) goes through the allowlist. Blocked ones are refused, or with `blocked:
//!   "browser"` opened in the system browser (http and https only, rate limited).
//! - **Bridge** (off by default): pages on `bridgeHosts` get `window.kitchenBridge.post(m)`.
//!   The message arrives in the main window as the `webview-message` event `{label, host,
//!   data}`. It is the only way in, it carries JSON only, and it never reaches a command.
//!
//! # Isolation per platform
//!
//! | Platform | `isolation: "profile"` (default) | `isolation: "incognito"` |
//! |---|---|---|
//! | Windows (WebView2) | own user data folder `<profile data>/webview` | InPrivate |
//! | Linux (WebKitGTK) | own web context in `<profile data>/webview` | ephemeral context |
//! | macOS 14+ (WKWebView) | own data store per profile (`data_store_identifier`) | non-persistent store |
//! | macOS 13 | the app's default store (shared by profiles): use incognito | non-persistent store |
//!
//! iOS and Android builds leave this module out.
//!
//! # Example
//!
//! From the frontend, with `webview.enabled` on and `docs.rs` on `webview.allow`:
//!
//! ```js
//! const label = await invoke('webview_open', { url: 'https://docs.rs/tauri', name: 'docs' });
//! await invoke('webview_close', { name: 'docs' });
//! ```
//!
//! The navigation decision, as the hook makes it:
//!
//! ```
//! use desktop_core::webview::{Verdict, WebviewConfig};
//!
//! let cfg = WebviewConfig { enabled: true, allow: vec!["docs.rs".into()], ..WebviewConfig::default() }.validated();
//! let url = tauri::Url::parse("https://docs.rs/tauri").unwrap();
//! assert_eq!(cfg.verdict(url.scheme(), url.host_str().unwrap()), Verdict::Allow);
//! ```

use desktop_core::webview::{self as policy, Isolation, Verdict, WebviewConfig};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder};

/// The settings, or an error when web views are switched off.
fn config(app: &AppHandle) -> Result<WebviewConfig, String> {
    let cfg = super::load_settings(app).config.webview.validated();
    if !cfg.enabled {
        return Err("embedded web views are switched off (Settings → Web views)".into());
    }
    Ok(cfg)
}

/// The allowlist's decision, with the network policy on top (offline mode and the deny
/// list refuse a page the allowlist would show).
fn decide(cfg: &WebviewConfig, url: &Url) -> Verdict {
    if url.scheme() == "about" && matches!(url.path(), "blank" | "srcdoc") {
        return Verdict::Allow;
    }
    let host = url.host_str().unwrap_or_default();
    match cfg.verdict(url.scheme(), host) {
        Verdict::Allow if desktop_core::net::check_host(host).is_err() => Verdict::Refuse,
        v => v,
    }
}

/// The label for `name`, or an error a person can read.
fn label_for(name: Option<&str>) -> Result<String, String> {
    policy::label(name.unwrap_or("main")).ok_or_else(|| "a web view name is letters, digits, - and _ (at most 40)".to_string())
}

/// Opens http(s) URLs in the system browser, at most once every two seconds per window,
/// so a page cannot flood the desktop with tabs.
#[derive(Clone)]
struct Outside(Arc<Mutex<Option<Instant>>>);

impl Outside {
    fn open(&self, url: &Url) {
        let mut last = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            log::warn!("web view: not opening {} (too many in a row)", url.origin().ascii_serialization());
            return;
        }
        *last = Some(Instant::now());
        if let Err(e) = open_in_browser(url) {
            log::warn!("web view: could not open the system browser: {e}");
        }
    }
}

/// The system browser, without a shell (the URL is one argument and only http or https).
fn open_in_browser(url: &Url) -> std::io::Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(std::io::Error::other("only http and https go to the browser"));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url.as_str()).spawn().map(|_| ())
}

/// The decision for one navigation, acted on: `true` lets the web view go there.
fn route(cfg: &WebviewConfig, outside: &Outside, url: &Url) -> bool {
    match decide(cfg, url) {
        Verdict::Allow => true,
        Verdict::External => {
            outside.open(url);
            false
        }
        Verdict::Refuse => {
            log::warn!("web view: blocked navigation to {}", url.origin().ascii_serialization());
            false
        }
    }
}

/// A bridge message from the page now showing on `host`: passed to the main window when
/// that host may use the bridge, dropped (and logged) otherwise.
fn bridge_in(app: &AppHandle, cfg: &WebviewConfig, label: &str, host: &str, url: &Url) {
    if !cfg.bridge_allowed(host) {
        log::warn!("web view {label}: bridge message from {host} refused");
        return;
    }
    let raw = url.query_pairs().find(|(k, _)| k == "m").map(|(_, v)| v.into_owned()).unwrap_or_default();
    match policy::parse_message(&raw) {
        Ok(data) => {
            let (app, payload) = (app.clone(), json!({ "label": label, "host": host, "data": data }));
            // Not from inside the navigation callback: the event loop is busy there.
            std::thread::spawn(move || {
                if let Err(e) = app.emit_to("main", "webview-message", payload) {
                    log::warn!("web view: could not pass a bridge message on: {e}");
                }
            });
        }
        Err(e) => log::warn!("web view {label}: {e}"),
    }
}

/// Open `url` in the web view window `web-<name>` (made if needed) and return its label.
#[tauri::command(async)]
pub fn webview_open(app: AppHandle, url: String, name: Option<String>, title: Option<String>) -> Result<String, String> {
    let cfg = config(&app)?;
    let url = Url::parse(&url).map_err(|e| format!("not a URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || decide(&cfg, &url) != Verdict::Allow {
        return Err(format!("{} is not on the web view allowlist", url.host_str().unwrap_or(url.scheme())));
    }
    let label = label_for(name.as_deref())?;
    if let Some(w) = app.get_webview_window(&label) {
        w.navigate(url).map_err(super::err)?;
        let _ = w.set_focus();
        return Ok(label);
    }
    let open = app.webview_windows().keys().filter(|l| policy::is_webview_label(l)).count();
    if open >= cfg.max_windows {
        return Err(format!("at most {} web views can be open at once", cfg.max_windows));
    }

    let host = Arc::new(Mutex::new(url.host_str().unwrap_or_default().to_string()));
    let outside = Outside(Arc::default());
    let on_nav = {
        let (app, cfg, label, host, outside) = (app.clone(), cfg.clone(), label.clone(), host.clone(), outside.clone());
        move |url: &Url| {
            if url.host_str() == Some(policy::BRIDGE_HOST) {
                let now = host.lock().unwrap_or_else(PoisonError::into_inner).clone();
                bridge_in(&app, &cfg, &label, &now, url);
                return false;
            }
            route(&cfg, &outside, url)
        }
    };
    let on_new = {
        let (app, cfg, label) = (app.clone(), cfg.clone(), label.clone());
        move |url: Url, _features| {
            // `window.open` and `target=_blank` stay in this window when allowed.
            if route(&cfg, &outside, &url) {
                let (app, label) = (app.clone(), label.clone());
                std::thread::spawn(move || {
                    if let Some(w) = app.get_webview_window(&label) {
                        let _ = w.navigate(url);
                    }
                });
            }
            NewWindowResponse::Deny
        }
    };
    let on_load = move |_: tauri::WebviewWindow, page: tauri::webview::PageLoadPayload<'_>| {
        if page.event() == PageLoadEvent::Started || page.event() == PageLoadEvent::Finished {
            *host.lock().unwrap_or_else(PoisonError::into_inner) = page.url().host_str().unwrap_or_default().to_ascii_lowercase();
        }
    };

    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url.clone()))
        .title(title.unwrap_or_else(|| url.host_str().unwrap_or_default().to_string()))
        .inner_size(1024.0, 768.0)
        .on_navigation(on_nav)
        .on_new_window(on_new)
        .on_page_load(on_load);
    builder = isolate(builder, &app, cfg.isolation);
    if cfg.bridge && !cfg.bridge_hosts.is_empty() {
        builder = builder.initialization_script(policy::bridge_script(&cfg));
    }
    builder.build().map_err(super::err)?;
    log::info!("web view {label} opened on {}", url.origin().ascii_serialization());
    Ok(label)
}

/// Where the window keeps cookies and storage (see the table in the module docs).
fn isolate<'a, R: Runtime, M: Manager<R>>(builder: WebviewWindowBuilder<'a, R, M>, app: &AppHandle<R>, isolation: Isolation) -> WebviewWindowBuilder<'a, R, M> {
    match isolation {
        Isolation::Incognito => builder.incognito(true),
        Isolation::Profile => builder
            // Windows and Linux.
            .data_directory(super::paths::app_dirs().1.join("webview"))
            // macOS 14+ (ignored elsewhere, and on macOS 13 the default store is used).
            .data_store_identifier(policy::store_id(&app.config().identifier, super::paths::profile())),
    }
}

/// Close the web view window `web-<name>` (nothing happens when it is not open).
#[tauri::command(async)]
pub fn webview_close(app: AppHandle, name: String) -> Result<(), String> {
    let label = label_for(Some(&name))?;
    match app.get_webview_window(&label) {
        Some(w) => w.close().map_err(super::err),
        None => Ok(()),
    }
}

/// Send `message` (JSON) to the page in `web-<name>`, when that page may use the bridge.
/// The page receives a `message` event whose `origin` is `"kitchen-app"`.
#[tauri::command(async)]
pub fn webview_post(app: AppHandle, name: String, message: Value) -> Result<(), String> {
    let cfg = config(&app)?;
    let label = label_for(Some(&name))?;
    let w = app.get_webview_window(&label).ok_or_else(|| format!("no web view {label}"))?;
    let page = w.url().map_err(super::err)?;
    let host = page.host_str().unwrap_or_default();
    if page.scheme() != "https" || !cfg.bridge_allowed(host) {
        return Err(format!("{host} may not use the bridge (Settings → Web views)"));
    }
    if message.to_string().len() > policy::MAX_MESSAGE {
        return Err(format!("message over {} bytes", policy::MAX_MESSAGE));
    }
    w.eval(policy::deliver_script(&message)).map_err(super::err)
}

/// Run `invoke` through `handler`, unless it comes from a web view window: then it is
/// refused before any command runs (Tauri's ACL refuses it as well; this is the second lock).
pub fn guard_invoke<R: Runtime>(invoke: tauri::ipc::Invoke<R>, handler: impl Fn(tauri::ipc::Invoke<R>) -> bool) -> bool {
    let label = invoke.message.webview_ref().label();
    if !policy::is_webview_label(label) {
        return handler(invoke);
    }
    log::warn!("web view {label}: refused command {}", invoke.message.command());
    invoke.resolver.reject("commands are not available to web views");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WebviewConfig {
        WebviewConfig { enabled: true, allow: vec!["*.example.com".into()], ..WebviewConfig::default() }.validated()
    }

    #[test]
    fn decisions_on_parsed_urls() {
        let c = cfg();
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(decide(&c, &u("https://www.example.com/a?b#c")), Verdict::Allow);
        assert_eq!(decide(&c, &u("https://example.com.evil.net/")), Verdict::Refuse);
        assert_eq!(decide(&c, &u("https://user@evil.net/@www.example.com")), Verdict::Refuse);
        assert_eq!(decide(&c, &u("about:blank")), Verdict::Allow);
        assert_eq!(decide(&c, &u("javascript:alert(1)")), Verdict::Refuse);
        assert_eq!(decide(&c, &u("https://tauri.localhost/")), Verdict::Refuse);
        assert_eq!(decide(&c, &u(policy::BRIDGE_URL)), Verdict::Refuse);
    }

    #[test]
    fn labels_never_reach_the_main_window() {
        assert_eq!(label_for(Some("main")).unwrap(), "web-main");
        assert_eq!(label_for(None).unwrap(), "web-main");
        assert!(label_for(Some("a b")).is_err());
    }

    #[test]
    fn only_http_goes_to_the_browser() {
        assert!(open_in_browser(&Url::parse("file:///etc/passwd").unwrap()).is_err());
    }
}
