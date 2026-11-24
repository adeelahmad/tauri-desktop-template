//! The loopback proxy: the window loads `http://127.0.0.1:<port>`, and every request
//! (pages, scripts, images, `fetch`, WebSockets) goes on to the site from here, with the
//! configured headers and User-Agent added and the site's certificate checked or pinned.
//!
//! Why: a web view can add headers to the page it opens, but not (on macOS and Linux) to
//! everything that page loads afterwards. Off by default ([`super::ProxyMode`]); direct mode
//! stays the simple path.
//!
//! - **Only this app's window.** Each launch makes a random token. The window first opens
//!   [`Proxy::auth_url`], which sets it as an `HttpOnly`, `SameSite=Strict` cookie and
//!   redirects; any request without it (another app, another user's browser on this
//!   computer, a page on another site) gets `403`. The `Host` must be the loopback address,
//!   so DNS rebinding does not get in either.
//! - **Loopback is a secure context**, so `getUserMedia`, service workers and WebCrypto
//!   still work on `http://127.0.0.1`; there is no local CA and the system trust store is
//!   not touched.
//! - **Upstream TLS** is rustls: the Mozilla roots, or with `insecureTls` the pinned
//!   certificate ([`super::pin::PinVerifier`]), for this one origin. Every connection asks
//!   [`crate::net::check_host`] first, so offline mode and the deny list apply.
//! - **Rewriting** is kept small: `Set-Cookie` loses `Domain` and `Secure` (the loopback leg
//!   is plain HTTP; `SameSite=None` becomes `Lax`), `Location` on the site becomes a local
//!   path, `Origin` and `Referer` go upstream as the site's own, and HSTS is dropped.
//!   Bodies are never rewritten, so a page that writes absolute links to its own origin
//!   leaves the proxy on click (the app sends those back through it).
//! - **Streaming.** Responses are copied as they arrive (MJPEG, chunked, server-sent events),
//!   never buffered. Each request uses its own upstream connection (`Connection: close`).
//! - **WebSockets** (PiKVM's keyboard, mouse and stream control) are upgraded upstream and
//!   then copied both ways.
//! - **WebRTC media** does not use HTTP: it goes from the web view to the site over UDP, so
//!   it never passes the proxy and gets no extra headers. For PiKVM, Janus runs on the same
//!   host, so this needs nothing.
//! - Cookies and storage belong to the loopback origin, so the port is the same on every
//!   launch ([`preferred_port`]); if it is taken, another one is used and the site's cookies
//!   start over for that launch.
//!
//! ```no_run
//! use desktop_core::wrapper::proxy::{tls_config, Proxy, Upstream};
//! use desktop_core::wrapper::url::WebUrl;
//! use std::sync::Arc;
//!
//! let site = WebUrl::parse("https://pikvm.local/")?;
//! let proxy = Proxy::start(Upstream {
//!     site,
//!     tls: Arc::new(tls_config(None)),
//!     headers: Arc::new(|_url: &str| vec![("X-Team".to_string(), "ops".to_string())]),
//!     user_agent: None,
//!     port: 0,
//! }).map_err(|e| e.to_string())?;
//! let first = proxy.auth_url("/kvm/"); // open this in the window
//! assert!(first.starts_with("http://127.0.0.1:"));
//! # Ok::<(), String>(())
//! ```

use super::url::WebUrl;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The cookie that carries the token.
const COOKIE: &str = "__kitchen_proxy";
/// Path of the one-time sign-in URL.
const AUTH_PATH: &str = "/__kitchen/auth";
/// The longest request or response head.
const MAX_HEAD: usize = 64 * 1024;
/// The most connections at once.
const MAX_CONNECTIONS: usize = 256;

/// Headers for one upstream URL.
pub type HeaderFn = dyn Fn(&str) -> Vec<(String, String)> + Send + Sync;

/// Where the proxy sends requests, and what it adds.
#[derive(Clone)]
pub struct Upstream {
    /// The site (its origin is used).
    pub site: WebUrl,
    /// TLS settings for `https` sites ([`tls_config`]).
    pub tls: Arc<rustls::ClientConfig>,
    /// Extra headers for an upstream URL (values never logged).
    pub headers: Arc<HeaderFn>,
    /// The User-Agent to send instead of the web view's.
    pub user_agent: Option<String>,
    /// Port to listen on (0: any free one).
    pub port: u16,
}

