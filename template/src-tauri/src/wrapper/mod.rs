//! The web app wrapper (feature `web-wrapper`, desktop only): one remote site in its own
//! window, made into a desktop app. The policy (URL guard, pinning, rules, the proxy) is
//! `desktop_core::wrapper`; this module opens the window, wires the per-OS hooks and serves
//! Preferences.
//!
//! - **The window** (`web-app`) shows the site with **no IPC**: no capability names it, and
//!   `lib.rs`' guard refuses any command call from a `web-*` window. Navigation stays on the
//!   site's origin; other pages, `target=_blank` and `window.open` open in the system browser.
//! - **Sessions persist**: cookies, local storage and IndexedDB live in the web view's
//!   persistent store (`<data>/wrapper` on Windows and Linux, the app's default
//!   `WKWebsiteDataStore` on macOS), never incognito.
//! - **Menu**: Reload, Back, Forward, Zoom, Toggle Key Capture, Mute, Fill Form,
//!   Open in Browser…, Preferences…, Quit (plus the usual Edit menu, so copy and paste work).
//! - **Open in Browser…** (Cmd/Ctrl+Shift+O) opens the page the window shows in a real
//!   browser, for sites that break in the web view: the user's default browser, or the
//!   developer's `wrapper.openInBrowserApp` (macOS `open -a <app>`, elsewhere a program on
//!   `PATH`), started with an argument list, never a shell. Only an `http(s)` page on the
//!   wrapped origin is opened (through the proxy: the site's own URL, never the local one
//!   with its token); see `desktop_core::wrapper::browser`.
//! - **Settings are the developer's.** The `wrapper` section of `config/settings.json` is
//!   compiled in ([`baked`]) and read-only at run time. The user can change only the fields
//!   in its `userEditable` list (by default the release shortcut, zoom, mute and "start
//!   captured"); those changes and the certificate pinned on first use are saved apart, as
//!   `config.wrapper` ([`UserWrapper`]), and merged over the built-in settings ([`effective`]).
//! - **Preferences** (`wrapper.html`, window `prefs`) is the app's own page; it is the only
//!   window that can call the `wrapper_*` commands below. It shows only the editable
//!   fields, and [`wrapper_set`] refuses any other field, whatever the page sends.
//! - **Downloads** go to the Downloads folder under a free name, with a notice in the page
//!   and "Show in Finder/Explorer".
//! - **Deep links** (`<scheme>://path?x`, with the `tauri_deep_link` plugin) open
//!   `<base>/path?x` on the same site, also when another launch hands them over.
//!
//! Per OS, see [`platform`]: TLS pinning, mute, the media permission prompts, key capture.
//!
//! # Example
//!
//! What the navigation hook decides:
//!
//! ```
//! use desktop_core::wrapper::{url::Verdict, WrapperConfig};
//!
//! let cfg = WrapperConfig { url: "https://pikvm.local".into(), ..WrapperConfig::default() }.check().unwrap();
//! assert_eq!(cfg.decide("https://pikvm.local/kvm/"), Verdict::Allow);
//! assert_eq!(cfg.decide("https://github.com/"), Verdict::External);
//! ```

pub mod capture;
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(any(target_os = "macos", windows)), path = "linux.rs")]
pub mod platform;

use crate::error::{codes, CommandError};
use desktop_core::wrapper::browser;
use desktop_core::wrapper::perm::{self, Media};
use desktop_core::wrapper::pin::{self, PinStore};
use desktop_core::wrapper::proxy::{self, Proxy, Upstream};
use desktop_core::wrapper::rules::{self, Source};
use desktop_core::wrapper::url::{self as wurl, Verdict, WebUrl};
use desktop_core::wrapper::{page, ChangeError, Checked, Editable, UserOverrides, UserWrapper, WrapperConfig, LABEL, PREFS_LABEL};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

/// The custom URL scheme (`tauri_deep_link`), as generated.
// Used by the deep-link plugin and the single-instance knock, when the app has them.
#[allow(dead_code)]
pub const SCHEME: &str = "{% if deep_link_scheme and deep_link_scheme != "" %}{{deep_link_scheme}}{% else %}{{project-name}}{% endif %}";

/// What the per-OS hooks need when the window is made.
pub struct Hooks {
    /// The site's origin (TLS errors are only ever accepted for it).
    pub site: desktop_core::wrapper::url::Origin,
    /// The pin, when certificate errors may be accepted for the site (direct mode).
    pub pins: Option<PinStore>,
    /// Start muted.
    pub muted: bool,
    /// Browser features to switch off (Linux sets them on the web view; elsewhere they are
    /// browser arguments or the page script).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub features: desktop_core::wrapper::WebFeatures,
    /// Header rules and the keychain service, where the web view adds headers itself
    /// (direct mode; Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub headers: Option<(Checked, String)>,
}

