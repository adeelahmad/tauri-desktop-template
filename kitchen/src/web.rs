//! Web app mode: everything `kitchen new --web <url>` and `kitchen icon fetch <url>` need to
//! know about a site before the app exists: that its URL is usable, its title (for the
//! app's name), its certificate (trusted, or self-signed and shown by its SHA-256 so it can
//! be pinned), and its best icon, made into the app's icon set.
//!
//! The fetches are plain HTTP/1.1 over rustls. With `insecure` on, an untrusted certificate
//! is accepted for the site's own host only, and only that one certificate (the first one
//! seen) for the whole run, so a page, its manifest and its icons all come from the same
//! server. Other hosts get the normal checks. Responses are capped, redirects are followed
//! at most five times and only to http(s).
//!
//! Icons are looked for in this order: the web manifest's icons, `apple-touch-icon`,
//! `<link rel="icon">`, then `/favicon.ico`; the largest one that decodes wins. SVG icons
//! are skipped (the `image` crate does not draw them).
//!
//! ```
//! use kitchen::web::{check_url, names, parse_page};
//!
//! let url = check_url("https://pikvm.local/kvm/").unwrap();
//! assert_eq!(url.host, "pikvm.local");
//! assert!(check_url("ftp://pikvm.local").is_err());
//! assert!(check_url("https://user@pikvm.local").is_err());
//!
//! let page = parse_page(r#"<title>PiKVM &amp; friends</title><link rel="apple-touch-icon" href="/a.png" sizes="180x180">"#);
//! assert_eq!(page.title.as_deref(), Some("PiKVM & friends"));
//! assert_eq!(page.icons[0].href, "/a.png");
//!
//! let (app_name, identifier) = names(page.title.as_deref(), &url, "kvm");
//! assert_eq!(app_name, "PiKVM friends");
//! assert_eq!(identifier, "local.pikvm.kvm");
//! ```

use crate::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The most a page, manifest or icon may be.
const MAX_BODY: usize = 8 << 20;
/// Connect, read and write timeout.
const TIMEOUT: Duration = Duration::from_secs(15);
/// The side of the square PNG the icon set is made from.
pub const ICON_SIDE: u32 = 1024;

/// A checked http(s) URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// `https` or `http`.
    pub scheme: String,
    /// Host, lower case (an IPv6 address keeps its brackets).
    pub host: String,
    /// Port (the scheme's default when not given).
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl Url {
    /// `scheme://host[:port]` (the port only when not the default).
    pub fn origin(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        if self.port == default {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }

    /// The whole URL.
    pub fn as_string(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    /// `href` (absolute, `//host/...`, `/path` or relative) against this URL. `None` for
    /// other schemes (`data:`, `javascript:` ...).
    pub fn join(&self, href: &str) -> Option<Url> {
        let href = href.trim();
        let href = href.split('#').next().unwrap_or_default();
        if let Some((scheme, _)) = href.split_once(':') {
            if scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
                && !scheme.is_empty()
            {
                return check_url(href).ok();
            }
        }
        if let Some(rest) = href.strip_prefix("//") {
            return check_url(&format!("{}://{rest}", self.scheme)).ok();
        }
        let path = if href.starts_with('/') {
            href.to_string()
        } else {
            let dir = self.path.split('?').next().unwrap_or("/");
            let dir = &dir[..dir.rfind('/').map_or(0, |i| i + 1)];
            format!("{dir}{href}")
        };
        check_url(&format!("{}{}", self.origin(), path)).ok()
    }
}

/// Check a site URL the way the app does: http or https, a host, no user name or password,
/// no spaces, quotes or angle brackets (the template's `web_url` rule).
pub fn check_url(text: &str) -> Result<Url> {
    let bad = |why: &str| Error::Usage(format!("{text:?} is not a usable site URL: {why}"));
    if text
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "@\\\"<>".contains(c))
    {
        return Err(bad("no spaces, @, \\, quotes or <>"));
    }
    let (scheme, rest) = text
        .split_once("://")
        .ok_or_else(|| bad("start it with https://"))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(bad("use https:// (or http://)"));
    }
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'?' => (&rest[..i], format!("/{}", &rest[i..])),
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let default = if scheme == "https" { 443 } else { 80 };
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6
            .split_once(']')
            .ok_or_else(|| bad("an IPv6 address needs its ]"))?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().map_err(|_| bad("the port is not a number"))?,
            None if after.is_empty() => default,
            None => return Err(bad("text after the IPv6 address")),
        };
        (format!("[{}]", h.to_ascii_lowercase()), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (
                h.to_ascii_lowercase(),
                p.parse().map_err(|_| bad("the port is not a number"))?,
            ),
            None => (authority.to_ascii_lowercase(), default),
        }
    };
    let host_ok = host.starts_with('[')
        || (!host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
    if !host_ok || port == 0 {
        return Err(bad("name a host, e.g. https://pikvm.local"));
    }
    Ok(Url {
        scheme,
        host,
        port,
        path,
    })
}

