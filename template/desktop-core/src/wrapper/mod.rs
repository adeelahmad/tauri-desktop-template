//! The web app wrapper (feature `web-wrapper`): one remote site as a native desktop app.
//!
//! This is the pure, tested half. The Tauri half (the window, the menu, Preferences,
//! the per-OS hooks for TLS, key capture, media permissions and mute) is
//! `src-tauri/src/wrapper/`. Nothing here opens a window.
//!
//! | Part | Module |
//! |---|---|
//! | Settings (the developer's, built in), and the one place they are validated | [`WrapperConfig`], [`WrapperConfig::check`] |
//! | What the user may change (the allowlist), and the user's overrides | [`user`] |
//! | URLs: origin, navigation guard, deep links | [`url`] |
//! | Certificate pinning (trust on first use) | [`pin`] |
//! | Header rules, URL patterns, `${ENV}` values, masking, auto-fill | [`rules`] |
//! | Media permissions and download names | [`perm`] |
//! | "Open in Browser…": the current page in a real browser | [`browser`] |
//! | The script the wrapper adds to the page | [`page`] |
//! | Keeping a restored window on screen | [`screen`] |
//! | The loopback proxy (headers on every request, pinning, WebSockets) | [`proxy`] |
//!
//! Key codes and the release shortcut are in [`crate::keymap`].
//!
//! # Security model
//!
//! - The page gets **no IPC**: its window (`web-app`) is in no capability, and every command
//!   call from a `web-*` window is refused before it runs.
//! - **One origin.** Navigation stays on the wrapped origin; links to anything else open in
//!   the system browser, other schemes are refused ([`url::decide`]).
//! - **TLS errors** are ignored only for that origin, and only with `insecureTls`; even then
//!   the certificate is pinned by its SHA-256 on first use and a different one is refused
//!   ([`pin`]).
//! - Microphone, camera and screen sharing are refused unless switched on, and only the
//!   wrapped origin can get them ([`perm::decide`]).
//!
//! # Example
//!
//! ```
//! use desktop_core::wrapper::{url::Verdict, WrapperConfig};
//!
//! let cfg = WrapperConfig { url: "https://pikvm.local/".into(), insecure_tls: true, ..WrapperConfig::default() };
//! let ok = cfg.check().expect("valid");
//! assert_eq!(ok.origin.to_string(), "https://pikvm.local");
//! assert_eq!(ok.decide("https://pikvm.local/kvm/"), Verdict::Allow);
//! assert_eq!(ok.decide("https://github.com/pikvm"), Verdict::External); // the system browser
//! assert_eq!(ok.decide("file:///etc/passwd"), Verdict::Refuse);
//!
//! let bad = WrapperConfig { url: "http://example.com".into(), release_shortcut: "K".into(), ..cfg };
//! assert_eq!(bad.check().unwrap_err().len(), 2); // plain http, and a shortcut with no modifier
//! ```

pub mod browser;
pub mod page;
pub mod perm;
pub mod pin;
pub mod proxy;
pub mod rules;
pub mod screen;
pub mod url;
pub mod user;

pub use user::{ChangeError, Editable, UserOverrides, UserWrapper, DEFAULT_EDITABLE};

use crate::keymap::Shortcut;
use serde::{Deserialize, Serialize};

/// The window label of the wrapped site. It starts with `web-`, so the app's command guard
/// and navigation guard treat it as remote content.
pub const LABEL: &str = "web-app";

/// The Preferences window's label (the app's own page, `wrapper.html`).
pub const PREFS_LABEL: &str = "prefs";

/// Browser features that can be switched off for the site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct WebFeatures {
    /// `RTCPeerConnection` and friends (PiKVM's low-latency video uses WebRTC).
    pub webrtc: bool,
    /// `navigator.serviceWorker`.
    pub service_workers: bool,
    /// `WebAssembly`.
    pub wasm: bool,
}

impl Default for WebFeatures {
    fn default() -> Self {
        Self { webrtc: true, service_workers: true, wasm: true }
    }
}

/// Whether the loopback proxy ([`proxy`]) sits between the window and the site.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// On when header rules must reach every request and the web view cannot add them
    /// itself (macOS and Linux); off otherwise.
    #[default]
    Auto,
    /// Always through the proxy.
    On,
    /// Never: the window talks to the site directly.
    Off,
}