/// The OS permissions key capture needs, for the first-run screen in Preferences.
pub struct Permissions {
    /// Everything key capture needs is granted.
    pub granted: bool,
    /// One entry per permission: `{id, label, granted}`.
    pub items: Value,
}

/// What the window runs on, shared by the hooks (Tauri state).
pub struct Wrapper {
    checked: Mutex<Option<Checked>>,
    pins: PinStore,
    proxy: Option<Proxy>,
    muted: AtomicBool,
    /// The page zoom now (the menu changes it even when the user may not save it).
    zoom: Mutex<f64>,
    downloads: Mutex<Vec<PathBuf>>,
    last_external: Mutex<Option<Instant>>,
}

impl Wrapper {
    fn checked(&self) -> Option<Checked> {
        self.checked.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The page URL as the site knows it (through the proxy: the site's own URL).
    fn site_url(&self, url: &str) -> String {
        self.proxy.as_ref().and_then(|p| p.to_upstream(url)).unwrap_or_else(|| url.to_string())
    }

    /// Where the window goes for a URL on the site (through the proxy: the local URL).
    #[allow(dead_code)] // deep links only
    fn window_url(&self, site_url: &str) -> String {
        self.proxy.as_ref().and_then(|p| p.to_local(site_url)).unwrap_or_else(|| site_url.to_string())
    }
}

/// The developer's settings: the `wrapper` section of `config/`, compiled in. Read-only.
pub fn baked() -> WrapperConfig {
    crate::app_config::get("wrapper").unwrap_or_default()
}

/// The user's part: the overrides of the editable fields and the learned pin.
fn user(app: &AppHandle) -> UserWrapper {
    crate::load_settings(app).config.wrapper
}

/// The settings in force: [`baked`] with the user's allowed changes merged over it.
pub fn effective(app: &AppHandle) -> WrapperConfig {
    user(app).merge(&baked())
}

/// Save the user's part.
fn save_user(app: &AppHandle, u: UserWrapper) -> Result<(), String> {
    let mut s = crate::load_settings(app);
    s.config.wrapper = u;
    crate::save_settings(app, &s)
}

/// Remember a value the menu changed (zoom, mute), when the developer lets the user change it.
fn remember(app: &AppHandle, field: Editable, value: Value) {
    let baked = baked();
    if !baked.is_editable(field) {
        return;
    }
    let mut u = user(app);
    match u.overrides.change(&baked, &json!({ field.key(): value })) {
        Ok(o) => {
            u.overrides = o;
            if let Err(e) = save_user(app, u) {
                log::warn!("could not save {}: {e}", field.key());
            }
        }
        Err(e) => log::warn!("{e}"),
    }
}

/// Resolve a value spec (literal, `${ENV}`, `config:`, `keychain:`). Errors never hold the value.
pub(crate) fn resolve(identifier: &str, spec: &str) -> Result<String, String> {
    match rules::source(spec) {
        Source::Config(path) => crate::app_config::get::<String>(path).ok_or_else(|| format!("config value {path} is missing")),
        Source::Keychain(name) => desktop_core::wrapper::keychain_get(identifier, name),
        Source::Text(t) => {
            let env = rules::login_env();
            rules::interpolate(t, &|k| env.get(k).cloned())
        }
    }
}

/// `Object.freeze(Object.prototype)` for the app's own windows only.
///
/// Tauri's `freezePrototype` setting runs in every web view, the wrapped site's too, and
/// many sites' code (antd, older polyfills) assigns properties that then become read-only
/// ("Attempted to assign to readonly property", a blank page). So web apps turn the setting
/// off in `tauri.conf.json` and freeze the prototype here, in Preferences and Settings.
pub const FREEZE_PROTOTYPE: &str = "Object.freeze(Object.prototype);";

/// Open the site's window, the menu and (when keys need a permission first) Preferences.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let cfg = effective(app);
    let checked = match cfg.check() {
        Ok(c) => Some(c),
        Err(e) => {
            log::error!("web app settings are not valid: {}", e.join("; "));
            None
        }
    };
    let saver = app.clone();
    // A pin learned on first use is the user's, kept apart from the built-in settings.
    let pins = PinStore::new(cfg.insecure_tls, checked.as_ref().and_then(|c| c.pin.clone()), move |p| {
        let mut u = user(&saver);
        u.learned_pin = p.unwrap_or_default().to_string();
        if let Err(e) = save_user(&saver, u) {
            log::warn!("could not save the certificate pin: {e}");
        }
    });
    let proxy = match &checked {
        Some(c) if cfg.uses_proxy() => Some(start_proxy(app, c, &pins)?),
        _ => None,
    };
    capture::init(app, cfg.capture_keys, checked.as_ref().map(|c| c.release.clone()).unwrap_or_default());
    app.manage(Wrapper {
        checked: Mutex::new(checked.clone()),
        pins,
        proxy,
        muted: AtomicBool::new(cfg.muted),
        zoom: Mutex::new(cfg.zoom),
        downloads: Mutex::default(),
        last_external: Mutex::default(),
    });
    app.set_menu(menu(app, &cfg)?)?;
    app.on_menu_event(on_menu);
    match checked {
        Some(c) => open_window(app, &c)?,
        None => open_prefs(app, "")?,
    }
    if cfg.capture_keys && !platform::permissions().granted {
        open_prefs(app, "#permissions")?;
    }
    Ok(())
}

fn start_proxy(app: &AppHandle, c: &Checked, pins: &PinStore) -> tauri::Result<Proxy> {
    let identifier = app.config().identifier.clone();
    let rules = c.clone();
    let headers: Arc<proxy::HeaderFn> = Arc::new(move |url: &str| match rules.headers_for(url, &|s| resolve(&identifier, s)) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("{e}");
            Vec::new()
        }
    });
    let site = WebUrl::parse(&c.config.url).map_err(|e| tauri::Error::Io(std::io::Error::other(e)))?;
    let up = Upstream {
        site,
        tls: Arc::new(proxy::tls_config(c.config.insecure_tls.then(|| pins.clone()))),
        headers,
        user_agent: (!c.config.user_agent.is_empty()).then(|| c.config.user_agent.clone()),
        port: proxy::preferred_port(&app.config().identifier),
    };
    Proxy::start(up).map_err(tauri::Error::Io)
}