/// TLS for the upstream leg: the Mozilla roots, or with `pin` only the pinned certificate.
pub fn tls_config(pin: Option<super::pin::PinStore>) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder =
        rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("ring supports TLS 1.2 and 1.3");
    let mut cfg = match pin {
        Some(store) => {
            builder.dangerous().with_custom_certificate_verifier(Arc::new(super::pin::PinVerifier::new(store))).with_no_client_auth()
        }
        None => {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    cfg
}

/// A port that is the same for the same app on every launch (so cookies and storage,
/// which belong to `127.0.0.1:<port>`, survive restarts): 20000–59999 from `app_id`.
pub fn preferred_port(app_id: &str) -> u16 {
    let d = <sha2::Sha256 as sha2::Digest>::digest(app_id.as_bytes());
    20000 + (u16::from_be_bytes([d[0], d[1]]) % 40000)
}

/// A running proxy. Dropping it stops it.
pub struct Proxy {
    port: u16,
    token: Arc<String>,
    site: WebUrl,
    stop: Arc<AtomicBool>,
}

impl Proxy {
    /// Listen on 127.0.0.1 (the preferred port, else any) and serve until dropped.
    pub fn start(up: Upstream) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, up.port)).or_else(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        let token = Arc::new(random_token()?);
        let stop = Arc::new(AtomicBool::new(false));
        let site = up.site.clone();
        let (tok, halt, up) = (token.clone(), stop.clone(), Arc::new(up));
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new().name("wrapper-proxy".into()).spawn(move || {
            for conn in listener.incoming() {
                if halt.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                if live.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                live.fetch_add(1, Ordering::Relaxed);
                let (tok, up, live) = (tok.clone(), up.clone(), live.clone());
                let _ = std::thread::Builder::new().name("wrapper-proxy-conn".into()).spawn(move || {
                    if let Err(e) = serve(conn, &up, &tok, port) {
                        log::debug!("proxy: {e}");
                    }
                    live.fetch_sub(1, Ordering::Relaxed);
                });
            }
        })?;
        log::info!("proxy for {} on 127.0.0.1:{port}", site.origin());
        Ok(Self { port, token, site, stop })
    }

    /// The port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>`.
    pub fn local_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The first URL the window opens: it sets the token cookie and redirects to `path`.
    pub fn auth_url(&self, path: &str) -> String {
        let path = if path.starts_with('/') && !path.starts_with("//") { path } else { "/" };
        format!("{}{AUTH_PATH}?t={}&to={}", self.local_origin(), self.token, percent_encode(path))
    }

    /// The site's URL for a local one (`None` when `url` is not on this proxy).
    pub fn to_upstream(&self, url: &str) -> Option<String> {
        let rest = url.strip_prefix(&self.local_origin())?;
        (rest.is_empty() || rest.starts_with(['/', '?', '#']))
            .then(|| format!("{}{}", self.site.origin(), if rest.is_empty() { "/" } else { rest }))
    }

    /// The local URL for one on the site (`None` for another origin).
    pub fn to_local(&self, url: &str) -> Option<String> {
        let u = WebUrl::parse(url).ok()?;
        (u.origin() == self.site.origin()).then(|| format!("{}{}", self.local_origin(), u.rest))
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, self.port).into(), Duration::from_millis(200));
    }
}

fn random_token() -> io::Result<String> {
    let mut b = [0u8; 24];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).map_err(|_| io::Error::other("no system randomness"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A parsed head: the first line and the headers, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    /// Request line or status line.
    pub line: String,
    /// `(name, value)` as received.
    pub headers: Vec<(String, String)>,
}

impl Head {
    /// Parse `raw` (without the final blank line).
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(raw).map_err(|_| "head is not UTF-8")?;
        let mut lines = text.split("\r\n");
        let line = lines.next().unwrap_or_default().to_string();
        let mut headers = Vec::new();
        for l in lines.filter(|l| !l.is_empty()) {
            let (k, v) = l.split_once(':').ok_or("bad header line")?;
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        Ok(Self { line, headers })
    }

    /// The first value of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn remove(&mut self, name: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    fn set(&mut self, name: &str, value: String) {
        self.remove(name);
        self.headers.push((name.to_string(), value));
    }

    /// The head as bytes, with the blank line.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = format!("{}\r\n", self.line);
        for (k, v) in &self.headers {
            s.push_str(&format!("{k}: {v}\r\n"));
        }
        s.push_str("\r\n");
        s.into_bytes()
    }

    fn is_upgrade(&self) -> bool {
        self.get("upgrade").is_some() && self.get("connection").is_some_and(|c| c.to_ascii_lowercase().contains("upgrade"))
    }
}