/// An icon a page names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconRef {
    /// Where it is (as written in the page).
    pub href: String,
    /// The largest side it declares (`sizes="192x192"`), 0 when unknown.
    pub declared: u32,
}

/// What the page itself says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// `<title>`, entities decoded, spaces collapsed.
    pub title: Option<String>,
    /// `<link rel="manifest">`.
    pub manifest: Option<String>,
    /// `apple-touch-icon` links first, then `icon` links, each group largest first.
    pub icons: Vec<IconRef>,
}

/// Read the title, manifest and icon links out of an HTML page (no script runs; a page
/// that builds them with JavaScript has none).
pub fn parse_page(html: &str) -> Page {
    let lower = html.to_ascii_lowercase();
    let mut page = Page::default();
    if let Some(start) = lower.find("<title") {
        if let Some(gt) = lower[start..].find('>') {
            let from = start + gt + 1;
            let end = lower[from..]
                .find("</title")
                .map_or(lower.len(), |e| from + e);
            let t = decode_entities(&html[from..end])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            page.title = (!t.is_empty()).then_some(t);
        }
    }
    let (mut apple, mut plain) = (Vec::new(), Vec::new());
    let mut at = 0;
    while let Some(i) = lower[at..].find("<link") {
        let start = at + i;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let attrs = attributes(&html[start + 5..end]);
        at = end;
        let rel = attrs
            .get("rel")
            .map(|r| r.to_ascii_lowercase())
            .unwrap_or_default();
        let Some(href) = attrs.get("href").filter(|h| !h.is_empty()) else {
            continue;
        };
        let rels: Vec<&str> = rel.split_whitespace().collect();
        let declared = attrs.get("sizes").map_or(0, |s| largest_size(s));
        let icon = IconRef {
            href: decode_entities(href),
            declared,
        };
        if rels.contains(&"manifest") {
            page.manifest.get_or_insert(icon.href);
        } else if rels.iter().any(|r| r.starts_with("apple-touch-icon")) {
            apple.push(icon);
        } else if rels.contains(&"icon") && attrs.get("type").is_none_or(|t| !t.contains("svg")) {
            plain.push(icon);
        }
    }
    for list in [&mut apple, &mut plain] {
        list.sort_by_key(|i| std::cmp::Reverse(i.declared));
    }
    page.icons = apple.into_iter().chain(plain).collect();
    page
}

/// The icons of a web app manifest (JSON), largest first; monochrome ones and SVGs left out.
pub fn parse_manifest(json: &str) -> Vec<IconRef> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out: Vec<IconRef> = v["icons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| {
            !i["purpose"]
                .as_str()
                .unwrap_or_default()
                .contains("monochrome")
        })
        .filter(|i| !i["type"].as_str().unwrap_or_default().contains("svg"))
        .filter_map(|i| {
            let href = i["src"].as_str()?.to_string();
            let declared = i["sizes"].as_str().map_or(0, largest_size);
            Some(IconRef { href, declared })
        })
        .collect();
    out.sort_by_key(|i| std::cmp::Reverse(i.declared));
    out
}

/// The largest side in a `sizes` value (`"16x16 32x32"`; `any` counts as 0).
fn largest_size(s: &str) -> u32 {
    s.split_whitespace()
        .filter_map(|wh| {
            wh.to_ascii_lowercase()
                .split_once('x')
                .and_then(|(w, h)| Some(w.parse::<u32>().ok()?.max(h.parse().ok()?)))
        })
        .max()
        .unwrap_or(0)
}

/// The attributes of one tag (`name="v"`, `name='v'`, `name=v`, bare `name`), names lower case.
fn attributes(tag: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let name = tag[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let s = i + 1;
                i = s;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = tag[s..i].to_string();
                i += 1;
            } else {
                let s = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = tag[s..i].to_string();
            }
        }
        if !name.is_empty() {
            out.entry(name).or_insert(value);
        }
    }
    out
}

