//! URLs for the web app wrapper: a strict parser, the origin, the navigation guard and
//! deep links.
//!
//! The parser accepts only what a wrapped site needs (`http`/`https`, a host, an optional
//! port) and refuses the tricks that make two URLs look alike: user info
//! (`https://pikvm.local@evil.com`), backslashes, control characters and spaces.
//!
//! ```
//! use desktop_core::wrapper::url::{decide, map_deep_link, Origin, Verdict, WebUrl};
//!
//! let site = WebUrl::parse("https://pikvm.local/kvm/")?;
//! let origin: Origin = site.origin();
//! assert_eq!(decide(&origin, "https://pikvm.local/api/streamer"), Verdict::Allow);
//! assert_eq!(decide(&origin, "https://pikvm.local:444/"), Verdict::External); // another port is another origin
//! assert_eq!(decide(&origin, "https://pikvm.local@evil.com/"), Verdict::Refuse);
//!
//! // `kvm://hid/keys?x=1` opens <base>/hid/keys?x=1, and never another origin.
//! assert_eq!(map_deep_link("kvm", &site.base(), "kvm://hid/keys?x=1")?, "https://pikvm.local/kvm/hid/keys?x=1");
//! assert!(map_deep_link("kvm", &site.base(), "kvm://@evil.com").is_err());
//! # Ok::<(), String>(())
//! ```

use std::fmt;

/// What the guard does with a navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Show it in the window.
    Allow,
    /// Open it in the system browser.
    External,
    /// Drop it.
    Refuse,
}

/// Scheme, host and port: what makes two pages the same site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// `https` or `http`.
    pub scheme: String,
    /// Lower case, IPv6 in brackets.
    pub host: String,
    /// The port, the default one filled in.
    pub port: u16,
}

impl Origin {
    /// The default port of the scheme.
    fn default_port(&self) -> bool {
        (self.scheme == "https" && self.port == 443) || (self.scheme == "http" && self.port == 80)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.default_port() {
            write!(f, "{}://{}", self.scheme, self.host)
        } else {
            write!(f, "{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}

/// A parsed `http(s)` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUrl {
    /// `https` or `http`.
    pub scheme: String,
    /// Lower case.
    pub host: String,
    /// The port, the default one filled in.
    pub port: u16,
    /// Path, query and fragment, starting with `/`.
    pub rest: String,
}

impl WebUrl {
    /// Parse `s`, refusing anything ambiguous.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty".into());
        }
        if s.chars().any(|c| c.is_control() || c == ' ' || c == '\\') {
            return Err(format!("{s:?} has spaces, control characters or backslashes"));
        }
        let (scheme, after) = s.split_once("://").ok_or_else(|| format!("{s:?} is not an http(s) URL"))?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "https" && scheme != "http" {
            return Err(format!("{scheme}: only http and https"));
        }
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let (authority, rest) = after.split_at(end);
        if authority.contains('@') {
            return Err("user names in URLs are not allowed".into());
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, tail) = v6.split_once(']').ok_or("unclosed [ in the host")?;
            if h.is_empty() || !h.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
                return Err("bad IPv6 address".into());
            }
            (format!("[{}]", h.to_ascii_lowercase()), tail.strip_prefix(':'))
        } else {
            let (h, p) = match authority.rsplit_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (authority, None),
            };
            let h = h.trim_end_matches('.').to_ascii_lowercase();
            let ok = !h.is_empty()
                && !h.starts_with(['.', '-'])
                && !h.contains("..")
                && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_');
            if !ok {
                return Err(format!("{h:?} is not a host name"));
            }
            (h, p)
        };
        let default = if scheme == "https" { 443 } else { 80 };
        let port = match port {
            None => default,
            Some(p) => p.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| format!("{p:?} is not a port"))?,
        };
        let rest = if rest.starts_with('/') { rest.to_string() } else { format!("/{rest}") };
        Ok(Self { scheme, host, port, rest })
    }

    /// The origin.
    pub fn origin(&self) -> Origin {
        Origin { scheme: self.scheme.clone(), host: self.host.clone(), port: self.port }
    }

    /// The origin plus the path (no query or fragment), without a trailing slash.
    pub fn base(&self) -> String {
        let path = self.rest.split(['?', '#']).next().unwrap_or("");
        format!("{}{}", self.origin(), path.trim_end_matches('/'))
    }

    /// Whether the host is this computer.
    pub fn is_loopback(&self) -> bool {
        is_loopback(&self.host)
    }
}

impl fmt::Display for WebUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.origin(), self.rest)
    }
}

/// `localhost`, 127.0.0.0/8 or `[::1]`.
pub fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "[::1]" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}

/// The guard: `url` stays in the window when it is on `origin`; other `http(s)` pages open
/// in the system browser; anything else (`file:`, `javascript:`, custom schemes, the app's
/// own `*.localhost` origins, ambiguous URLs) is refused. `about:blank` stays.
pub fn decide(origin: &Origin, url: &str) -> Verdict {
    if url == "about:blank" || url == "about:srcdoc" {
        return Verdict::Allow;
    }
    match WebUrl::parse(url) {
        Ok(u) if u.origin() == *origin => Verdict::Allow,
        Ok(u) if u.host.ends_with(".localhost") || u.host.ends_with(".invalid") => Verdict::Refuse,
        Ok(_) => Verdict::External,
        Err(_) => Verdict::Refuse,
    }
}