/// The site's window.
fn open_window(app: &AppHandle, c: &Checked) -> tauri::Result<()> {
    let state = app.state::<Wrapper>();
    let start = match &state.proxy {
        Some(p) => p.auth_url(&WebUrl::parse(&c.config.url).map(|u| u.rest).unwrap_or_else(|_| "/".into())),
        None => c.config.url.clone(),
    };
    let url = Url::parse(&start).map_err(tauri::Error::InvalidUrl)?;
    let title = app.package_info().name.clone();
    let on_nav = {
        let app = app.clone();
        move |url: &Url| navigate(&app, url)
    };
    let on_new = {
        let app = app.clone();
        move |url: Url, _features| {
            if navigate(&app, &url) {
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Some(w) = app.get_webview_window(LABEL) {
                        let _ = w.navigate(url);
                    }
                });
            }
            NewWindowResponse::Deny
        }
    };
    let on_load = {
        let app = app.clone();
        move |w: tauri::WebviewWindow, p: tauri::webview::PageLoadPayload<'_>| {
            if p.event() == PageLoadEvent::Finished {
                page_loaded(&app, &w, p.url().as_str());
            }
        }
    };
    let on_permission = {
        let app = app.clone();
        move |webview: tauri::Webview, kind: PermissionKind| {
            let media = match kind {
                PermissionKind::Microphone => Media::Microphone,
                PermissionKind::Camera => Media::Camera,
                PermissionKind::DisplayCapture => Media::Screen,
                _ => Media::Other,
            };
            let state = app.state::<Wrapper>();
            let page = webview.url().ok().map(|u| state.site_url(u.as_str()));
            let cfg = effective(&app);
            let allowed = state.checked().is_some_and(|c| perm::decide(&cfg, &c.origin, page.as_deref(), media));
            log::info!("permission {media:?}: {}", if allowed { "allowed" } else { "refused" });
            if allowed {
                PermissionResponse::Allow
            } else {
                PermissionResponse::Deny
            }
        }
    };
    let on_download = {
        let app = app.clone();
        move |_w: tauri::Webview, event: DownloadEvent<'_>| download(&app, event)
    };
    let mut b = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url.clone()))
        .title(title)
        .inner_size(1280.0, 800.0)
        .min_inner_size(480.0, 360.0)
        .on_navigation(on_nav)
        .on_new_window(on_new)
        .on_page_load(on_load)
        .on_permission_request(on_permission)
        .on_download(on_download)
        .initialization_script(page::init_script(&c.config.features))
        // Persistent cookies and storage (Windows and Linux; macOS keeps the default store).
        .data_directory(crate::paths::app_dirs().1.join("wrapper"));
    if !c.config.user_agent.is_empty() {
        b = b.user_agent(&c.config.user_agent);
    }
    if let Some(args) = platform::browser_args(&c.config.features) {
        b = b.additional_browser_args(&args);
    }
    let w = b.build()?;
    let _ = w.set_zoom(c.config.zoom);
    let hooks = Hooks {
        site: c.origin.clone(),
        pins: (c.config.insecure_tls && state.proxy.is_none()).then(|| state.pins.clone()),
        muted: state.muted.load(Ordering::SeqCst),
        features: c.config.features,
        headers: if state.proxy.is_none() { Some((c.clone(), app.config().identifier.clone())) } else { None },
    };
    let reload = hooks.pins.is_some();
    w.with_webview(move |pv| platform::install(pv, hooks))?;
    if reload {
        // The first request can reach the server before the certificate hook is in place
        // (macOS), so load the site again once it is. Runs after `install` on the UI thread.
        w.navigate(url.clone())?;
    }
    let app2 = app.clone();
    w.on_window_event(move |e| match e {
        tauri::WindowEvent::Focused(true) => {
            if effective(&app2).start_captured && capture::is_enabled() {
                if let Err(e) = capture::start() {
                    log::info!("key capture not started: {e}");
                }
            }
        }
        // Safety: keys are never held while another window has focus.
        tauri::WindowEvent::Focused(false) => capture::stop(),
        tauri::WindowEvent::Destroyed => {
            capture::stop();
            app2.exit(0);
        }
        _ => {}
    });
    log::info!("web app window opened on {}", c.origin);
    Ok(())
}