/// The common HTML entities, and numeric ones.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The app's name (from the page title, else the host) and a bundle identifier (the host's
/// labels reversed, then the app's folder name), both in the forms the template accepts.
pub fn names(title: Option<&str>, url: &Url, folder: &str) -> (String, String) {
    let clean = |s: &str| {
        let kept: String = s
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || " .-_'".contains(c) {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let words: Vec<&str> = kept.split_whitespace().collect();
        let mut out = String::new();
        for w in words {
            if out.chars().count() + w.chars().count() + 1 > 40 {
                break;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(w);
        }
        out
    };
    let host = url.host.trim_start_matches('[').trim_end_matches(']');
    let is_ip = host.parse::<std::net::IpAddr>().is_ok();
    let mut name = title.map(clean).unwrap_or_default();
    if name.is_empty() {
        name = clean(if is_ip {
            folder
        } else {
            host.split('.').next().unwrap_or(folder)
        });
    }
    if name.is_empty() {
        name = "Web App".into();
    }
    let label = |s: &str| {
        let l: String = s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        let l = l.trim_matches('-').to_string();
        match l.chars().next() {
            Some(c) if c.is_ascii_alphabetic() => l,
            Some(_) => format!("x{l}"),
            None => String::new(),
        }
    };
    let mut parts: Vec<String> = if is_ip {
        vec!["com".into(), "example".into()]
    } else {
        host.rsplit('.')
            .map(label)
            .filter(|l| !l.is_empty())
            .collect()
    };
    if parts.len() < 2 {
        parts = vec!["com".into(), "example".into()];
    }
    let app = label(folder);
    parts.push(if app.is_empty() { "app".into() } else { app });
    (name, parts.join("."))
}

/// What a site showed.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    /// The URL after redirects.
    pub url: Option<Url>,
    /// `<title>`.
    pub title: Option<String>,
    /// SHA-256 (lower-case hex) of the site's certificate, for https.
    pub sha256: Option<String>,
    /// Whether the system's roots trust that certificate.
    pub trusted: bool,
    /// The icon, a square PNG of [`ICON_SIDE`] pixels, when one was found.
    pub icon: Option<Vec<u8>>,
    /// Where the icon came from, and its size before scaling.
    pub icon_from: Option<(String, u32)>,
    /// Notes for the person (icons skipped and why).
    pub notes: Vec<String>,
}

/// Fetch the page at `url`, its certificate, title and best icon. An untrusted certificate
/// is an error unless `insecure` (then it is accepted for this host only, and pinned for
/// the run).
pub fn probe(url: &Url, insecure: bool) -> Result<Probe> {
    let client = Client::new(url, insecure)?;
    let (final_url, page) = client.get(url)?;
    let html = String::from_utf8_lossy(&page.body);
    let parsed = parse_page(&html);
    let mut p = Probe {
        url: Some(final_url.clone()),
        title: parsed.title.clone(),
        ..Probe::default()
    };
    if let Some(seen) = client.verifier.seen(&url.host) {
        p.sha256 = Some(seen.0);
        p.trusted = seen.1;
    }
    let mut candidates: Vec<String> = Vec::new();
    if let Some(m) = parsed.manifest.as_deref().and_then(|m| final_url.join(m)) {
        match client.get(&m) {
            Ok((murl, r)) => candidates.extend(
                parse_manifest(&String::from_utf8_lossy(&r.body))
                    .iter()
                    .filter_map(|i| murl.join(&i.href))
                    .map(|u| u.as_string()),
            ),
            Err(e) => p.notes.push(format!("manifest {}: {e}", m.as_string())),
        }
    }
    candidates.extend(
        parsed
            .icons
            .iter()
            .filter_map(|i| final_url.join(&i.href))
            .map(|u| u.as_string()),
    );
    if let Some(f) = final_url.join("/favicon.ico") {
        candidates.push(f.as_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    candidates.retain(|c| seen.insert(c.clone()));
    let mut best: Option<(image::DynamicImage, String, u32)> = None;
    for c in candidates.iter().take(8) {
        let Ok(u) = check_url(c) else { continue };
        let img = client
            .get(&u)
            .map_err(|e| e.to_string())
            .and_then(|(_, r)| {
                image::load_from_memory(&r.body)
                    .map_err(|e| format!("not an image this can read ({e})"))
            });
        match img {
            Ok(img) => {
                let side = img.width().min(img.height());
                if best.as_ref().is_none_or(|b| side > b.2) {
                    best = Some((img, c.clone(), side));
                }
                if side >= 512 {
                    break;
                }
            }
            Err(e) => p.notes.push(format!("icon {c}: {e}")),
        }
    }
    if let Some((img, from, side)) = best {
        p.icon = Some(square_png(&img)?);
        p.icon_from = Some((from, side));
    }
    Ok(p)
}

/// `img` centred on a transparent square, scaled to [`ICON_SIDE`], as PNG bytes.
pub fn square_png(img: &image::DynamicImage) -> Result<Vec<u8>> {
    let side = img.width().max(img.height()).max(1);
    let mut canvas = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut canvas,
        &img.to_rgba8(),
        i64::from((side - img.width()) / 2),
        i64::from((side - img.height()) / 2),
    );
    let out = image::imageops::resize(
        &canvas,
        ICON_SIDE,
        ICON_SIDE,
        image::imageops::FilterType::Lanczos3,
    );
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(out)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| Error::Tool(format!("encode the icon: {e}")))?;
    Ok(bytes)
}

/// Make an app's icon set from a square PNG: `cargo tauri icon` when the Tauri CLI is
/// installed (every size, `.icns` and `.ico`), else the PNG sizes and `.ico` written here
/// (the `.icns` then stays the template's until `cargo tauri icon` runs). Returns lines
/// for the report.
pub fn install_icon(app: &Path, png: &[u8]) -> Result<Vec<String>> {
    let icons = app.join("src-tauri/icons");
    let source = icons.join("source-1024.png");
    crate::io(
        format!("write {}", source.display()),
        std::fs::write(&source, png),
    )?;
    let tauri = std::process::Command::new("cargo")
        .args(["tauri", "icon"])
        .arg(&source)
        .current_dir(app)
        .output();
    if tauri.as_ref().is_ok_and(|o| o.status.success()) {
        // `cargo tauri icon` also writes mobile and Windows Store sizes the app does not use.
        for extra in ["android", "ios"] {
            let _ = std::fs::remove_dir_all(icons.join(extra));
        }
        if let Ok(dir) = std::fs::read_dir(&icons) {
            for e in dir.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if (name.starts_with("Square") && name.ends_with("Logo.png"))
                    || name == "StoreLogo.png"
                {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        return Ok(vec!["update   src-tauri/icons (cargo tauri icon)".into()]);
    }
    let img =
        image::load_from_memory(png).map_err(|e| Error::Tool(format!("read the icon: {e}")))?;
    let save = |name: &str, side: u32, format: image::ImageFormat| -> Result<()> {
        let p = icons.join(name);
        img.resize_exact(side, side, image::imageops::FilterType::Lanczos3)
            .save_with_format(&p, format)
            .map_err(|e| Error::Tool(format!("write {}: {e}", p.display())))
    };
    for (name, side) in [
        ("32x32.png", 32),
        ("64x64.png", 64),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ] {
        save(name, side, image::ImageFormat::Png)?;
    }
    save("icon.ico", 256, image::ImageFormat::Ico)?;
    Ok(vec![
        "update   src-tauri/icons (PNG sizes and icon.ico)".into(),
        "note     the macOS icon.icns is still the template's: run `cargo tauri icon src-tauri/icons/source-1024.png` once the Tauri CLI is installed".into(),
    ])
}

/// One HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    /// Status code.
    pub status: u16,
    /// Headers, names lower case.
    pub headers: Vec<(String, String)>,
    /// The body (de-chunked).
    pub body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A small HTTP/1.1 client for one site.
struct Client {
    tls: Arc<rustls::ClientConfig>,
    verifier: Arc<Verifier>,
}

impl Client {
    fn new(site: &Url, insecure: bool) -> Result<Self> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let roots = Arc::new(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        });
        let webpki =
            rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
                .build()
                .map_err(|e| Error::Tool(format!("TLS setup: {e}")))?;
        let verifier = Arc::new(Verifier {
            webpki,
            provider: provider.clone(),
            insecure_host: insecure.then(|| {
                site.host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string()
            }),
            seen: Mutex::default(),
        });
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| Error::Tool(format!("TLS setup: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        Ok(Self {
            tls: Arc::new(tls),
            verifier,
        })
    }

    /// GET `url`, following up to five redirects; a non-2xx status is an error.
    fn get(&self, url: &Url) -> Result<(Url, Response)> {
        let mut url = url.clone();
        for _ in 0..6 {
            let r = self.once(&url)?;
            if matches!(r.status, 301 | 302 | 303 | 307 | 308) {
                let next = r
                    .header("location")
                    .and_then(|l| url.join(l))
                    .ok_or_else(|| {
                        Error::Tool(format!("{}: a redirect to nowhere usable", url.as_string()))
                    })?;
                url = next;
                continue;
            }
            if !(200..300).contains(&r.status) {
                return Err(Error::Tool(format!(
                    "{}: HTTP {}",
                    url.as_string(),
                    r.status
                )));
            }
            return Ok((url, r));
        }
        Err(Error::Tool(format!(
            "{}: too many redirects",
            url.as_string()
        )))
    }

    fn once(&self, url: &Url) -> Result<Response> {
        let fail = |e: &dyn std::fmt::Display| Error::Tool(format!("{}: {e}", url.as_string()));
        let host = url.host.trim_start_matches('[').trim_end_matches(']');
        let addr = (host, url.port)
            .to_socket_addrs()
            .map_err(|e| fail(&e))?
            .next()
            .ok_or_else(|| fail(&"no address"))?;
        let mut tcp = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| fail(&e))?;
        tcp.set_read_timeout(Some(TIMEOUT)).map_err(|e| fail(&e))?;
        tcp.set_write_timeout(Some(TIMEOUT)).map_err(|e| fail(&e))?;
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: kitchen/{}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
            url.path,
            url.origin().split_once("://").map_or("", |(_, h)| h),
            env!("CARGO_PKG_VERSION")
        );
        let mut raw = Vec::new();
        if url.scheme == "https" {
            let tls = self.tls.clone();
            let name = ServerName::try_from(host.to_string()).map_err(|e| fail(&e))?;
            let conn = rustls::ClientConnection::new(tls, name).map_err(|e| fail(&e))?;
            let mut s = rustls::StreamOwned::new(conn, tcp);
            s.write_all(request.as_bytes()).map_err(|e| fail(&e))?;
            read_capped(&mut s, &mut raw).map_err(|e| fail(&e))?;
        } else {
            tcp.write_all(request.as_bytes()).map_err(|e| fail(&e))?;
            read_capped(&mut tcp, &mut raw).map_err(|e| fail(&e))?;
        }
        parse_response(&raw).ok_or_else(|| fail(&"not an HTTP response"))
    }
}