/// Read up to the end of a head; returns `(head, bytes already read past it)`.
fn read_head(r: &mut dyn FnMut(&mut [u8]) -> io::Result<usize>) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            buf.truncate(i);
            return Ok(Some((buf, rest)));
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::other("head too large"));
        }
        let n = r(&mut chunk)?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn respond(mut c: &TcpStream, status: &str, extra: &[(&str, &str)], body: &str) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n", body.len());
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    c.write_all(head.as_bytes())?;
    c.write_all(body.as_bytes())
}

/// Whether the request carries the token cookie.
fn has_token(head: &Head, token: &str) -> bool {
    head.headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, v)| v.split(';'))
        .any(|c| c.trim().split_once('=').is_some_and(|(k, v)| k == COOKIE && constant_eq(v.as_bytes(), token.as_bytes())))
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The request as it goes to the site: hop-by-hop headers and the token cookie removed,
/// `Host`, `Origin` and `Referer` made the site's, the User-Agent and extra headers added.
pub fn rewrite_request(mut head: Head, up: &Upstream, local_origin: &str) -> Head {
    let site = up.site.origin().to_string();
    let upgrade = head.is_upgrade();
    for h in ["connection", "keep-alive", "proxy-connection", "proxy-authorization", "te", "trailer", "host"] {
        head.remove(h);
    }
    let cookies: Vec<String> = head
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, v)| v.split(';').map(|c| c.trim().to_string()).collect::<Vec<_>>())
        .filter(|c| !c.is_empty() && !c.starts_with(&format!("{COOKIE}=")))
        .collect();
    head.remove("cookie");
    if !cookies.is_empty() {
        head.headers.push(("Cookie".into(), cookies.join("; ")));
    }
    if let Some(o) = head.get("origin").map(String::from) {
        head.set("Origin", if o == local_origin { site.clone() } else { o });
    }
    if let Some(r) = head.get("referer").map(String::from) {
        if let Some(rest) = r.strip_prefix(local_origin) {
            head.set("Referer", format!("{site}{rest}"));
        }
    }
    if let Some(ua) = &up.user_agent {
        head.set("User-Agent", ua.clone());
    }
    let target = head.line.split(' ').nth(1).unwrap_or("/");
    for (k, v) in (up.headers)(&format!("{site}{target}")) {
        head.set(&k, v);
    }
    let host = site.split("://").nth(1).unwrap_or_default().to_string();
    head.headers.insert(0, ("Host".into(), host));
    if upgrade {
        head.set("Connection", "Upgrade".into());
    } else {
        head.set("Connection", "close".into());
    }
    head
}

/// The response as it goes to the window (see the module docs).
pub fn rewrite_response(mut head: Head, site: &str) -> Head {
    let switching = head.line.split(' ').nth(1) == Some("101");
    head.remove("strict-transport-security");
    head.remove("alt-svc");
    for (k, v) in head.headers.iter_mut() {
        if k.eq_ignore_ascii_case("set-cookie") {
            *v = rewrite_cookie(v);
        } else if k.eq_ignore_ascii_case("location") {
            if let Some(rest) = v.strip_prefix(site) {
                *v = if rest.is_empty() { "/".into() } else { rest.to_string() };
            }
        }
    }
    if !switching {
        head.remove("keep-alive");
        head.set("Connection", "close".into());
    }
    head
}

/// `Set-Cookie` for the loopback leg: no `Domain`, no `Secure`, `SameSite=None` → `Lax`.
pub fn rewrite_cookie(v: &str) -> String {
    let mut parts = v.split(';').map(str::trim);
    let mut out = vec![parts.next().unwrap_or_default().to_string()];
    for a in parts {
        let name = a.split('=').next().unwrap_or_default().trim().to_ascii_lowercase();
        match name.as_str() {
            "domain" | "secure" | "partitioned" => {}
            "samesite" if a.to_ascii_lowercase().ends_with("none") => out.push("SameSite=Lax".into()),
            _ => out.push(a.to_string()),
        }
    }
    out.join("; ")
}

/// One connection to the site, usable from two threads (one reading, one writing).
enum Site {
    Plain(TcpStream, TcpStream),
    Tls { conn: Box<Mutex<rustls::ClientConnection>>, r: TcpStream, w: TcpStream },
}