/// The navigation hook: `true` lets the window go there.
fn navigate(app: &AppHandle, url: &Url) -> bool {
    let state = app.state::<Wrapper>();
    if let Some(c) = page::control(url.as_str()) {
        let app = app.clone();
        std::thread::spawn(move || control(&app, c));
        return false;
    }
    let Some(checked) = state.checked() else { return false };
    if let Some(p) = &state.proxy {
        if p.to_upstream(url.as_str()).is_some() {
            return true;
        }
        // A link written with the site's own absolute URL: back through the proxy.
        if let Some(local) = p.to_local(url.as_str()) {
            go(app, &local);
            return false;
        }
    }
    match checked.decide(url.as_str()) {
        Verdict::Allow => state.proxy.is_none(),
        Verdict::External => {
            open_external(&state, url);
            false
        }
        Verdict::Refuse => {
            log::warn!("blocked navigation to {}", url.origin().ascii_serialization());
            false
        }
    }
}

fn go(app: &AppHandle, url: &str) {
    let (app, url) = (app.clone(), url.to_string());
    std::thread::spawn(move || {
        if let (Some(w), Ok(u)) = (app.get_webview_window(LABEL), Url::parse(&url)) {
            let _ = w.navigate(u);
        }
    });
}

/// The system browser, at most once every two seconds, http(s) only, without a shell.
fn open_external(state: &Wrapper, url: &Url) {
    let mut last = state.last_external.lock().unwrap_or_else(PoisonError::into_inner);
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) || !matches!(url.scheme(), "http" | "https") {
        return;
    }
    *last = Some(Instant::now());
    if let Err(e) = open_with_system(url.as_str(), false) {
        log::warn!("could not open the system browser: {e}");
    }
}

/// "Open in Browser…": the page the window shows (`window_url`), as the site knows it, in
/// the developer's browser app or the default one. Anything but an `http(s)` page on the
/// wrapped origin is refused.
fn open_in_browser(app: &AppHandle, window_url: &str) {
    let state = app.state::<Wrapper>();
    let Some(c) = state.checked() else { return };
    let url = match c.browser_url(&state.site_url(window_url)) {
        Ok(u) => u,
        Err(e) => {
            log::warn!("open in browser refused: {e}");
            return;
        }
    };
    let Some(l) = browser::launch(&c.config.open_in_browser_app, &url, cfg!(target_os = "macos")) else {
        if let Err(e) = open_with_system(&url, false) {
            log::warn!("could not open the default browser: {e}");
        }
        return;
    };
    // `open -a` returns at once, and fails when the app is missing: then the default
    // browser. Elsewhere the program is the browser itself, so only a failed start falls back.
    let wait = cfg!(target_os = "macos");
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new(&l.program);
        cmd.args(&l.args);
        let ok = if wait { cmd.status().map(|s| s.success()) } else { cmd.spawn().map(|_| true) };
        if !matches!(ok, Ok(true)) {
            log::warn!("could not open {}; using the default browser", l.program);
            if let Err(e) = open_with_system(&url, false) {
                log::warn!("could not open the default browser: {e}");
            }
        }
    });
}

