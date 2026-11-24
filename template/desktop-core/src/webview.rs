//! Policy for embedded web views: which remote pages the app may show in its own windows,
//! where their cookies and storage live, and the optional message bridge.
//!
//! This module is the pure, testable half of feature `webview`; the Tauri half (opening
//! windows, the navigation hook, the system browser) lives in `src-tauri/src/webview.rs`.
//! Nothing here touches the network.
//!
//! - **Allowlist.** [`WebviewConfig::allow`] holds host patterns: `example.com` is that host
//!   only, `*.example.com` is the domain and every subdomain (the same patterns as
//!   `net.allow`). A bare `*` or a pattern for a whole top-level domain (`*.com`) is dropped
//!   by [`WebviewConfig::validated`], so the list is always explicit. Only `https` pages are
//!   shown (plus `http` to this computer, or anywhere with `allowHttp`).
//! - **Blocked navigations** are refused ([`Blocked::Refuse`], the default) or handed to
//!   the system browser ([`Blocked::Browser`]). Other schemes (`file:`, `javascript:`,
//!   `data:`, custom ones) and the app's own origins (`*.localhost`, such as
//!   `tauri.localhost`) are always refused and never handed to the system.
//! - **Isolation.** [`Isolation::Profile`] gives every app profile its own cookie and
//!   storage jar ([`store_id`] names it on macOS); [`Isolation::Incognito`] keeps nothing
//!   on disk.
//! - **Bridge.** Off by default. With `bridge` on, pages on [`WebviewConfig::bridge_hosts`]
//!   (which must also be allowed) get `window.kitchenBridge.post(message)`; messages
//!   travel as a navigation to [`BRIDGE_URL`] that is caught and cancelled, so remote pages
//!   never get Tauri IPC.
//!
//! # Example
//!
//! ```
//! use desktop_core::webview::{Blocked, Verdict, WebviewConfig};
//!
//! let cfg = WebviewConfig {
//!     enabled: true,
//!     allow: vec!["docs.rs".into(), "*.github.com".into(), "*".into()],
//!     ..WebviewConfig::default()
//! }
//! .validated();
//! assert_eq!(cfg.allow, ["docs.rs", "*.github.com"]); // `*` is never allowed
//!
//! assert_eq!(cfg.verdict("https", "docs.rs"), Verdict::Allow);
//! assert_eq!(cfg.verdict("https", "gist.github.com"), Verdict::Allow);
//! assert_eq!(cfg.verdict("https", "evil.com"), Verdict::Refuse);
//! assert_eq!(cfg.verdict("http", "docs.rs"), Verdict::Refuse); // plain HTTP is not shown
//! assert_eq!(cfg.verdict("file", ""), Verdict::Refuse);
//!
//! let open_others = WebviewConfig { blocked: Blocked::Browser, ..cfg };
//! assert_eq!(open_others.verdict("https", "evil.com"), Verdict::External); // the system browser
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Window labels of embedded web views start with this; the app's own windows never do.
pub const LABEL_PREFIX: &str = "web-";

/// The address bridge messages are sent to. `.invalid` never resolves (RFC 2606), so a
/// message that escaped the hook could not reach any server.
pub const BRIDGE_URL: &str = "https://kitchen-bridge.invalid/post";

/// The host of [`BRIDGE_URL`].
pub const BRIDGE_HOST: &str = "kitchen-bridge.invalid";

/// The largest bridge message, in bytes of JSON.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// What happens to a navigation the allowlist does not cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Blocked {
    /// Refuse it and log it.
    #[default]
    Refuse,
    /// Open it in the system browser instead (http and https only, at most one every two
    /// seconds per window).
    Browser,
}

/// Where a web view keeps cookies, local storage and cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    /// A separate, persistent jar for each app profile (`<profile data>/webview`).
    #[default]
    Profile,
    /// Nothing is kept: every window starts signed out.
    Incognito,
}

/// The decision for one navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Show it in the web view.
    Allow,
    /// Hand it to the system browser.
    External,
    /// Drop it.
    Refuse,
}