/// Read until the end, at most [`MAX_BODY`] plus headers; a TLS close without notify at
/// the end counts as the end (many small servers do that).
fn read_capped(r: &mut impl Read, out: &mut Vec<u8>) -> std::io::Result<()> {
    let mut buf = [0u8; 16 << 10];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > MAX_BODY + (64 << 10) {
                    return Err(std::io::Error::other("the response is too large"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !out.is_empty() => {
                return Ok(())
            }
            Err(e) => return Err(e),
        }
    }
}

/// Split a whole HTTP/1.1 response.
pub fn parse_response(raw: &[u8]) -> Option<Response> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let mut body = raw[end + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        body = dechunk(&body)?;
    } else if let Some(len) = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
    {
        body.truncate(len);
    }
    Some(Response {
        status,
        headers,
        body,
    })
}

/// Undo `Transfer-Encoding: chunked`.
fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = b.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&b[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        b = &b[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..).unwrap_or_default();
    }
}

/// Checks certificates with the system-independent web roots, records what each host
/// showed, and accepts an untrusted one for the site's host only, and only the first one
/// seen there.
#[derive(Debug)]
struct Verifier {
    webpki: Arc<rustls::client::WebPkiServerVerifier>,
    provider: Arc<rustls::crypto::CryptoProvider>,
    insecure_host: Option<String>,
    /// host → (SHA-256, trusted)
    seen: Mutex<BTreeMap<String, (String, bool)>>,
}

