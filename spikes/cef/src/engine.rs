//! The hooks on `tauri-runtime-cef`: the window, navigation, downloads, the profile, mute,
//! media permissions and the loopback proxy for a pinned self-signed site.

use crate::config::Config;
use crate::policy::{self, Media, Nav, Route};
use crate::wrapper::pin::{self, PinStore};
use crate::wrapper::proxy::{self, Proxy, Upstream};
use crate::wrapper::url::WebUrl;
use crate::APP_ID;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, Submenu};
use tauri::webview::{DownloadEvent, PermissionKind, PermissionResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_runtime_cef::cef::{ContentSettingTypes, ContentSettingValues, ImplBrowser, ImplBrowserHost};
use tauri_runtime_cef::{Cef, CefRuntime, CertificateErrorPolicy, WebviewCefExt};

/// The app on the CEF runtime, statically dispatched.
type R = CefRuntime;

/// The window's label.
const LABEL: &str = "site";
/// The mute menu item's id.
const MUTE_ID: &str = "mute";

/// Shared state.
struct State {
    route: Route,
    muted: AtomicBool,
    /// Kept alive for the app's lifetime (dropping it stops the proxy).
    _proxy: Option<Proxy>,
}

/// Start CEF and the window, and run until the window closes.
pub fn run(cfg: Config) -> tauri::Result<()> {
    let profile = profile_dir();
    log::info!("profile: {}", profile.display());

    let cef = Cef::default()
        // A persistent Chromium profile of our own (cookies, local storage, IndexedDB,
        // granted permissions), and session cookies kept too so sign-ins survive restarts.
        .root_cache_path(&profile)
        .persist_session_cookies(true)
        // Pinned sites go through the proxy (plain http on 127.0.0.1): a certificate error in
        // the window itself is never one to click through.
        .certificate_errors(CertificateErrorPolicy::Cancel)
        .default_content_setting(ContentSettingTypes::NOTIFICATIONS, ContentSettingValues::BLOCK)
        .default_content_setting(ContentSettingTypes::GEOLOCATION, ContentSettingValues::BLOCK)
        .default_content_setting(ContentSettingTypes::USB_GUARD, ContentSettingValues::BLOCK)
        .default_content_setting(ContentSettingTypes::SERIAL_GUARD, ContentSettingValues::BLOCK)
        .default_content_setting(ContentSettingTypes::HID_GUARD, ContentSettingValues::BLOCK);
    let cef = match &cfg.user_agent {
        // Through the proxy the proxy sends it; direct, Chromium does.
        Some(ua) if !cfg.insecure_tls => cef.user_agent(ua.clone()),
        _ => cef,
    };

    tauri::Builder::<R>::new()
        .runtime(cef)
        .setup(move |app| {
            setup(app.handle(), &cfg, &profile)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == MUTE_ID {
                toggle_mute(app);
            }
        })
        .run(tauri::generate_context!())
}

/// `~/Library/Application Support/<id>/cef` (macOS), `%LOCALAPPDATA%\<id>\cef` (Windows),
/// `$XDG_DATA_HOME/<id>/cef` or `~/.local/share/<id>/cef` (Linux). Not the cache folder:
/// macOS may purge `~/Library/Caches`, and this holds the sign-ins.
fn profile_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let base = if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(home)
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
    };
    base.join(APP_ID).join("cef")
}