/// Embedded web view settings (`webview` in settings). Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebviewConfig {
    /// Let the app open remote pages in its own windows.
    pub enabled: bool,
    /// Hosts that may be shown: `example.com` or `*.example.com`.
    pub allow: Vec<String>,
    /// What to do with everything else.
    pub blocked: Blocked,
    /// Cookie and storage isolation.
    pub isolation: Isolation,
    /// Show plain `http` pages on allowed hosts too (this computer is always allowed).
    pub allow_http: bool,
    /// The most web view windows open at once.
    pub max_windows: usize,
    /// Give pages on `bridge_hosts` `window.kitchenBridge` (off by default).
    pub bridge: bool,
    /// Hosts whose pages may use the bridge (same patterns; each must also be allowed).
    pub bridge_hosts: Vec<String>,
}

impl Default for WebviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow: Vec::new(),
            blocked: Blocked::Refuse,
            isolation: Isolation::Profile,
            allow_http: false,
            max_windows: 8,
            bridge: false,
            bridge_hosts: Vec::new(),
        }
    }
}

impl WebviewConfig {
    /// Patterns normalised (lower case, no trailing dot) and invalid ones dropped; the
    /// window count kept between 1 and 64; bridge hosts limited to allowed hosts.
    pub fn validated(mut self) -> Self {
        let clean = |list: &[String]| -> Vec<String> {
            let mut out: Vec<String> = list.iter().filter_map(|p| normalise(p)).collect();
            out.dedup();
            out
        };
        self.allow = clean(&self.allow);
        self.bridge_hosts = clean(&self.bridge_hosts);
        let allow = self.allow.clone();
        self.bridge_hosts.retain(|p| {
            let probe = p.strip_prefix("*.").unwrap_or(p);
            allow.iter().any(|a| host_matches(a, probe))
        });
        self.max_windows = self.max_windows.clamp(1, 64);
        self
    }

    /// Whether `host` is on the allowlist.
    pub fn allows_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.allow.iter().any(|p| host_matches(p, &host))
    }

    /// Whether a page on `host` may use the bridge.
    pub fn bridge_allowed(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.bridge && self.allows_host(&host) && self.bridge_hosts.iter().any(|p| host_matches(p, &host))
    }

    /// What to do with a navigation to `scheme://host`.
    pub fn verdict(&self, scheme: &str, host: &str) -> Verdict {
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "https" && scheme != "http" {
            return Verdict::Refuse;
        }
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        // `tauri.localhost`, `ipc.localhost`, ...: the app's own pages and IPC.
        if host.is_empty() || host == BRIDGE_HOST || host.ends_with(".localhost") {
            return Verdict::Refuse;
        }
        let plain_ok = scheme == "https" || self.allow_http || is_local(&host);
        if plain_ok && self.allows_host(&host) {
            return Verdict::Allow;
        }
        match self.blocked {
            Blocked::Browser => Verdict::External,
            Blocked::Refuse => Verdict::Refuse,
        }
    }
}

/// A clean pattern, or `None` for one that would allow too much or is not a host.
fn normalise(pattern: &str) -> Option<String> {
    let p = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = p.strip_prefix("*.").unwrap_or(&p);
    let valid = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '[' | ']' | ':')) && !host.starts_with('.') && !host.contains("..");
    // `*.com` would allow a whole top-level domain.
    let too_wide = p.starts_with("*.") && !host.contains('.');
    (valid && !too_wide).then_some(p)
}

/// `pattern` is `host` exactly, or `*.domain` for the domain and its subdomains.
fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.')),
        None => pattern == host,
    }
}

/// This computer: `localhost`, 127.0.0.0/8 or `[::1]`.
fn is_local(host: &str) -> bool {
    host == "localhost" || host == "[::1]" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}

/// A window label for `name`: `web-` plus letters, digits, `-` and `_` (at most 40).
///
/// ```
/// assert_eq!(desktop_core::webview::label("Docs"), Some("web-Docs".into()));
/// assert_eq!(desktop_core::webview::label("../main"), None);
/// ```
pub fn label(name: &str) -> Option<String> {
    let name = name.strip_prefix(LABEL_PREFIX).unwrap_or(name);
    let ok = !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| format!("{LABEL_PREFIX}{name}"))
}

