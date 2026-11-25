//! The wrapper's decisions, free of any engine type so they are tested without CEF.

use crate::config::MediaPolicy;
use crate::wrapper::url::{self as wurl, Origin, Verdict, WebUrl};
use std::path::{Path, PathBuf};

/// What to do with a navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nav {
    /// Load it in the window.
    Stay,
    /// Cancel it and load this URL in the window instead (the site's own absolute URL,
    /// rewritten onto the loopback proxy).
    Redirect(String),
    /// Cancel it and open it in the default browser.
    Browser(String),
    /// Cancel it.
    Refuse,
}

/// Where the window's pages live: the site itself, or the loopback proxy in front of it.
#[derive(Debug, Clone)]
pub struct Route {
    /// The site's origin.
    pub site: Origin,
    /// `http://127.0.0.1:<port>` when the site goes through the proxy.
    pub local: Option<Origin>,
}

impl Route {
    /// The origin the window's pages are on.
    pub fn window_origin(&self) -> &Origin {
        self.local.as_ref().unwrap_or(&self.site)
    }

    /// Same origin stays, other `http(s)` origins go to the default browser, anything else
    /// (`file:`, custom schemes, `*.localhost`) is refused.
    pub fn navigation(&self, url: &str) -> Nav {
        match wurl::decide(self.window_origin(), url) {
            Verdict::Allow => Nav::Stay,
            Verdict::Refuse => Nav::Refuse,
            Verdict::External => match (&self.local, WebUrl::parse(url)) {
                // A link written with the site's own absolute URL: back through the proxy.
                (Some(local), Ok(u)) if u.origin() == self.site => Nav::Redirect(format!("{local}{}", u.rest)),
                // Never hand the proxy's own URLs (they carry the session) to a browser.
                (_, Ok(u)) if u.is_loopback() => Nav::Refuse,
                _ => Nav::Browser(url.to_string()),
            },
        }
    }
}

/// The media a permission request is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    /// Camera.
    Camera,
    /// Microphone.
    Microphone,
    /// Screen or window capture.
    Screen,
}

/// Allow `media` when the page asking is on the window's origin and the policy allows it.
pub fn media_allowed(policy: MediaPolicy, route: &Route, page_url: Option<&str>, media: Media) -> bool {
    let on_site = page_url.and_then(|u| WebUrl::parse(u).ok()).is_some_and(|u| u.origin() == *route.window_origin());
    on_site
        && match media {
            Media::Camera => policy.camera,
            Media::Microphone => policy.microphone,
            Media::Screen => policy.screen,
        }
}

/// A file name that is safe to create in the downloads folder: the last path component of
/// `suggested` (or of the URL's path), without separators, control characters or a leading dot.
pub fn download_name(suggested: &Path, url_path: &str) -> String {
    let from_suggested = suggested.file_name().map(|n| n.to_string_lossy().into_owned());
    let from_url = url_path.split(['?', '#']).next().unwrap_or("").rsplit('/').next().map(str::to_string);
    let raw = from_suggested.filter(|n| !n.trim().is_empty()).or(from_url).unwrap_or_default();
    let clean: String = raw
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() {
        "download".into()
    } else {
        clean.chars().take(200).collect()
    }
}

/// `dir/name`, or `dir/name (1).ext`, `(2)`, ... when that already exists.
pub fn unique_path(dir: &Path, name: &str, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..10_000).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !exists(p)).unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(proxied: bool) -> Route {
        Route {
            site: WebUrl::parse("https://kvm.example:8443/").unwrap().origin(),
            local: proxied.then(|| WebUrl::parse("http://127.0.0.1:23456/").unwrap().origin()),
        }
    }

    #[test]
    fn navigation_direct() {
        let r = route(false);
        assert_eq!(r.navigation("https://kvm.example:8443/a?b"), Nav::Stay);
        assert_eq!(r.navigation("about:blank"), Nav::Stay);
        assert_eq!(r.navigation("https://kvm.example/"), Nav::Browser("https://kvm.example/".into()));
        assert_eq!(r.navigation("https://docs.example/x"), Nav::Browser("https://docs.example/x".into()));
        assert_eq!(r.navigation("file:///etc/passwd"), Nav::Refuse);
        assert_eq!(r.navigation("tauri://localhost/"), Nav::Refuse);
    }

    #[test]
    fn navigation_through_the_proxy() {
        let r = route(true);
        assert_eq!(r.navigation("http://127.0.0.1:23456/console"), Nav::Stay);
        assert_eq!(r.navigation("https://kvm.example:8443/console#x"), Nav::Redirect("http://127.0.0.1:23456/console#x".into()));
        assert_eq!(r.navigation("https://other.example/"), Nav::Browser("https://other.example/".into()));
        assert_eq!(r.navigation("http://127.0.0.1:9999/"), Nav::Refuse);
    }

    #[test]
    fn media() {
        let r = route(false);
        let p = MediaPolicy { camera: true, microphone: false, screen: false };
        assert!(media_allowed(p, &r, Some("https://kvm.example:8443/call"), Media::Camera));
        assert!(!media_allowed(p, &r, Some("https://kvm.example:8443/call"), Media::Microphone));
        assert!(!media_allowed(p, &r, Some("https://evil.example/"), Media::Camera));
        assert!(!media_allowed(p, &r, None, Media::Camera));
        let r = route(true);
        assert!(media_allowed(p, &r, Some("http://127.0.0.1:23456/"), Media::Camera));
        assert!(!media_allowed(p, &r, Some("https://kvm.example:8443/"), Media::Camera));
    }

    #[test]
    fn download_names() {
        assert_eq!(download_name(Path::new("report.pdf"), "/x"), "report.pdf");
        assert_eq!(download_name(Path::new("../../.bashrc"), "/x"), "bashrc");
        assert_eq!(download_name(Path::new(""), "/files/a%20b.zip?sig=1"), "a%20b.zip");
        assert_eq!(download_name(Path::new(""), "/"), "download");
        assert_eq!(download_name(Path::new("a:b|c"), "/"), "a_b_c");
    }

    #[test]
    fn unique_paths() {
        let dir = Path::new("/d");
        let taken = |p: &Path| p == Path::new("/d/a.txt") || p == Path::new("/d/a (1).txt") || p == Path::new("/d/noext");
        assert_eq!(unique_path(dir, "b.txt", &taken), PathBuf::from("/d/b.txt"));
        assert_eq!(unique_path(dir, "a.txt", &taken), PathBuf::from("/d/a (2).txt"));
        assert_eq!(unique_path(dir, "noext", &taken), PathBuf::from("/d/noext (1)"));
    }
}