impl Verifier {
    fn seen(&self, host: &str) -> Option<(String, bool)> {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(host)
            .cloned()
    }
}

/// SHA-256 of `der`, lower-case hex (the form the app stores).
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `AB:CD:...`, the form browsers show.
pub fn display_fingerprint(fp: &str) -> String {
    fp.as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let host = match server_name {
            ServerName::DnsName(d) => d.as_ref().to_ascii_lowercase(),
            ServerName::IpAddress(ip) => std::net::IpAddr::from(*ip).to_string(),
            _ => String::new(),
        };
        let fp = fingerprint(end_entity.as_ref());
        let trusted = self
            .webpki
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
            .is_ok();
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let first = seen
            .entry(host.clone())
            .or_insert_with(|| (fp.clone(), trusted))
            .clone();
        if trusted {
            return Ok(ServerCertVerified::assertion());
        }
        if self.insecure_host.as_deref() != Some(host.as_str()) {
            return Err(rustls::Error::General(format!(
                "the certificate of {host} is not trusted (SHA-256 {}); for a self-signed site answer yes to \"ignore TLS certificate errors\" (--insecure-tls)",
                display_fingerprint(&fp)
            )));
        }
        if first.0 != fp {
            return Err(rustls::Error::General(format!(
                "{host} showed another certificate during this run ({} after {})",
                display_fingerprint(&fp),
                display_fingerprint(&first.0)
            )));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The answers `kitchen new --web` sets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebAnswers {
    /// The site URL.
    pub url: String,
    /// Ignore TLS errors for the site (with pinning).
    pub insecure_tls: bool,
    /// The SHA-256 to pin (lower-case hex), when known.
    pub cert_sha256: String,
    /// Capture all keys.
    pub capture_keys: bool,
    /// The release shortcut.
    pub release_shortcut: String,
    /// Allow the microphone.
    pub microphone: bool,
    /// Start muted.
    pub muted: bool,
    /// Allow the webcam.
    pub camera: bool,
    /// Allow screen sharing.
    pub screen: bool,
    /// The custom URL scheme (empty: the app's name).
    pub scheme: String,
}

impl WebAnswers {
    /// As `kitchen.toml` answers: (name, value), plus the features to turn on.
    pub fn answers(&self) -> (Vec<String>, Vec<(String, String)>) {
        // The window's remembered size and place, the menu bar icon (and with it the Dock
        // switch) and starting at login: user preferences in the wrapper (dockIcon,
        // menuBarIcon, runAtStartup, rememberWindow), built on these template parts.
        let mut on: Vec<String> = [
            "web_wrapper",
            "tray_menu",
            "tauri_plugins",
            "tauri_window_state",
            "tauri_autostart",
        ]
        .map(String::from)
        .to_vec();
        for (name, v) in [
            ("web_insecure_tls", self.insecure_tls),
            ("capture_keys", self.capture_keys),
            ("web_microphone", self.microphone),
            ("web_muted", self.muted),
            ("web_camera", self.camera),
            ("web_screen", self.screen),
        ] {
            if v {
                on.push(name.into());
            }
        }
        let mut set = vec![("web_url".to_string(), self.url.clone())];
        if !self.cert_sha256.is_empty() {
            set.push(("web_cert_sha256".into(), self.cert_sha256.clone()));
        }
        if !self.release_shortcut.is_empty() {
            set.push(("release_shortcut".into(), self.release_shortcut.clone()));
        }
        if !self.scheme.is_empty() {
            // The scheme is only registered (Info.plist, tauri.conf.json) with the plugin on.
            on.push("tauri_deep_link".into());
            set.push(("deep_link_scheme".into(), self.scheme.clone()));
        }
        (on, set)
    }
}

/// Check a release shortcut the way the app parses it (at least one modifier, or a lone
/// F13–F24, Pause or ScrollLock), and the template's rule.
pub fn check_shortcut(s: &str) -> Result<String> {
    let s = s.trim();
    let bad = |why: &str| Error::Usage(format!("{s:?} is not a usable shortcut: {why}"));
    if s.is_empty()
        || s.len() > 40
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == ' ')
    {
        return Err(bad("letters, digits and + only, e.g. Ctrl+Alt+Shift+K"));
    }
    let parts: Vec<String> = s
        .split('+')
        .map(|p| p.trim().to_ascii_lowercase())
        .collect();
    let mods = [
        "ctrl", "control", "alt", "option", "opt", "shift", "cmd", "command", "meta", "super",
        "win",
    ];
    let (m, k): (Vec<&String>, Vec<&String>) =
        parts.iter().partition(|p| mods.contains(&p.as_str()));
    if k.len() != 1 || k[0].is_empty() {
        return Err(bad("name exactly one key, e.g. Ctrl+Alt+Shift+K"));
    }
    let lone = ["pause", "scrolllock"].contains(&k[0].as_str())
        || k[0]
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (13..=24).contains(&n));
    if m.is_empty() && !lone {
        return Err(bad(
            "add a modifier (Ctrl, Alt, Shift or Cmd), or use F13–F24, Pause or ScrollLock",
        ));
    }
    Ok(s.to_string())
}