/// Whether a window label belongs to an embedded web view.
pub fn is_webview_label(label: &str) -> bool {
    label.starts_with(LABEL_PREFIX)
}

/// The 16-byte data store id for a profile's web views (macOS 14+ and iOS 17+ keep a
/// separate `WKWebsiteDataStore` per id). Stable for the same app and profile.
pub fn store_id(app_identifier: &str, profile: &str) -> [u8; 16] {
    let digest = Sha256::digest(format!("{app_identifier}\0webview\0{profile}").as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    // Mark it as a version 4, variant 1 UUID so WebKit takes it as one.
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

/// Read a bridge message from the query of a [`BRIDGE_URL`] navigation (`m=<JSON>`,
/// already percent-decoded by the caller). Too large or not JSON: an error for the log.
pub fn parse_message(raw: &str) -> Result<serde_json::Value, String> {
    if raw.len() > MAX_MESSAGE {
        return Err(format!("bridge message over {MAX_MESSAGE} bytes"));
    }
    serde_json::from_str(raw).map_err(|e| format!("bridge message is not JSON: {e}"))
}

/// The script injected into pages when the bridge is on. It defines
/// `window.kitchenBridge.post(message)` only on hosts allowed to use it; replies from the
/// app arrive as `message` events whose `origin` is `"kitchen-app"`.
pub fn bridge_script(cfg: &WebviewConfig) -> String {
    let hosts = serde_json::to_string(&cfg.bridge_hosts).unwrap_or_else(|_| "[]".into());
    BRIDGE_JS.replace("__HOSTS__", &hosts).replace("__MAX__", &MAX_MESSAGE.to_string()).replace("__URL__", BRIDGE_URL)
}

/// The bridge, before its placeholders are filled in (no `format!`: the braces would clash
/// with the template engine).
const BRIDGE_JS: &str = r#"(function () {
  if (window.top !== window || window.kitchenBridge) return;
  var hosts = __HOSTS__;
  var h = location.hostname.toLowerCase();
  var ok = location.protocol === "https:" && hosts.some(function (p) {
    return p.indexOf("*.") === 0 ? h === p.slice(2) || h.slice(-(p.length - 1)) === p.slice(1) : h === p;
  });
  if (!ok) return;
  Object.defineProperty(window, "kitchenBridge", {
    value: Object.freeze({
      post: function (message) {
        var s = JSON.stringify(message);
        if (s === undefined || s.length > __MAX__) throw new Error("kitchenBridge: message missing or too large");
        location.href = "__URL__?m=" + encodeURIComponent(s);
      }
    })
  });
})();"#;