impl Site {
    fn connect(up: &Upstream) -> io::Result<Self> {
        let host = up.site.host.trim_start_matches('[').trim_end_matches(']').to_string();
        crate::net::check_host(&host).map_err(io::Error::other)?;
        let sock = TcpStream::connect((host.as_str(), up.site.port))?;
        sock.set_nodelay(true)?;
        if up.site.scheme == "http" {
            return Ok(Site::Plain(sock.try_clone()?, sock));
        }
        let name = rustls::pki_types::ServerName::try_from(host).map_err(|e| io::Error::other(e.to_string()))?;
        let mut conn = rustls::ClientConnection::new(up.tls.clone(), name).map_err(io::Error::other)?;
        let mut s = &sock;
        while conn.is_handshaking() {
            conn.complete_io(&mut s)?;
        }
        Ok(Site::Tls { conn: Box::new(Mutex::new(conn)), r: sock.try_clone()?, w: sock })
    }

    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Site::Plain(r, _) => (&*r).read(buf),
            Site::Tls { conn, r, w } => loop {
                {
                    let mut c = conn.lock().unwrap_or_else(PoisonError::into_inner);
                    match c.reader().read(buf) {
                        Ok(n) => return Ok(n),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(0),
                        Err(e) => return Err(e),
                    }
                }
                let mut raw = [0u8; 16 * 1024];
                let n = (&*r).read(&mut raw)?;
                let mut c = conn.lock().unwrap_or_else(PoisonError::into_inner);
                if n == 0 {
                    return Ok(0);
                }
                let mut slice = &raw[..n];
                while !slice.is_empty() {
                    c.read_tls(&mut slice)?;
                    c.process_new_packets().map_err(io::Error::other)?;
                }
                while c.wants_write() {
                    c.write_tls(&mut &*w)?;
                }
            },
        }
    }

    fn write_all(&self, data: &[u8]) -> io::Result<()> {
        match self {
            Site::Plain(_, w) => (&*w).write_all(data),
            Site::Tls { conn, w, .. } => {
                let mut c = conn.lock().unwrap_or_else(PoisonError::into_inner);
                c.writer().write_all(data)?;
                while c.wants_write() {
                    c.write_tls(&mut &*w)?;
                }
                Ok(())
            }
        }
    }

    fn close(&self) {
        let s = match self {
            Site::Plain(r, _) => r,
            Site::Tls { r, .. } => r,
        };
        let _ = s.shutdown(Shutdown::Both);
    }
}