/// `open` / `explorer` / `xdg-open` with one argument (with `reveal`: select the file).
fn open_with_system(target: &str, reveal: bool) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        if reveal {
            c.arg("-R");
        }
        c
    };
    #[cfg(windows)]
    let mut cmd = if reveal {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{target}"));
        return c.spawn().map(|_| ());
    } else {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = std::process::Command::new("xdg-open");
    #[cfg(not(any(target_os = "macos", windows)))]
    let target = if reveal { std::path::Path::new(target).parent().and_then(|p| p.to_str()).unwrap_or(target) } else { target };
    let _ = reveal;
    cmd.arg(target).spawn().map(|_| ())
}

/// A message from the page script.
fn control(app: &AppHandle, c: page::Control) {
    match c {
        page::Control::Release => capture::stop(),
        page::Control::Reveal(id) => {
            let path =
                app.state::<Wrapper>().downloads.lock().unwrap_or_else(PoisonError::into_inner).get((id as usize).wrapping_sub(1)).cloned();
            if let Some(p) = path {
                let _ = open_with_system(&p.to_string_lossy(), true);
            }
        }
    }
}

fn page_loaded(app: &AppHandle, w: &tauri::WebviewWindow, url: &str) {
    let state = app.state::<Wrapper>();
    let muted = state.muted.load(Ordering::SeqCst);
    let _ = w.eval(page::mute_script(muted));
    if capture::is_active() {
        let _ = w.eval(page::overlay_script(true, &format!("Keys captured. Press {} to release.", capture::release_text())));
        if platform::KEYS_ARRIVE_IN_PAGE {
            if let Some(c) = state.checked() {
                let _ = w.eval(page::watch_script(Some(&c.release)));
            }
        }
    }
    if effective(app).fill_on_load {
        fill(app, w, url);
    }
}

/// Fill the forms the auto-fill rules name for this page (only on the site itself).
fn fill(app: &AppHandle, w: &tauri::WebviewWindow, url: &str) {
    let state = app.state::<Wrapper>();
    let Some(c) = state.checked() else { return };
    let site_url = state.site_url(url);
    if c.decide(&site_url) != Verdict::Allow {
        return;
    }
    let id = app.config().identifier.clone();
    for (p, rule) in &c.autofill {
        if !p.matches(&site_url) {
            continue;
        }
        let mut fields = Vec::new();
        for (sel, spec) in &rule.fields {
            match resolve(&id, spec) {
                Ok(v) => fields.push((sel.clone(), v)),
                Err(e) => log::warn!("auto-fill {sel}: {e}"),
            }
        }
        let _ = w.eval(rules::fill_script(&fields));
    }
}

fn download(app: &AppHandle, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            let name = destination
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("download").to_string());
            let dir = app.path().download_dir().unwrap_or_else(|_| std::env::temp_dir());
            *destination = perm::download_path(&dir, &name);
            log::info!("download started: {}", destination.display());
            true
        }
        DownloadEvent::Finished { path, success, .. } => {
            let Some(w) = app.get_webview_window(LABEL) else { return true };
            match (success, path) {
                (true, Some(p)) => {
                    let mut list = app.state::<Wrapper>().downloads.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    list.push(p.clone());
                    *app.state::<Wrapper>().downloads.lock().unwrap_or_else(PoisonError::into_inner) = list.clone();
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let label = if cfg!(target_os = "macos") {
                        "Show in Finder"
                    } else if cfg!(windows) {
                        "Show in Explorer"
                    } else {
                        "Show in folder"
                    };
                    let _ = w.eval(page::toast_script(&format!("Downloaded {name}"), Some((list.len() as u32, label))));
                }
                _ => {
                    let _ = w.eval(page::toast_script("Download failed", None));
                }
            }
            true
        }
        _ => true,
    }
}