/// The script that delivers `message` from the app to a page (`message` event, origin
/// `"kitchen-app"`).
pub fn deliver_script(message: &serde_json::Value) -> String {
    // JSON is a JS expression; escaping the line separators keeps old engines happy.
    let json = message.to_string().replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    let mut js = String::from(r#"window.dispatchEvent(new MessageEvent("message", { data: "#);
    js.push_str(&json);
    js.push_str(r#", origin: "kitchen-app" }));"#);
    js
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(allow: &[&str]) -> WebviewConfig {
        WebviewConfig { enabled: true, allow: allow.iter().map(|s| s.to_string()).collect(), ..WebviewConfig::default() }.validated()
    }

    #[test]
    fn off_and_empty_by_default() {
        let c = WebviewConfig::default();
        assert!(!c.enabled && !c.bridge && c.allow.is_empty());
        assert_eq!(c.verdict("https", "example.com"), Verdict::Refuse);
    }

    #[test]
    fn allowlist_matcher() {
        let c = cfg(&["Example.COM.", "*.docs.rs"]);
        assert_eq!(c.allow, ["example.com", "*.docs.rs"]);
        // exact host
        assert!(c.allows_host("example.com"));
        assert!(c.allows_host("EXAMPLE.com."));
        assert!(!c.allows_host("www.example.com"));
        assert!(!c.allows_host("example.com.evil.net"));
        assert!(!c.allows_host("notexample.com"));
        // wildcard: the domain and its subdomains, nothing that only ends the same
        assert!(c.allows_host("docs.rs"));
        assert!(c.allows_host("a.b.docs.rs"));
        assert!(!c.allows_host("evildocs.rs"));
        assert!(!c.allows_host("docs.rs.evil.net"));
    }

    #[test]
    fn too_wide_or_broken_patterns_are_dropped() {
        let c = cfg(&["*", "*.com", "", "*.", "a..b", ".x.org", "bad host", "ok.org", "ok.org", "[::1]"]);
        assert_eq!(c.allow, ["ok.org", "[::1]"]);
    }

    #[test]
    fn schemes_and_blocked_navigations() {
        let c = WebviewConfig { blocked: Blocked::Browser, ..cfg(&["example.com", "localhost", "*.localhost"]) };
        assert_eq!(c.verdict("https", "example.com"), Verdict::Allow);
        assert_eq!(c.verdict("HTTPS", "Example.com"), Verdict::Allow);
        assert_eq!(c.verdict("https", "tauri.localhost"), Verdict::Refuse);
        assert_eq!(c.verdict("http", "ipc.localhost"), Verdict::Refuse);
        assert_eq!(c.verdict("http", "example.com"), Verdict::External);
        assert_eq!(c.verdict("http", "localhost"), Verdict::Allow);
        for s in ["file", "javascript", "data", "blob", "tauri", "ipc", "myapp"] {
            assert_eq!(c.verdict(s, "example.com"), Verdict::Refuse, "{s}");
        }
        assert_eq!(c.verdict("https", BRIDGE_HOST), Verdict::Refuse);
        let http = WebviewConfig { allow_http: true, ..c.clone() };
        assert_eq!(http.verdict("http", "example.com"), Verdict::Allow);
        let strict = WebviewConfig { blocked: Blocked::Refuse, ..c };
        assert_eq!(strict.verdict("https", "other.com"), Verdict::Refuse);
    }

    #[test]
    fn bridge_needs_switch_list_and_allowlist() {
        let mut c = cfg(&["*.example.com"]);
        c.bridge_hosts = vec!["app.example.com".into(), "other.org".into()];
        let c = c.validated();
        assert_eq!(c.bridge_hosts, ["app.example.com"]); // other.org is not allowed
        assert!(!c.bridge_allowed("app.example.com")); // bridge is off
        let c = WebviewConfig { bridge: true, ..c };
        assert!(c.bridge_allowed("app.example.com"));
        assert!(!c.bridge_allowed("www.example.com"));
        let script = bridge_script(&c);
        assert!(script.contains(r#"["app.example.com"]"#) && script.contains(BRIDGE_URL));
    }

    #[test]
    fn messages_and_labels() {
        assert_eq!(parse_message(r#"{"a":1}"#).unwrap()["a"], 1);
        assert!(parse_message("not json").is_err());
        assert!(parse_message(&"1".repeat(MAX_MESSAGE + 1)).is_err());
        assert!(deliver_script(&serde_json::json!({"t": "a\u{2028}b"})).contains("\\u2028"));
        assert_eq!(label("web-docs"), Some("web-docs".into()));
        assert_eq!(label("main"), Some("web-main".into()));
        assert_eq!(label(""), None);
        assert!(is_webview_label("web-x") && !is_webview_label("main"));
    }

    #[test]
    fn store_ids_differ_by_profile_and_are_stable() {
        let a = store_id("com.example.app", "default");
        assert_eq!(a, store_id("com.example.app", "default"));
        assert_ne!(a, store_id("com.example.app", "work"));
        assert_eq!(a[6] >> 4, 4);
    }

    #[test]
    fn settings_round_trip_in_camel_case() {
        let c: WebviewConfig = serde_json::from_str(r#"{"enabled": true, "blocked": "refuse", "isolation": "incognito", "bridgeHosts": [], "maxWindows": 0}"#).unwrap();
        let c = c.validated();
        assert_eq!((c.blocked, c.isolation, c.max_windows), (Blocked::Refuse, Isolation::Incognito, 1));
    }
}