fn serve(client: TcpStream, up: &Upstream, token: &str, port: u16) -> io::Result<()> {
    client.set_nodelay(true)?;
    client.set_read_timeout(Some(Duration::from_secs(30)))?;
    let local = format!("http://127.0.0.1:{port}");
    let Some((raw, extra)) = read_head(&mut |b| (&client).read(b))? else { return Ok(()) };
    let head = Head::parse(&raw).map_err(io::Error::other)?;
    let host_ok = head.get("host").is_some_and(|h| h == format!("127.0.0.1:{port}"));
    if !host_ok {
        return respond(&client, "421 Misdirected Request", &[], "this proxy serves only 127.0.0.1\n");
    }
    let target = head.line.split(' ').nth(1).unwrap_or("/").to_string();
    if let Some(q) = target.strip_prefix(AUTH_PATH).and_then(|r| r.strip_prefix('?')) {
        let arg = |k: &str| q.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).map(percent_decode);
        if arg("t").is_some_and(|t| constant_eq(t.as_bytes(), token.as_bytes())) {
            let to = arg("to").filter(|p| p.starts_with('/') && !p.starts_with("//")).unwrap_or_else(|| "/".into());
            let cookie = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict");
            return respond(&client, "302 Found", &[("Location", &to), ("Set-Cookie", &cookie)], "");
        }
        return respond(&client, "403 Forbidden", &[], "wrong token\n");
    }
    if !has_token(&head, token) {
        return respond(&client, "403 Forbidden", &[], "this proxy serves only the app's own window\n");
    }
    client.set_read_timeout(None)?;
    let out = rewrite_request(head, up, &local);
    let site = match Site::connect(up) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log::warn!("proxy: could not reach {}: {e}", up.site.origin());
            return respond(&client, "502 Bad Gateway", &[], &format!("could not reach the site: {e}\n"));
        }
    };
    site.write_all(&out.to_bytes())?;
    if !extra.is_empty() {
        site.write_all(&extra)?;
    }
    // The window → the site: request bodies and WebSocket frames.
    let (up_site, from) = (site.clone(), client.try_clone()?);
    let pump = std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match (&from).read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if up_site.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });
    // The site → the window: the head rewritten, then the body as it arrives.
    let result = (|| -> io::Result<()> {
        let Some((raw, rest)) = read_head(&mut |b| site.read(b))? else { return Ok(()) };
        let head = rewrite_response(Head::parse(&raw).map_err(io::Error::other)?, &up.site.origin().to_string());
        let mut c = &client;
        c.write_all(&head.to_bytes())?;
        c.write_all(&rest)?;
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = site.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            c.write_all(&buf[..n])?;
        }
    })();
    let _ = client.shutdown(Shutdown::Both);
    site.close();
    let _ = pump.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up() -> Upstream {
        Upstream {
            site: WebUrl::parse("https://pikvm.local").unwrap(),
            tls: Arc::new(tls_config(None)),
            headers: Arc::new(|u: &str| if u.contains("/api/") { vec![("X-Key".into(), "k".into())] } else { vec![] }),
            user_agent: Some("KVM/1".into()),
            port: 0,
        }
    }

    #[test]
    fn requests_go_upstream_as_the_sites_own() {
        let raw = b"GET /api/x HTTP/1.1\r\nHost: 127.0.0.1:5000\r\nConnection: keep-alive\r\nCookie: __kitchen_proxy=t; sid=1\r\nOrigin: http://127.0.0.1:5000\r\nReferer: http://127.0.0.1:5000/kvm/\r\nUser-Agent: WebKit\r\nX-Key: page";
        let h = rewrite_request(Head::parse(raw).unwrap(), &up(), "http://127.0.0.1:5000");
        assert_eq!(h.headers[0], ("Host".into(), "pikvm.local".into()));
        assert_eq!(h.get("cookie"), Some("sid=1"), "the token never leaves this computer");
        assert_eq!(h.get("origin"), Some("https://pikvm.local"));
        assert_eq!(h.get("referer"), Some("https://pikvm.local/kvm/"));
        assert_eq!(h.get("user-agent"), Some("KVM/1"));
        assert_eq!(h.get("x-key"), Some("k"), "the rule replaces the page's own");
        assert_eq!(h.get("connection"), Some("close"));
        let ws = b"GET /api/ws HTTP/1.1\r\nHost: x\r\nConnection: keep-alive, Upgrade\r\nUpgrade: websocket";
        let h = rewrite_request(Head::parse(ws).unwrap(), &up(), "http://127.0.0.1:5000");
        assert_eq!((h.get("connection"), h.get("upgrade")), (Some("Upgrade"), Some("websocket")));
    }

    #[test]
    fn responses_are_made_local() {
        let raw = b"HTTP/1.1 302 Found\r\nLocation: https://pikvm.local/login\r\nSet-Cookie: auth=x; Domain=pikvm.local; Path=/; Secure; HttpOnly; SameSite=None\r\nStrict-Transport-Security: max-age=1\r\nConnection: keep-alive";
        let h = rewrite_response(Head::parse(raw).unwrap(), "https://pikvm.local");
        assert_eq!(h.get("location"), Some("/login"));
        assert_eq!(h.get("set-cookie"), Some("auth=x; Path=/; HttpOnly; SameSite=Lax"));
        assert_eq!(h.get("strict-transport-security"), None);
        assert_eq!(h.get("connection"), Some("close"));
        let other = rewrite_response(Head::parse(b"HTTP/1.1 302 Found\r\nLocation: https://sso.example/").unwrap(), "https://pikvm.local");
        assert_eq!(other.get("location"), Some("https://sso.example/"));
        let ws = rewrite_response(
            Head::parse(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket").unwrap(),
            "https://pikvm.local",
        );
        assert_eq!(ws.get("connection"), Some("Upgrade"));
    }

    #[test]
    fn tokens_ports_and_urls() {
        let h = Head::parse(b"GET / HTTP/1.1\r\nCookie: a=1; __kitchen_proxy=abc").unwrap();
        assert!(has_token(&h, "abc"));
        assert!(!has_token(&h, "abd"));
        assert!(!has_token(&Head::parse(b"GET / HTTP/1.1").unwrap(), "abc"));
        assert_eq!(preferred_port("com.example.kvm"), preferred_port("com.example.kvm"));
        assert!((20000..60000).contains(&preferred_port("x")));
        assert_eq!(percent_decode(&percent_encode("/a b?c=1&d")), "/a b?c=1&d");
        let p = Proxy::start(up()).unwrap();
        assert!(p.auth_url("//evil.com").ends_with("&to=/"));
        assert_eq!(p.to_upstream(&format!("{}/kvm/?x", p.local_origin())).unwrap(), "https://pikvm.local/kvm/?x");
        assert_eq!(p.to_local("https://pikvm.local/a").unwrap(), format!("{}/a", p.local_origin()));
        assert_eq!(p.to_local("https://other.local/a"), None);
        assert_eq!(p.to_upstream("http://127.0.0.1:1/"), None);
    }
}
