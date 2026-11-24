//! Media permissions and downloads for the wrapped site.
//!
//! [`decide`] answers the web view's permission requests: the microphone, the camera and
//! screen sharing each need their switch in Preferences **and** a page on the wrapped
//! origin; everything else is refused. Revoking a switch applies to the next request (or
//! after Reload).
//!
//! [`download_path`] picks where a download goes: the user's Downloads folder, under the
//! site's file name made safe, with ` (1)`, ` (2)` ... when the name is taken.
//!
//! ```
//! use desktop_core::wrapper::perm::{decide, Media};
//! use desktop_core::wrapper::{url::WebUrl, WrapperConfig};
//!
//! let cfg = WrapperConfig { url: "https://meet.example".into(), microphone: true, ..WrapperConfig::default() };
//! let site = WebUrl::parse(&cfg.url).unwrap().origin();
//! assert!(decide(&cfg, &site, Some("https://meet.example/room"), Media::Microphone));
//! assert!(!decide(&cfg, &site, Some("https://meet.example/room"), Media::Camera)); // not switched on
//! assert!(!decide(&cfg, &site, Some("https://evil.example/"), Media::Microphone)); // another origin
//! ```

use super::url::{Origin, WebUrl};
use super::WrapperConfig;
use std::path::{Path, PathBuf};

/// What a page asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    /// `getUserMedia({audio})`.
    Microphone,
    /// `getUserMedia({video})`.
    Camera,
    /// `getDisplayMedia()`.
    Screen,
    /// Anything else (geolocation, notifications, MIDI ...): always refused.
    Other,
}

/// Allow `kind` for the page at `page_url` (`None`: unknown, refused)?
pub fn decide(cfg: &WrapperConfig, site: &Origin, page_url: Option<&str>, kind: Media) -> bool {
    let same = page_url.and_then(|u| WebUrl::parse(u).ok()).is_some_and(|u| u.origin() == *site);
    same && match kind {
        Media::Microphone => cfg.microphone,
        Media::Camera => cfg.camera,
        Media::Screen => cfg.screen,
        Media::Other => false,
    }
}

/// A file name from a site made safe: no folders, no leading dots, no characters the
/// file systems refuse, at most 120 characters; `download` when nothing is left.
pub fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last.chars().map(|c| if c.is_control() || r#"<>:"|?*"#.contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim_end_matches(['.', ' ']);
    let mut out: String = trimmed.chars().take(120).collect();
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "LPT1", "LPT2"].contains(&stem.as_str()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        "download".into()
    } else {
        out
    }
}

/// `dir/name`, or `dir/name (n).ext` with the first free `n`.
pub fn download_path(dir: &Path, name: &str) -> PathBuf {
    let name = safe_name(name);
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (1..10_000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_decisions() {
        let site = WebUrl::parse("https://pikvm.local").unwrap().origin();
        let page = Some("https://pikvm.local/kvm/");
        let off = WrapperConfig { url: "https://pikvm.local".into(), ..WrapperConfig::default() };
        for k in [Media::Microphone, Media::Camera, Media::Screen, Media::Other] {
            assert!(!decide(&off, &site, page, k), "{k:?} off by default");
        }
        let on = WrapperConfig { microphone: true, camera: true, screen: true, ..off };
        assert!(decide(&on, &site, page, Media::Microphone));
        assert!(decide(&on, &site, page, Media::Camera));
        assert!(decide(&on, &site, page, Media::Screen));
        assert!(!decide(&on, &site, page, Media::Other));
        assert!(!decide(&on, &site, Some("https://pikvm.local:8443/"), Media::Camera));
        assert!(!decide(&on, &site, Some("http://pikvm.local/"), Media::Camera));
        assert!(!decide(&on, &site, None, Media::Camera));
        let revoked = WrapperConfig { camera: false, ..on };
        assert!(!decide(&revoked, &site, page, Media::Camera), "revoking applies to the next request");
    }

    #[test]
    fn download_names() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\x\\a<b>.txt"), "a_b_.txt");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name(""), "download");
        assert_eq!(safe_name("CON.txt"), "_CON.txt");
        let dir = tempfile::tempdir().unwrap();
        let p = download_path(dir.path(), "report.pdf");
        assert_eq!(p, dir.path().join("report.pdf"));
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(download_path(dir.path(), "report.pdf"), dir.path().join("report (1).pdf"));
        std::fs::write(dir.path().join("report (1).pdf"), b"x").unwrap();
        assert_eq!(download_path(dir.path(), "report.pdf"), dir.path().join("report (2).pdf"));
        std::fs::write(dir.path().join("README"), b"x").unwrap();
        assert_eq!(download_path(dir.path(), "README"), dir.path().join("README (1)"));
    }
}