/// The wrapper's settings: the developer's choice, made before the build. They are the
/// `wrapper` section of the app's `config/settings.json` (written by `kitchen new --web`,
/// changed by editing the file or with `kitchen set`), compiled in, and read-only at run
/// time. The user can change only the fields named in [`user_editable`](Self::user_editable),
/// and those changes are kept apart ([`UserWrapper`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct WrapperConfig {
    /// The site: an `https` URL (plain `http` only to this computer).
    pub url: String,
    /// Accept a certificate the system does not trust, for this origin only, pinned by
    /// [`pinned_sha256`](Self::pinned_sha256).
    pub insecure_tls: bool,
    /// The pinned certificate's SHA-256 (hex, colons allowed); empty pins the first one seen.
    pub pinned_sha256: String,
    /// Send system keys (Cmd/Win, Cmd+Tab, Alt+Tab, media keys) to the page.
    pub capture_keys: bool,
    /// Capture as soon as the window has focus.
    pub start_captured: bool,
    /// The shortcut that releases the keys, e.g. `Ctrl+Alt+Shift+K`.
    pub release_shortcut: String,
    /// Page zoom, 0.25 to 5.
    pub zoom: f64,
    /// Let the site use the microphone.
    pub microphone: bool,
    /// Let the site use the camera.
    pub camera: bool,
    /// Let the site share the screen (`getDisplayMedia`).
    pub screen: bool,
    /// Start with the page's sound muted.
    pub muted: bool,
    /// A custom User-Agent (empty: the web view's own).
    pub user_agent: String,
    /// Browser features to switch off.
    pub features: WebFeatures,
    /// Extra request headers by URL pattern.
    pub headers: Vec<rules::HeaderRule>,
    /// Form fields to fill by URL pattern.
    pub autofill: Vec<rules::AutofillRule>,
    /// Fill forms when a page loads (else only from the menu, "Fill Form").
    pub fill_on_load: bool,
    /// The loopback proxy.
    pub proxy: ProxyMode,
    /// The browser "Open in Browser…" uses: empty (the default) for the user's default
    /// browser, else an app name, e.g. `Google Chrome` (macOS: `open -a`) or `firefox` (a
    /// program on `PATH` elsewhere). Plain app name characters only ([`browser::check_app`]).
    pub open_in_browser_app: String,
    /// The fields the user may change in Preferences (camelCase keys, see [`Editable`]);
    /// every other field stays as built. Default: [`DEFAULT_EDITABLE`]. `[]` shows no
    /// settings at all.
    pub user_editable: Vec<String>,
    /// macOS: show the app in the Dock (`true`, the default). Off, the app lives in the menu
    /// bar only (needs [`menu_bar_icon`](Self::menu_bar_icon): [`check`](Self::check)
    /// refuses both off, or the app could never be reached).
    pub dock_icon: bool,
    /// An icon in the macOS menu bar (the system tray on Windows and Linux) with Show/Hide,
    /// Release Keys, Mute, Preferences and Quit. Off by default.
    pub menu_bar_icon: bool,
    /// Start the app when the user logs in. Off by default. With the Dock icon off it
    /// starts hidden, in the menu bar.
    pub run_at_startup: bool,
    /// Restore the window's size, position, maximized and full-screen state on the next
    /// launch (moved back on screen when a display was removed). On by default.
    pub remember_window: bool,
}

impl Default for WrapperConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            insecure_tls: false,
            pinned_sha256: String::new(),
            capture_keys: false,
            start_captured: false,
            release_shortcut: crate::keymap::DEFAULT_RELEASE.into(),
            zoom: 1.0,
            microphone: false,
            camera: false,
            screen: false,
            muted: false,
            user_agent: String::new(),
            features: WebFeatures::default(),
            headers: Vec::new(),
            autofill: Vec::new(),
            fill_on_load: false,
            proxy: ProxyMode::Auto,
            open_in_browser_app: String::new(),
            user_editable: DEFAULT_EDITABLE.iter().map(|f| f.key().to_string()).collect(),
            dock_icon: true,
            menu_bar_icon: false,
            run_at_startup: false,
            remember_window: true,
        }
    }
}

/// A [`WrapperConfig`] that passed [`WrapperConfig::check`], with its parts parsed.
#[derive(Debug, Clone)]
pub struct Checked {
    /// The settings as given.
    pub config: WrapperConfig,
    /// The site's origin.
    pub origin: url::Origin,
    /// The base URL (origin plus path), without a trailing slash.
    pub base: String,
    /// The release shortcut.
    pub release: Shortcut,
    /// The pinned fingerprint, normalised (64 lower-case hex digits), if any.
    pub pin: Option<String>,
    /// Header rules with their patterns compiled.
    pub headers: Vec<(rules::Pattern, rules::HeaderRule)>,
    /// Auto-fill rules with their patterns compiled.
    pub autofill: Vec<(rules::Pattern, rules::AutofillRule)>,
}