/// Turn a deep link `<scheme>://path?query#fragment` into `<base>/path?query#fragment`,
/// and check that the result is still on `base`'s origin.
pub fn map_deep_link(scheme: &str, base: &str, link: &str) -> Result<String, String> {
    let rest = link
        .strip_prefix(scheme)
        .and_then(|r| r.strip_prefix(':'))
        .filter(|_| !scheme.is_empty())
        .ok_or_else(|| format!("not a {scheme}: link"))?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    if rest.chars().any(|c| c.is_control() || c == ' ' || c == '\\' || c == '@') || rest.starts_with('/') || rest.contains("://") {
        return Err("the link's path is not allowed".into());
    }
    if rest.split(['?', '#']).next().unwrap_or("").split('/').any(|seg| seg == ".." || seg.eq_ignore_ascii_case("%2e%2e")) {
        return Err("the link's path may not go up (..)".into());
    }
    let joined = match rest {
        "" => base.to_string(),
        r if r.starts_with(['?', '#']) => format!("{base}/{r}"),
        r => format!("{base}/{r}"),
    };
    let want = WebUrl::parse(base)?.origin();
    let got = WebUrl::parse(&joined)?;
    if got.origin() != want {
        return Err("the link leads to another site".into());
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> Origin {
        WebUrl::parse("https://pikvm.local/").unwrap().origin()
    }

    #[test]
    fn parses_and_refuses_lookalikes() {
        let u = WebUrl::parse("HTTPS://PiKVM.Local.:443/a?b#c").unwrap();
        assert_eq!((u.scheme.as_str(), u.host.as_str(), u.port, u.rest.as_str()), ("https", "pikvm.local", 443, "/a?b#c"));
        assert_eq!(u.to_string(), "https://pikvm.local/a?b#c");
        assert_eq!(WebUrl::parse("https://[::1]:8443").unwrap().origin().to_string(), "https://[::1]:8443");
        assert_eq!(WebUrl::parse("https://h?x").unwrap().rest, "/?x");
        for bad in [
            "",
            "pikvm.local",
            "ftp://x",
            "https://a@b",
            "https://a b",
            "https://a\\b",
            "https://:443",
            "https://h:0",
            "https://h:99999",
            "https://-h",
            "https://a..b",
            "https://[zz]",
        ] {
            assert!(WebUrl::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_guard_keeps_one_origin() {
        let o = origin();
        assert_eq!(decide(&o, "https://pikvm.local/kvm/#x"), Verdict::Allow);
        assert_eq!(decide(&o, "https://PIKVM.local:443/"), Verdict::Allow);
        assert_eq!(decide(&o, "about:blank"), Verdict::Allow);
        assert_eq!(decide(&o, "http://pikvm.local/"), Verdict::External, "another scheme is another origin");
        assert_eq!(decide(&o, "https://pikvm.local:8443/"), Verdict::External);
        assert_eq!(decide(&o, "https://pikvm.local.evil.com/"), Verdict::External);
        assert_eq!(decide(&o, "https://docs.pikvm.org/"), Verdict::External);
        assert_eq!(decide(&o, "https://pikvm.local@evil.com/"), Verdict::Refuse);
        assert_eq!(decide(&o, "https://tauri.localhost/"), Verdict::Refuse);
        assert_eq!(decide(&o, "https://kitchen-wrapper.invalid/release"), Verdict::Refuse);
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x", "kvm://x", "blob:https://pikvm.local/1"] {
            assert_eq!(decide(&o, bad), Verdict::Refuse, "{bad}");
        }
    }

    #[test]
    fn deep_links_stay_on_the_site() {
        let base = "https://pikvm.local/kvm";
        assert_eq!(map_deep_link("kvm", base, "kvm://").unwrap(), base);
        assert_eq!(map_deep_link("kvm", base, "kvm:").unwrap(), base);
        assert_eq!(map_deep_link("kvm", base, "kvm://a/b?x=1#f").unwrap(), "https://pikvm.local/kvm/a/b?x=1#f");
        assert_eq!(map_deep_link("kvm", base, "kvm://?x=1").unwrap(), "https://pikvm.local/kvm/?x=1");
        assert_eq!(map_deep_link("kvm", base, "kvm:a").unwrap(), "https://pikvm.local/kvm/a");
        for bad in [
            "kvm://@evil.com",
            "kvm:////evil.com",
            "kvm://a/../../x",
            "kvm://a\\b",
            "kvm://https://evil.com",
            "other://a",
            "kvmx://a",
            "kvm://a%2E%2E/..",
            "kvm://a\nb",
        ] {
            assert!(map_deep_link("kvm", base, bad).is_err(), "{bad}");
        }
        assert!(map_deep_link("", base, "://x").is_err());
    }
}