/// Check a custom URL scheme (`myapp` for `myapp://...`).
pub fn check_scheme(s: &str) -> Result<String> {
    let ok = (2..=32).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c))
        && !matches!(
            s,
            "http"
                | "https"
                | "file"
                | "ftp"
                | "mailto"
                | "javascript"
                | "data"
                | "about"
                | "blob"
                | "ws"
                | "wss"
        );
    if ok {
        Ok(s.to_string())
    } else {
        Err(Error::Usage(format!("{s:?} is not a usable URL scheme: 2-32 lower-case letters, digits, + . -, starting with a letter, and not a web scheme")))
    }
}

/// `kitchen icon fetch <url>`: write the site's icon as a 1024×1024 PNG to `out`, or, in an
/// app (`app` is its root), make the app's icon set from it.
pub fn icon_fetch(
    url: &str,
    insecure: bool,
    app: Option<&Path>,
    out: Option<PathBuf>,
) -> Result<crate::Report> {
    let url = check_url(url)?;
    let p = probe(&url, insecure)?;
    let mut r = crate::Report::new(format!("Icon of {}", url.as_string()));
    if let Some(fp) = &p.sha256 {
        r.lines.push(format!(
            "certificate SHA-256 {}{}",
            display_fingerprint(fp),
            if p.trusted {
                ""
            } else {
                " (not trusted by the system: self-signed?)"
            }
        ));
    }
    r.lines
        .extend(p.notes.iter().map(|n| format!("skipped  {n}")));
    let (Some(png), Some((from, side))) = (p.icon, p.icon_from) else {
        return Err(Error::Tool(format!("{}: no icon found (looked at the manifest, apple-touch-icon, link rel=icon and /favicon.ico)", url.as_string())));
    };
    r.lines.push(format!("found    {from} ({side}px)"));
    match (app, out) {
        (_, Some(path)) => {
            crate::io(
                format!("write {}", path.display()),
                std::fs::write(&path, &png),
            )?;
            r.lines.push(format!("write    {}", path.display()));
        }
        (Some(root), None) => r.lines.extend(install_icon(root, &png)?),
        (None, None) => {
            let path = PathBuf::from("icon.png");
            crate::io("write icon.png", std::fs::write(&path, &png))?;
            r.lines.push("write    icon.png".into());
        }
    }
    if side < 256 {
        r.next.push(format!("the site's largest icon is {side}px, so the app icon is scaled up: replace src-tauri/icons/source-1024.png with a sharper one and run `cargo tauri icon` on it"));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_checked_and_joined() {
        let u = check_url("https://PiKVM.local:8443/kvm/index.html?x=1").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("pikvm.local", 8443, "/kvm/index.html?x=1")
        );
        assert_eq!(u.origin(), "https://pikvm.local:8443");
        assert_eq!(
            u.join("icon.png").unwrap().as_string(),
            "https://pikvm.local:8443/kvm/icon.png"
        );
        assert_eq!(u.join("/favicon.ico").unwrap().path, "/favicon.ico");
        assert_eq!(
            u.join("//cdn.example/x.png").unwrap().as_string(),
            "https://cdn.example/x.png"
        );
        assert_eq!(u.join("http://other/").unwrap().host, "other");
        assert!(u.join("data:image/png;base64,AAAA").is_none());
        assert!(u.join("javascript:alert(1)").is_none());
        assert_eq!(check_url("https://[::1]:9/").unwrap().host, "[::1]");
        assert_eq!(check_url("http://h?q").unwrap().path, "/?q");
        for bad in [
            "pikvm.local",
            "file:///etc/passwd",
            "https://",
            "https://a b",
            "https://u:p@h/",
            "https://h:x/",
            "https://h\\x",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pages_and_manifests_give_icons_in_order() {
        let html = r#"<html><head><TITLE>
            My &#x4B;VM </title>
            <link rel="icon" href="/small.png" sizes="16x16">
            <link rel=icon href=/big.png sizes="16x16 64x64">
            <link rel="icon" type="image/svg+xml" href="/v.svg">
            <link rel="apple-touch-icon-precomposed" href="/apple.png">
            <link rel="manifest" href="/site.webmanifest">
            </head>"#;
        let p = parse_page(html);
        assert_eq!(p.title.as_deref(), Some("My KVM"));
        assert_eq!(p.manifest.as_deref(), Some("/site.webmanifest"));
        let hrefs: Vec<&str> = p.icons.iter().map(|i| i.href.as_str()).collect();
        assert_eq!(hrefs, ["/apple.png", "/big.png", "/small.png"]);
        let m = parse_manifest(
            r#"{"icons":[{"src":"a.png","sizes":"192x192"},{"src":"b.png","sizes":"512x512"},{"src":"m.png","sizes":"1024x1024","purpose":"monochrome"},{"src":"v.svg","type":"image/svg+xml","sizes":"any"}]}"#,
        );
        assert_eq!(
            m.iter().map(|i| i.href.as_str()).collect::<Vec<_>>(),
            ["b.png", "a.png"]
        );
        assert!(parse_manifest("not json").is_empty());
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &bogus; &"),
            "a & b <c> &bogus; &"
        );
    }

    #[test]
    fn names_come_from_the_title_or_host() {
        let u = check_url("https://pikvm.local/").unwrap();
        assert_eq!(
            names(Some("PiKVM: \"main\" <kvm>"), &u, "kvm"),
            ("PiKVM main kvm".into(), "local.pikvm.kvm".into())
        );
        assert_eq!(
            names(None, &u, "kvm"),
            ("pikvm".into(), "local.pikvm.kvm".into())
        );
        let ip = check_url("https://192.168.1.5:8443/").unwrap();
        assert_eq!(
            names(Some(""), &ip, "my-kvm"),
            ("my-kvm".into(), "com.example.my-kvm".into())
        );
        let digits = check_url("https://1password.example/").unwrap();
        assert_eq!(names(None, &digits, "x").1, "example.x1password.x");
    }

    #[test]
    fn responses_are_parsed() {
        let r = parse_response(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"abcde"[..]));
        assert_eq!(r.header("content-type"), Some("text/html"));
        let r = parse_response(
            b"HTTP/1.0 301 Moved\r\nLocation: /x\r\nContent-Length: 2\r\n\r\nhi there",
        )
        .unwrap();
        assert_eq!((r.status, r.body.as_slice()), (301, &b"hi"[..]));
        assert!(parse_response(b"garbage").is_none());
    }

    #[test]
    fn shortcuts_and_schemes() {
        assert!(check_shortcut("Ctrl+Alt+Shift+K").is_ok());
        assert!(check_shortcut("F13").is_ok());
        assert!(check_shortcut("K").is_err());
        assert!(check_shortcut("Ctrl+K+J").is_err());
        assert!(check_shortcut("Ctrl+\"").is_err());
        assert!(check_scheme("kvm").is_ok());
        assert!(check_scheme("https").is_err());
        assert!(check_scheme("Kvm").is_err());
        assert!(check_scheme("k").is_err());
    }

    #[test]
    fn answers_name_the_template_questions() {
        let a = WebAnswers {
            url: "https://pikvm.local/".into(),
            insecure_tls: true,
            cert_sha256: "ab".repeat(32),
            capture_keys: true,
            ..WebAnswers::default()
        };
        let (on, set) = a.answers();
        assert_eq!(
            on,
            [
                "web_wrapper",
                "tray_menu",
                "tauri_plugins",
                "tauri_window_state",
                "tauri_autostart",
                "web_insecure_tls",
                "capture_keys"
            ]
        );
        assert_eq!(
            set[0],
            ("web_url".to_string(), "https://pikvm.local/".to_string())
        );
        assert_eq!(set[1].0, "web_cert_sha256");
    }

    #[test]
    fn icons_are_made_square() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            40,
            20,
            image::Rgba([255, 0, 0, 255]),
        ));
        let png = square_png(&img).unwrap();
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (ICON_SIDE, ICON_SIDE));
        assert_eq!(
            back.to_rgba8().get_pixel(0, 0)[3],
            0,
            "padding is transparent"
        );
        assert_eq!(back.to_rgba8().get_pixel(512, 512)[0], 255);
    }
}