impl WrapperConfig {
    /// Validate everything, in one place: the URL, the shortcut, the pin, zoom, the
    /// User-Agent, the "Open in Browser…" app, the Dock and menu bar icons (not both off),
    /// every header rule and auto-fill rule. All problems are listed.
    pub fn check(&self) -> Result<Checked, Vec<String>> {
        let mut errors = Vec::new();
        let parsed = url::WebUrl::parse(&self.url).map_err(|e| format!("url: {e}"));
        let parsed = match parsed {
            Ok(u) if u.scheme == "http" && !u.is_loopback() => Err("url: use https (plain http only to this computer)".to_string()),
            other => other,
        };
        let release = self.release_shortcut.parse::<Shortcut>().map_err(|e| format!("release shortcut: {e}"));
        let pin = if self.pinned_sha256.trim().is_empty() {
            Ok(None)
        } else {
            pin::normalise(&self.pinned_sha256).map(Some).map_err(|e| format!("pinned certificate: {e}"))
        };
        if !(0.25..=5.0).contains(&self.zoom) {
            errors.push(format!("zoom: {} is outside 0.25 to 5", self.zoom));
        }
        if self.user_agent.len() > 512 || self.user_agent.chars().any(char::is_control) {
            errors.push("user agent: at most 512 characters, no line breaks".into());
        }
        if !self.dock_icon && !self.menu_bar_icon {
            errors.push("dockIcon, menuBarIcon: keep at least one on, or the app cannot be reached".into());
        }
        if let Err(e) = browser::check_app(&self.open_in_browser_app) {
            errors.push(format!("openInBrowserApp: {e}"));
        }
        for key in self.user_editable.iter().filter(|k| Editable::from_key(k).is_none()) {
            let all: Vec<_> = Editable::ALL.iter().map(|f| f.key()).collect();
            errors.push(format!("userEditable: {key} cannot be made editable (only {})", all.join(", ")));
        }
        let base = parsed.as_ref().map(url::WebUrl::base).unwrap_or_default();
        let mut headers = Vec::new();
        for (i, h) in self.headers.iter().enumerate() {
            match h.check(&base) {
                Ok(p) => headers.push((p, h.clone())),
                Err(e) => errors.push(format!("header {} ({}): {e}", i + 1, h.name)),
            }
        }
        let mut autofill = Vec::new();
        for (i, a) in self.autofill.iter().enumerate() {
            match a.check(&base) {
                Ok(p) => autofill.push((p, a.clone())),
                Err(e) => errors.push(format!("auto-fill {}: {e}", i + 1)),
            }
        }
        match (parsed, release, pin) {
            (Ok(u), Ok(release), Ok(pin)) if errors.is_empty() => {
                Ok(Checked { config: self.clone(), origin: u.origin(), base, release, pin, headers, autofill })
            }
            (u, r, p) => {
                errors.extend([u.err(), r.err(), p.err()].into_iter().flatten());
                Err(errors)
            }
        }
    }

    /// Whether the developer lets the user change `field`.
    pub fn is_editable(&self, field: Editable) -> bool {
        self.user_editable.iter().any(|k| k == field.key())
    }