fn setup(app: &AppHandle<R>, cfg: &Config, profile: &Path) -> tauri::Result<()> {
    let (proxy, start) = if cfg.insecure_tls {
        let p = start_proxy(cfg, profile)?;
        let start = p.auth_url(&cfg.site.rest);
        (Some(p), start)
    } else {
        (None, cfg.site.to_string())
    };
    let local = proxy.as_ref().map(|p| WebUrl::parse(&p.local_origin()).map(|u| u.origin())).transpose().map_err(io_err)?;
    let route = Route { site: cfg.site.origin(), local };
    app.manage(State { route, muted: AtomicBool::new(cfg.muted), _proxy: proxy });
    log::info!("opening {}", cfg.site);

    let on_nav = {
        let app = app.clone();
        move |url: &Url| navigation(&app, url)
    };
    let on_permission = {
        let media = cfg.media;
        move |webview: tauri::Webview<R>, kind: PermissionKind| {
            let what = match kind {
                PermissionKind::Camera => Media::Camera,
                PermissionKind::Microphone => Media::Microphone,
                PermissionKind::DisplayCapture => Media::Screen,
                // Geolocation and notifications are blocked by the content settings too.
                PermissionKind::Geolocation | PermissionKind::Notifications | PermissionKind::Midi => return PermissionResponse::Deny,
                _ => return PermissionResponse::Default,
            };
            let state = webview.state::<State>();
            let page = webview.url().ok().map(|u| u.to_string());
            let allowed = policy::media_allowed(media, &state.route, page.as_deref(), what);
            log::info!("permission {what:?} for {}: {}", page.as_deref().unwrap_or("?"), if allowed { "allowed" } else { "refused" });
            if allowed {
                PermissionResponse::Allow
            } else {
                PermissionResponse::Deny
            }
        }
    };
    let on_download = {
        let dir =
            app.path().download_dir().unwrap_or_else(|_| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads"));
        move |_w: tauri::Webview<R>, event: DownloadEvent<'_>| download(&dir, event)
    };

    let mute = CheckMenuItem::with_id(app, MUTE_ID, "Mute Audio", true, cfg.muted, Some("CmdOrCtrl+Shift+M"))?;
    let menu = Menu::default(app)?;
    menu.append(&Submenu::with_items(app, "Audio", true, &[&mute])?)?;

    let url = Url::parse(&start).map_err(io_err)?;
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title(cfg.site.host.clone())
        .inner_size(1280.0, 800.0)
        .menu(menu)
        .on_navigation(on_nav)
        .on_permission_request(on_permission)
        .on_download(on_download)
        .build()?;
    if cfg.muted {
        set_muted(&window, true);
    }
    Ok(())
}

/// The loopback proxy in front of a site with a self-signed certificate: Chromium talks plain
/// http to 127.0.0.1, the proxy talks TLS to the site and accepts only the pinned
/// certificate (trust on first use without `--pin`). The pin is kept next to the profile.
fn start_proxy(cfg: &Config, profile: &Path) -> tauri::Result<Proxy> {
    let pin_file = profile.with_file_name("pin.txt");
    let saved = std::fs::read_to_string(&pin_file).ok().and_then(|s| pin::normalise(s.trim()).ok());
    if let (Some(want), Some(have)) = (&cfg.pin, &saved) {
        if want != have {
            log::warn!("--pin differs from the saved pin; using --pin");
        }
    }
    let save = {
        let f = pin_file.clone();
        move |fp: Option<&str>| {
            let r = match fp {
                Some(fp) => std::fs::create_dir_all(f.parent().unwrap_or(Path::new("."))).and_then(|_| std::fs::write(&f, fp)),
                None => std::fs::remove_file(&f),
            };
            match (r, fp) {
                (Ok(()), Some(fp)) => log::info!("pinned {}", pin::display(fp)),
                (Ok(()), None) => log::info!("pin forgotten"),
                (Err(e), _) => log::warn!("could not save the pin to {}: {e}", f.display()),
            }
        }
    };
    let store = PinStore::new(true, cfg.pin.clone().or(saved), save);
    let up = Upstream {
        site: cfg.site.clone(),
        tls: Arc::new(proxy::tls_config(Some(store))),
        headers: Arc::new(|_url: &str| Vec::new()),
        user_agent: cfg.user_agent.clone(),
        // The same port on every launch: the profile's cookies belong to 127.0.0.1:<port>.
        port: proxy::preferred_port(APP_ID),
    };
    Ok(Proxy::start(up)?)
}

/// `on_navigation`: `true` loads the URL in the window.
fn navigation(app: &AppHandle<R>, url: &Url) -> bool {
    let state = app.state::<State>();
    match state.route.navigation(url.as_str()) {
        Nav::Stay => true,
        Nav::Refuse => {
            log::warn!("refused navigation to {url}");
            false
        }
        Nav::Browser(target) => {
            log::info!("opening {target} in the default browser");
            if let Err(e) = open_in_browser(&target) {
                log::warn!("could not open {target}: {e}");
            }
            false
        }
        Nav::Redirect(local) => {
            // Not from inside the callback: navigate once CEF has cancelled this one.
            let app = app.clone();
            std::thread::spawn(move || {
                if let (Some(w), Ok(u)) = (app.get_webview_window(LABEL), Url::parse(&local)) {
                    let _ = w.navigate(u);
                }
            });
            false
        }
    }
}

/// The OS's default browser (`open` / `xdg-open` / `explorer`), no plugin needed.
fn open_in_browser(url: &str) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url).spawn().map(drop)
}

/// `on_download`: every download goes to the downloads folder under a fresh name, with no
/// save dialog (the runtime shows one only when the destination is left unchanged).
fn download(dir: &Path, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            let name = policy::download_name(destination, url.path());
            let path = policy::unique_path(dir, &name, &|p| p.exists());
            log::info!("download {url} -> {}", path.display());
            *destination = path;
            true
        }
        DownloadEvent::Finished { url, path, success } => {
            log::info!("download {url} {}: {:?}", if success { "finished" } else { "failed" }, path);
            true
        }
        _ => true,
    }
}

fn toggle_mute(app: &AppHandle<R>) {
    let state = app.state::<State>();
    let muted = !state.muted.load(Ordering::SeqCst);
    state.muted.store(muted, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window(LABEL) {
        set_muted(&w, muted);
    }
}

/// Mute through CEF's browser host (`CefBrowserHost::SetAudioMuted`), for the whole page
/// including iframes and WebRTC: no script and no private API.
fn set_muted(w: &tauri::WebviewWindow<R>, muted: bool) {
    let r = w.with_cef_webview(move |cw| {
        if let Some(host) = cw.browser().host() {
            host.set_audio_muted(i32::from(muted));
        }
    });
    match r {
        Ok(()) => log::info!("{}", if muted { "muted" } else { "unmuted" }),
        Err(e) => log::warn!("mute: {e}"),
    }
}

fn io_err(e: impl std::fmt::Display) -> tauri::Error {
    tauri::Error::Io(std::io::Error::other(e.to_string()))
}