/// Open a deep link (`<scheme>://path?x`) on the site.
// Used by the deep-link plugin and the single-instance knock, when the app has them.
#[allow(dead_code)]
pub fn open_link(app: &AppHandle, link: &str) {
    let Some(state) = app.try_state::<Wrapper>() else { return };
    let Some(c) = state.checked() else { return };
    match wurl::map_deep_link(SCHEME, &c.base, link) {
        Ok(target) => {
            let local = state.window_url(&target);
            go(app, &local);
            if let Some(w) = app.get_webview_window(LABEL) {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        Err(e) => log::warn!("deep link refused: {e}"),
    }
}

// ---- menu ----

const M_RELOAD: &str = "wrap-reload";
const M_BACK: &str = "wrap-back";
const M_FORWARD: &str = "wrap-forward";
const M_ZOOM_IN: &str = "wrap-zoom-in";
const M_ZOOM_OUT: &str = "wrap-zoom-out";
const M_ZOOM_RESET: &str = "wrap-zoom-reset";
const M_CAPTURE: &str = "wrap-capture";
const M_MUTE: &str = "wrap-mute";
const M_FILL: &str = "wrap-fill";
const M_OPEN_BROWSER: &str = "wrap-open-browser";
const M_PREFS: &str = "wrap-prefs";

/// A menu text: its translation when the app has translations (feature `i18n`, ids in
/// `locales/*/app.ftl`), else the English.
fn text(id: &str, en: &str) -> String {
    #[cfg(feature = "i18n")]
    {
        let t = desktop_core::i18n::tr(id, None);
        if t != id {
            return t;
        }
    }
    let _ = id;
    en.to_string()
}

fn menu(app: &AppHandle, cfg: &WrapperConfig) -> tauri::Result<Menu<tauri::Wry>> {
    let item = |id: &str, label: &str, keys: Option<&str>| MenuItem::with_id(app, id, label, true, keys);
    let name = app.package_info().name.clone();
    let app_menu = Submenu::with_items(
        app,
        &name,
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &item(M_PREFS, &text("wrapper-menu-preferences", "Preferences…"), Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        text("wrapper-menu-edit", "Edit"),
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        text("wrapper-menu-view", "View"),
        true,
        &[
            &item(M_RELOAD, &text("wrapper-menu-reload", "Reload"), Some("CmdOrCtrl+R"))?,
            &item(M_BACK, &text("wrapper-menu-back", "Back"), Some("CmdOrCtrl+["))?,
            &item(M_FORWARD, &text("wrapper-menu-forward", "Forward"), Some("CmdOrCtrl+]"))?,
            &PredefinedMenuItem::separator(app)?,
            &item(M_ZOOM_IN, &text("wrapper-menu-zoom-in", "Zoom In"), Some("CmdOrCtrl+="))?,
            &item(M_ZOOM_OUT, &text("wrapper-menu-zoom-out", "Zoom Out"), Some("CmdOrCtrl+-"))?,
            &item(M_ZOOM_RESET, &text("wrapper-menu-actual-size", "Actual Size"), Some("CmdOrCtrl+0"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let page_menu = Submenu::with_items(
        app,
        text("wrapper-menu-page", "Page"),
        true,
        &[
            &MenuItem::with_id(app, M_CAPTURE, text("wrapper-menu-capture", "Toggle Key Capture"), cfg.capture_keys, None::<&str>)?,
            &item(M_MUTE, &text("wrapper-menu-mute", "Mute / Unmute"), Some("CmdOrCtrl+Shift+M"))?,
            &MenuItem::with_id(app, M_FILL, text("wrapper-menu-fill", "Fill Form"), !cfg.autofill.is_empty(), None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &item(M_OPEN_BROWSER, &text("wrapper-menu-open-in-browser", "Open in Browser…"), Some("CmdOrCtrl+Shift+O"))?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &view, &page_menu])
}

fn on_menu(app: &AppHandle, e: MenuEvent) {
    let Some(w) = app.get_webview_window(LABEL) else {
        if e.id() == M_PREFS {
            let _ = open_prefs(app, "");
        }
        return;
    };
    let state = app.state::<Wrapper>();
    let zoom = |delta: Option<f64>| {
        let z = {
            let mut z = state.zoom.lock().unwrap_or_else(PoisonError::into_inner);
            *z = delta.map_or(1.0, |d| ((*z + d) * 100.0).round() / 100.0).clamp(0.25, 5.0);
            *z
        };
        let _ = w.set_zoom(z);
        remember(app, Editable::Zoom, z.into());
    };
    match e.id().as_ref() {
        M_RELOAD => {
            let _ = w.reload();
        }
        M_BACK => {
            let _ = w.eval("history.back()");
        }
        M_FORWARD => {
            let _ = w.eval("history.forward()");
        }
        M_ZOOM_IN => zoom(Some(0.1)),
        M_ZOOM_OUT => zoom(Some(-0.1)),
        M_ZOOM_RESET => zoom(None),
        M_CAPTURE => {
            if let Err(e) = capture::toggle() {
                log::warn!("key capture: {e}");
                let _ = w.eval(page::toast_script(&e, None));
                if !platform::permissions().granted {
                    let _ = open_prefs(app, "#permissions");
                }
            }
        }
        M_MUTE => set_muted(app, !state.muted.load(Ordering::SeqCst)),
        M_FILL => fill(app, &w, w.url().map(|u| u.to_string()).unwrap_or_default().as_str()),
        M_OPEN_BROWSER => open_in_browser(app, w.url().map(|u| u.to_string()).unwrap_or_default().as_str()),
        M_PREFS => {
            let _ = open_prefs(app, "");
        }
        _ => {}
    }
}

/// Mute or unmute the page, and remember it.
fn set_muted(app: &AppHandle, muted: bool) {
    let state = app.state::<Wrapper>();
    state.muted.store(muted, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.with_webview(move |pv| platform::set_muted(pv, muted));
        let _ = w.eval(page::mute_script(muted));
    }
    remember(app, Editable::Muted, muted.into());
}

/// Open Preferences (or bring it to the front); `anchor` picks a section (`#permissions`).
pub fn open_prefs(app: &AppHandle, anchor: &str) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(PREFS_LABEL) {
        let _ = w.show();
        return w.set_focus();
    }
    WebviewWindowBuilder::new(app, PREFS_LABEL, WebviewUrl::App(format!("wrapper.html{anchor}").into()))
        .title("Preferences")
        .inner_size(620.0, 720.0)
        // The app's own page keeps the frozen prototype (see `FREEZE_PROTOTYPE`).
        .initialization_script(FREEZE_PROTOTYPE)
        .build()
        .map(|_| ())
}

// ---- commands (Preferences only: `web-*` windows are refused before any command runs) ----

/// What Preferences shows: only the fields the developer lets the user change
/// (`editable`, `values`), the site (read-only), the pin and whether it can be forgotten,
/// the keychain entries the rules read, capture, and the OS permissions key capture needs.
/// Header values, auto-fill values and the other built-in settings are never sent.
#[tauri::command]
pub fn wrapper_get(app: AppHandle) -> Result<Value, CommandError> {
    let baked = baked();
    let cfg = user(&app).merge(&baked);
    let state = app.state::<Wrapper>();
    let p = platform::permissions();
    Ok(json!({
        "site": cfg.url,
        "editable": cfg.editable().map(Editable::key).collect::<Vec<_>>(),
        "values": UserOverrides::shown(&cfg),
        "insecureTls": cfg.insecure_tls,
        "pin": state.pins.get().map(|f| pin::display(&f)),
        "canForgetPin": baked.learns_pin(),
        "secrets": baked.keychain_names(),
        "proxy": state.proxy.as_ref().map(|p| json!({"port": p.port()})),
        "capture": {"enabled": capture::is_enabled(), "active": capture::is_active()},
        "permissions": {"granted": p.granted, "items": p.items, "platform": std::env::consts::OS},
        "problems": cfg.check().err().unwrap_or_default(),
    }))
}

/// Save the user's changes from Preferences: `changes` is `{field: value}` with only
/// fields on the developer's `userEditable` list (`null` goes back to the built-in value).
/// Any other field is refused (`denied`) and nothing is saved. Returns whether a restart
/// is needed (turning key capture on or off changes the menu).
#[tauri::command]
pub fn wrapper_set(app: AppHandle, changes: Value) -> Result<Value, CommandError> {
    let baked = baked();
    let old_user = user(&app);
    let old = old_user.merge(&baked);
    let (u, new, checked) = changed(&baked, &old_user, &changes)?;
    save_user(&app, u).map_err(CommandError::from)?;
    let state = app.state::<Wrapper>();
    *state.checked.lock().unwrap_or_else(PoisonError::into_inner) = Some(checked.clone());
    capture::configure(new.capture_keys, checked.release.clone());
    if new.zoom != old.zoom {
        *state.zoom.lock().unwrap_or_else(PoisonError::into_inner) = new.zoom;
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.set_zoom(new.zoom);
        }
    }
    if new.muted != state.muted.load(Ordering::SeqCst) {
        set_muted(&app, new.muted);
    }
    Ok(json!({ "restart": new.capture_keys != old.capture_keys }))
}

/// The user's part with `changes` applied, the settings then in force, and their check:
/// a field the developer did not allow is `denied`, a bad value `invalid`; nothing is saved.
fn changed(baked: &WrapperConfig, user: &UserWrapper, changes: &Value) -> Result<(UserWrapper, WrapperConfig, Checked), CommandError> {
    let mut u = user.clone();
    u.overrides = u.overrides.change(baked, changes).map_err(|e| match e {
        ChangeError::NotEditable(_) => CommandError::new(codes::DENIED, e.to_string()),
        ChangeError::Invalid(_) => CommandError::invalid(e.to_string()),
    })?;
    let new = u.try_merge(baked).map_err(|e| CommandError::new(codes::INVALID, e.join("; ")))?;
    let checked = new.check().map_err(|e| CommandError::new(codes::INVALID, e.join("; ")))?;
    Ok((u, new, checked))
}

/// Forget the certificate pinned on first use: the next one the site shows is pinned (after
/// a reload). Only with `insecureTls` on and no pin chosen by the developer.
#[tauri::command]
pub fn wrapper_forget_pin(app: AppHandle) -> Result<(), CommandError> {
    if !baked().learns_pin() {
        return Err(CommandError::new(codes::DENIED, "this app's certificate pin is set by its developer"));
    }
    app.state::<Wrapper>().pins.forget();
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.reload();
    }
    Ok(())
}

/// Store (or with an empty value delete) a secret in the OS keychain, for the
/// `keychain:<name>` values of the built-in header and auto-fill rules. Only the names
/// those rules read are accepted.
#[tauri::command]
pub fn wrapper_set_secret(app: AppHandle, name: String, value: Option<String>) -> Result<(), CommandError> {
    if !baked().keychain_names().contains(&name) {
        return Err(CommandError::new(codes::DENIED, "no rule of this app reads that keychain entry"));
    }
    let value = value.filter(|v| !v.is_empty());
    desktop_core::wrapper::keychain_set(&app.config().identifier, &name, value.as_deref()).map_err(CommandError::from)
}

/// Open the OS settings for a permission (`accessibility`, `input`, `screen`, `microphone`,
/// `camera`). macOS only; elsewhere nothing happens.
#[tauri::command]
pub fn wrapper_open_privacy(pane: String) -> Result<(), CommandError> {
    platform::open_privacy(&pane).map_err(|e| CommandError::from(e.to_string()))
}

/// Restart the app (for settings that apply at start).
#[tauri::command]
pub fn wrapper_restart(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scheme_is_a_url_scheme() {
        assert!(!SCHEME.is_empty());
        assert!(SCHEME.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)));
        let cfg = WrapperConfig { url: "https://pikvm.local/kvm".into(), ..WrapperConfig::default() }.check().unwrap();
        assert_eq!(wurl::map_deep_link(SCHEME, &cfg.base, &format!("{SCHEME}://x?y=1")).unwrap(), "https://pikvm.local/kvm/x?y=1");
    }

    #[test]
    fn preferences_can_change_only_what_the_developer_allows() {
        let baked = WrapperConfig { url: "https://pikvm.local/".into(), insecure_tls: true, ..WrapperConfig::default() };
        let user = UserWrapper::default();
        // Fields the UI might send that are the developer's: refused, nothing changes.
        for changes in [
            json!({"url": "https://evil.example/"}),
            json!({"insecureTls": false}),
            json!({"headers": [{"pattern": "*", "name": "X-A", "value": "1"}]}),
            json!({"userAgent": "x"}),
            json!({"microphone": true}),
            json!({"zoom": 1.2, "pinnedSha256": "ab"}),
            json!({"userEditable": ["url"]}),
            json!({"openInBrowserApp": "Safari"}),
        ] {
            let e = changed(&baked, &user, &changes).unwrap_err();
            assert_eq!(e.code, codes::DENIED, "{changes}: {e:?}");
        }
        let e = changed(&baked, &user, &json!({"zoom": 50.0})).unwrap_err();
        assert_eq!(e.code, codes::INVALID);
        let (u, new, checked) = changed(&baked, &user, &json!({"zoom": 1.5, "muted": true, "releaseShortcut": "Ctrl+Alt+F12"})).unwrap();
        assert_eq!((new.zoom, new.muted, new.url.as_str()), (1.5, true, "https://pikvm.local/"));
        assert_eq!(checked.release.to_string(), "Ctrl+Alt+F12");
        assert_eq!(u.overrides.zoom, Some(1.5));
        assert!(baked.learns_pin(), "Forget is offered: insecureTls with no built-in pin");
    }

    #[test]
    fn values_resolve_from_the_environment() {
        std::env::set_var("KITCHEN_WRAPPER_TEST", "v");
        // login_env is read once; the variable is set before its first use in this process.
        assert!(resolve("com.example.test", "${KITCHEN_WRAPPER_TEST_MISSING}").unwrap_err().contains("is not set"));
        assert!(resolve("com.example.test", "config:no.such.key").is_err());
        assert_eq!(resolve("com.example.test", "plain").unwrap(), "plain");
    }
}