    /// The fields the user may change, in the developer's order (unknown names skipped).
    pub fn editable(&self) -> impl Iterator<Item = Editable> + '_ {
        let mut seen = Vec::new();
        self.user_editable.iter().filter_map(|k| Editable::from_key(k)).filter(move |f| {
            let new = !seen.contains(f);
            seen.push(*f);
            new
        })
    }

    /// Whether the certificate is pinned on first use (and can be forgotten in
    /// Preferences): `insecureTls` is on and the developer pinned no certificate. A pin the
    /// developer chose is fixed; change it in `config/` and rebuild.
    pub fn learns_pin(&self) -> bool {
        self.insecure_tls && self.pinned_sha256.trim().is_empty()
    }

    /// The keychain entries the header and auto-fill rules read (`keychain:<name>`), so
    /// Preferences can store exactly those secrets and no others.
    pub fn keychain_names(&self) -> Vec<String> {
        let specs = self.headers.iter().map(|h| h.value.as_str()).chain(self.autofill.iter().flat_map(|a| a.fields.values().map(String::as_str)));
        let mut names: Vec<String> = specs
            .filter_map(|s| match rules::source(s) {
                rules::Source::Keychain(n) if !n.is_empty() => Some(n.to_string()),
                _ => None,
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Whether the proxy is used, on this OS ([`ProxyMode::Auto`] turns it on for header
    /// rules where the web view cannot add headers to every request).
    pub fn uses_proxy(&self) -> bool {
        match self.proxy {
            ProxyMode::On => true,
            ProxyMode::Off => false,
            ProxyMode::Auto => !self.headers.is_empty() && !cfg!(windows),
        }
    }
}

impl Checked {
    /// The URL "Open in Browser…" may open for the page the window shows (the site's own
    /// URL): only an `http(s)` page on the wrapped origin ([`browser::page_url`]).
    pub fn browser_url(&self, page: &str) -> Result<String, String> {
        browser::page_url(&self.origin, page)
    }

    /// The navigation guard's decision for `url` ([`url::decide`]).
    pub fn decide(&self, url: &str) -> url::Verdict {
        url::decide(&self.origin, url)
    }

    /// The headers to add to a request for `url` (values resolved by `resolve`; an error
    /// names the header, never its value).
    pub fn headers_for(&self, url: &str, resolve: &dyn Fn(&str) -> Result<String, String>) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for (p, h) in &self.headers {
            if p.matches(url) {
                out.push((h.name.clone(), resolve(&h.value).map_err(|e| format!("header {}: {e}", h.name))?));
            }
        }
        Ok(out)
    }
}

/// A secret from the OS keychain (service: the app's identifier, account: `name`), for
/// `keychain:<name>` values.
pub fn keychain_get(service: &str, name: &str) -> Result<String, String> {
    keyring::Entry::new(service, name).and_then(|e| e.get_password()).map_err(|e| format!("keychain entry {name}: {e}"))
}

/// Store (or with `None` delete) a secret in the OS keychain.
pub fn keychain_set(service: &str, name: &str, value: Option<&str>) -> Result<(), String> {
    let entry = keyring::Entry::new(service, name).map_err(|e| e.to_string())?;
    match value {
        Some(v) => entry.set_password(v).map_err(|e| e.to_string()),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: &str) -> WrapperConfig {
        WrapperConfig { url: url.into(), ..WrapperConfig::default() }
    }

    #[test]
    fn everything_off_and_safe_by_default() {
        let c = WrapperConfig::default();
        assert!(!c.insecure_tls && !c.capture_keys && !c.microphone && !c.camera && !c.screen && !c.muted);
        assert_eq!(c.release_shortcut, "Ctrl+Alt+Shift+K");
        assert!(c.check().is_err(), "no URL yet");
        assert!(!c.uses_proxy());
    }

    #[test]
    fn check_lists_every_problem() {
        let c = WrapperConfig {
            zoom: 9.0,
            user_agent: "a\nb".into(),
            pinned_sha256: "nope".into(),
            headers: vec![rules::HeaderRule { pattern: "/api/*".into(), name: "Host".into(), value: "x".into() }],
            ..cfg("ftp://x")
        };
        let e = c.check().unwrap_err();
        assert_eq!(e.len(), 5, "{e:?}");
        assert!(e.iter().any(|m| m.starts_with("url:")));
        assert!(e.iter().any(|m| m.starts_with("zoom:")));
        assert!(e.iter().any(|m| m.contains("Host")));
    }

    #[test]
    fn http_only_to_this_computer() {
        assert!(cfg("http://127.0.0.1:8080/").check().is_ok());
        assert!(cfg("http://localhost/").check().is_ok());
        assert!(cfg("http://pikvm.local/").check().is_err());
        let ok = cfg("https://PiKVM.local:8443/kvm/").check().unwrap();
        assert_eq!(ok.base, "https://pikvm.local:8443/kvm");
        assert_eq!(ok.origin.port, 8443);
    }

    #[test]
    fn headers_apply_by_pattern_and_never_show_values_in_errors() {
        let c = WrapperConfig {
            headers: vec![
                rules::HeaderRule { pattern: "/api/*".into(), name: "X-Token".into(), value: "${TOKEN}".into() },
                rules::HeaderRule { pattern: "*".into(), name: "X-App".into(), value: "kvm".into() },
            ],
            ..cfg("https://h.example/")
        };
        let ok = c.check().unwrap();
        let env = |s: &str| rules::interpolate(s, &|k| (k == "TOKEN").then(|| "s3cret".to_string()));
        assert_eq!(
            ok.headers_for("https://h.example/api/x", &env).unwrap(),
            [("X-Token".to_string(), "s3cret".to_string()), ("X-App".into(), "kvm".into())]
        );
        assert_eq!(ok.headers_for("https://h.example/", &env).unwrap().len(), 1);
        let missing = |s: &str| rules::interpolate(s, &|_| None);
        let e = ok.headers_for("https://h.example/api/x", &missing).unwrap_err();
        assert!(e.contains("X-Token") && e.contains("TOKEN") && !e.contains("s3cret"), "{e}");
    }

    #[test]
    fn open_in_browser_app_is_checked() {
        assert_eq!(WrapperConfig::default().open_in_browser_app, "", "the default browser");
        let ok = WrapperConfig { open_in_browser_app: "Google Chrome".into(), ..cfg("https://h.example/") }.check().unwrap();
        assert_eq!(ok.browser_url("https://h.example/a?b#c").unwrap(), "https://h.example/a?b#c");
        assert!(ok.browser_url("https://other.example/").is_err());
        assert!(ok.browser_url("about:blank").is_err());
        let e = WrapperConfig { open_in_browser_app: "Chrome; rm -rf ~".into(), ..cfg("https://h.example/") }.check().unwrap_err();
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("openInBrowserApp:"), "{e:?}");
        let c: WrapperConfig = serde_json::from_str(r#"{"url":"https://a.b","openInBrowserApp":"Firefox"}"#).unwrap();
        assert_eq!(c.open_in_browser_app, "Firefox");
    }

    #[test]
    fn the_app_stays_reachable() {
        let c = WrapperConfig::default();
        assert!(c.dock_icon && !c.menu_bar_icon && !c.run_at_startup && c.remember_window);
        let menu_bar_only = WrapperConfig { dock_icon: false, menu_bar_icon: true, ..cfg("https://h.example/") };
        assert!(menu_bar_only.check().is_ok());
        let e = WrapperConfig { dock_icon: false, ..cfg("https://h.example/") }.check().unwrap_err();
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("dockIcon, menuBarIcon:"), "{e:?}");
        let c: WrapperConfig = serde_json::from_str(r#"{"url":"https://a.b","menuBarIcon":true,"runAtStartup":true,"rememberWindow":false}"#).unwrap();
        assert!(c.dock_icon && c.menu_bar_icon && c.run_at_startup && !c.remember_window);
    }

    #[test]
    fn proxy_mode() {
        let mut c = cfg("https://h.example/");
        assert!(!c.uses_proxy());
        c.proxy = ProxyMode::On;
        assert!(c.uses_proxy());
        c.proxy = ProxyMode::Auto;
        c.headers.push(rules::HeaderRule { pattern: "*".into(), name: "X-A".into(), value: "1".into() });
        assert_eq!(c.uses_proxy(), !cfg!(windows));
    }

    #[test]
    fn the_allowlist_is_checked() {
        let c = WrapperConfig { user_editable: vec!["zoom".into(), "url".into(), "zoom".into()], ..cfg("https://h.example/") };
        let e = c.check().unwrap_err();
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("userEditable: url"), "{e:?}");
        assert_eq!(c.editable().collect::<Vec<_>>(), [Editable::Zoom], "unknown and repeated names are skipped");
        assert!(c.is_editable(Editable::Zoom) && !c.is_editable(Editable::Muted));
    }

    #[test]
    fn keychain_names_come_from_the_rules() {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("#pw".to_string(), "keychain:kvm-password".to_string());
        fields.insert("#user".to_string(), "admin".to_string());
        let c = WrapperConfig {
            headers: vec![rules::HeaderRule { pattern: "*".into(), name: "X-Token".into(), value: "keychain:token".into() }],
            autofill: vec![rules::AutofillRule { pattern: "/login".into(), fields }],
            ..cfg("https://h.example/")
        };
        assert_eq!(c.keychain_names(), ["kvm-password", "token"]);
        assert!(cfg("https://h.example/").keychain_names().is_empty());
    }

    #[test]
    fn settings_round_trip_in_camel_case() {
        let c: WrapperConfig =
            serde_json::from_str(r#"{"url":"https://a.b","insecureTls":true,"captureKeys":true,"features":{"wasm":false},"proxy":"on"}"#)
                .unwrap();
        assert!(c.insecure_tls && c.capture_keys && !c.features.wasm && c.features.webrtc);
        assert_eq!(c.proxy, ProxyMode::On);
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["releaseShortcut"], "Ctrl+Alt+Shift+K");
    }
}
