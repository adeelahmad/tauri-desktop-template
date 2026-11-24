//! The one place this crate makes HTTP requests, and the network policy every request obeys.
//!
//! The policy (`net` in settings) can switch the app offline (only this computer stays
//! reachable), limit it to an allowlist of hosts or block a denylist, pick the proxy (the
//! system's `HTTPS_PROXY`/`ALL_PROXY` by default, a manual one, or none), trust a company's
//! CA certificates instead of the bundled Mozilla roots (for TLS-inspecting proxies), and
//! pin hosts to the CA certificates that alone may vouch for them. Redirects are followed
//! here, one hop at a time, so every hop is checked against the policy. Child processes get
//! the same proxy and offline settings through `apply_env`. Requests go out over HTTPS,
//! except plain HTTP to this computer.
//!
//! Main items: [`configure`] and [`check`] (the policy), [`agent`], [`get`], [`get_bytes`],
//! [`download_to`], [`post_json`] (requests), [`apply_env`] (child processes).
//!
//! # Example
//!
//! ```
//! use desktop_core::net::{check, configure, NetConfig};
//!
//! configure(NetConfig { deny: vec!["*.example.com".into()], ..NetConfig::default() });
//! assert!(check("https://api.example.com/v1").is_err());
//! assert!(check("https://example.org/").is_ok());
//! assert!(check("http://example.org/").is_err()); // plain HTTP only to this computer
//! assert!(check("http://127.0.0.1:8080/").is_ok());
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

const MAX_REDIRECTS: usize = 8;
const USER_AGENT: &str = concat!("desktop-core/", env!("CARGO_PKG_VERSION"));

/// The network policy (`net` in settings). The default allows everything through the
/// system proxy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NetConfig {
    /// Block every request that leaves this computer.
    pub offline: bool,
    /// Only these hosts (empty allows any). `example.com` is that host only; `*.example.com`
    /// is the host and every subdomain.
    pub allow: Vec<String>,
    /// Never these hosts (same patterns as `allow`).
    pub deny: Vec<String>,
    /// Empty: the system proxy (`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`). `direct`: no proxy.
    /// Otherwise a proxy URL (`http://`, `https://`, `socks5://`, with optional user:pass@).
    pub proxy: String,
    /// Hosts reached without the proxy (same patterns as `allow`).
    pub no_proxy: Vec<String>,
    /// CA certificate files (PEM) to trust instead of the bundled Mozilla roots, e.g. the
    /// certificate of a company's TLS-inspecting proxy.
    pub ca_files: Vec<PathBuf>,
    /// Host pattern → CA certificates (PEM text or a PEM file) that alone may vouch for it.
    pub pins: BTreeMap<String, Vec<String>>,
    /// Ignore `pins` (behind a TLS-inspecting proxy that re-signs every site).
    pub pins_off: bool,
}

impl NetConfig {
    /// Host patterns trimmed and lower-cased; empty entries dropped.
    pub fn validated(mut self) -> Self {
        let clean = |v: &mut Vec<String>| {
            *v = v.iter().map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase()).filter(|h| !h.is_empty()).collect();
        };
        clean(&mut self.allow);
        clean(&mut self.deny);
        clean(&mut self.no_proxy);
        self.proxy = self.proxy.trim().to_string();
        if !(self.proxy.is_empty() || self.proxy == "direct" || ureq::Proxy::new(&self.proxy).is_ok()) {
            log::warn!("ignoring an unusable proxy setting; using the system proxy");
            self.proxy.clear();
        }
        self.pins = self.pins.into_iter().map(|(h, c)| (h.trim().to_ascii_lowercase(), c)).filter(|(h, c)| !h.is_empty() && !c.is_empty()).collect();
        self
    }
}

fn policy_lock() -> &'static RwLock<NetConfig> {
    static POLICY: OnceLock<RwLock<NetConfig>> = OnceLock::new();
    POLICY.get_or_init(|| RwLock::new(NetConfig::default()))
}

/// Apply a new policy to every later request.
pub fn configure(config: NetConfig) {
    if let Ok(mut p) = policy_lock().write() {
        *p = config.validated();
    }
}

/// The policy in force.
pub fn policy() -> NetConfig {
    policy_lock().read().map(|p| p.clone()).unwrap_or_default()
}

/// Scheme and lower-case host of an absolute URL (IPv6 keeps its brackets).
fn split(url: &str) -> Option<(&str, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next()?
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, p)| if p.chars().all(|c| c.is_ascii_digit()) { h } else { authority })
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some((scheme, host))
}

/// This computer: `localhost`, 127.0.0.0/8 or `[::1]`.
fn is_local(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host == "[::1]" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}

/// `pattern` is `host` exactly, or `*.domain` for the domain and its subdomains.
fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.')),
        None => pattern == "*" || pattern == host,
    }
}

fn any_match(patterns: &[String], host: &str) -> bool {
    patterns.iter().any(|p| host_matches(p, host))
}

/// Whether the policy lets this URL be requested; the error says why not.
pub fn check(url: &str) -> Result<(), String> {
    check_with(url, false)
}

/// `check`, with plain HTTP allowed to any host when `any_http` (a caller that keeps its
/// own list of HTTP hosts, such as a model server on the local network).
fn check_with(url: &str, any_http: bool) -> Result<(), String> {
    let Some((scheme, host)) = split(url) else { return Err(format!("not a URL: {url}")) };
    let local = is_local(&host);
    match scheme {
        "https" => {}
        "http" if local || any_http => {}
        _ => return Err(format!("refusing a non-HTTPS URL: {}", redact(url))),
    }
    check_host(&host)
}

/// Whether the policy lets this host be contacted at all, for clients that do not speak
/// HTTP (mail, sockets). This computer is always reachable.
pub fn check_host(host: &str) -> Result<(), String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if is_local(&host) {
        return Ok(());
    }
    let p = policy_lock().read().map_err(|_| "network policy lock poisoned".to_string())?;
    if p.offline {
        Err(format!("offline mode is on, so {host} was not contacted"))
    } else if any_match(&p.deny, &host) {
        Err(format!("{host} is blocked by the network policy"))
    } else if !p.allow.is_empty() && !any_match(&p.allow, &host) {
        Err(format!("{host} is not on the network allowlist"))
    } else {
        Ok(())
    }
}

/// The URL without its query string (which may carry a key).
fn redact(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

fn read_pem(source: &str) -> Result<Vec<ureq::tls::Certificate<'static>>, String> {
    let text = if source.trim_start().starts_with("-----BEGIN") {
        source.as_bytes().to_vec()
    } else {
        std::fs::read(source).map_err(|e| format!("CA file {source}: {e}"))?
    };
    let certs: Vec<_> = ureq::tls::parse_pem(&text)
        .filter_map(|item| match item {
            Ok(ureq::tls::PemItem::Certificate(c)) => Some(c),
            _ => None,
        })
        .collect();
    if certs.is_empty() {
        return Err(format!("no certificate in {}", if text.len() > 200 { "the PEM text" } else { source }));
    }
    Ok(certs)
}

/// The certificates that may vouch for `host` under the policy: its pins, else the CA
/// files, else `None` for the bundled Mozilla roots. Any TLS client should use this.
pub fn trusted_roots(host: &str) -> Result<Option<Vec<ureq::tls::Certificate<'static>>>, String> {
    let p = policy();
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let pinned = if p.pins_off { None } else { p.pins.iter().find(|(pat, _)| host_matches(pat, &host)).map(|(_, c)| c.clone()) };
    let read_all = |sources: &mut dyn Iterator<Item = String>| sources.map(|s| read_pem(&s)).collect::<Result<Vec<_>, _>>().map(|v| v.concat());
    Ok(match pinned {
        Some(sources) => Some(read_all(&mut sources.into_iter())?),
        None if !p.ca_files.is_empty() => Some(read_all(&mut p.ca_files.iter().map(|f| f.to_string_lossy().into_owned()))?),
        None => None,
    })
}

/// Timeouts for one request.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Time to connect (TCP and TLS).
    pub connect: Duration,
    /// The whole request, body included.
    pub total: Option<Duration>,
    /// Between bytes of the body (long downloads).
    pub idle: Option<Duration>,
}

impl Limits {
    /// Connect within `connect_secs`, finish within `total_secs`.
    pub const fn total(connect_secs: u64, total_secs: u64) -> Self {
        Self { connect: Duration::from_secs(connect_secs), total: Some(Duration::from_secs(total_secs)), idle: None }
    }
}

/// An agent for one URL under the policy: its proxy, its trusted roots or pins, no
/// automatic redirects, and statuses returned rather than raised. Checks the URL first.
pub fn agent(url: &str, limits: Limits) -> Result<ureq::Agent, String> {
    agent_with(url, limits, false)
}

/// `agent` for a URL whose plain-HTTP host the caller has already approved.
pub fn agent_http(url: &str, limits: Limits) -> Result<ureq::Agent, String> {
    agent_with(url, limits, true)
}

fn agent_with(url: &str, limits: Limits, any_http: bool) -> Result<ureq::Agent, String> {
    check_with(url, any_http)?;
    let (_, host) = split(url).ok_or_else(|| format!("not a URL: {url}"))?;
    let p = policy();
    let proxy = if is_local(&host) || any_match(&p.no_proxy, &host) || p.proxy == "direct" {
        None
    } else if p.proxy.is_empty() {
        ureq::Proxy::try_from_env()
    } else {
        Some(ureq::Proxy::new(&p.proxy).map_err(|e| e.to_string())?)
    };
    let roots = trusted_roots(&host)?;
    let mut tls = ureq::tls::TlsConfig::builder();
    if let Some(roots) = roots {
        tls = tls.root_certs(ureq::tls::RootCerts::new_with_certs(&roots));
    }
    Ok(ureq::Agent::config_builder()
        .proxy(proxy)
        .tls_config(tls.build())
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(limits.connect))
        .timeout_global(limits.total)
        .timeout_recv_body(limits.idle)
        .user_agent(USER_AGENT)
        .build()
        .into())
}

/// Where a redirect points, resolved against the URL that sent it.
fn resolve(base: &str, location: &str) -> Option<String> {
    if location.contains("://") {
        return Some(location.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if let Some(path) = location.strip_prefix("//") {
        return Some(format!("{scheme}://{path}"));
    }
    if location.starts_with('/') {
        return Some(format!("{scheme}://{authority}{location}"));
    }
    let path = &rest[authority.len()..];
    let dir = redact(path).rsplit_once('/').map_or("", |(d, _)| d);
    Some(format!("{scheme}://{authority}{dir}/{location}"))
}

/// GET `url`, following redirects one checked hop at a time.
pub fn get(url: &str, headers: &[(&str, &str)], limits: Limits) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let mut req = agent(&url, limits)?.get(&url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.call().map_err(|e| format!("{}: {e}", redact(&url)))?;
        if !resp.status().is_redirection() {
            return Ok(resp);
        }
        let location = resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or_else(|| format!("redirect without a location from {}", redact(&url)))?;
        url = resolve(&url, location).ok_or_else(|| format!("bad redirect from {}", redact(&url)))?;
    }
    Err(format!("too many redirects for {}", redact(&url)))
}

/// GET `url` and return at most `max_bytes` of body; any status but 2xx is an error.
pub fn get_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let mut resp = get(url, &[], Limits::total(20, 600))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} for {}", redact(url)));
    }
    resp.body_mut().with_config().limit(max_bytes).read_to_vec().map_err(|e| e.to_string())
}

/// Stream `url` into `dest`, resuming a partial `<dest>.part` and renaming it into place
/// when complete. `progress(bytes_so_far, total)` runs as data arrives.
pub fn download_to(url: &str, dest: &std::path::Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    use std::io::{Read, Write};
    check(url)?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = dest.with_extension(format!("{}part", dest.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()));
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let range = format!("bytes={have}-");
    let headers: &[(&str, &str)] = if have > 0 { &[("range", &range)] } else { &[] };
    let limits = Limits { connect: Duration::from_secs(20), total: None, idle: Some(Duration::from_secs(120)) };
    let resp = get(url, headers, limits)?;
    let status = resp.status().as_u16();
    let resumed = status == 206;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} for {}", redact(url)));
    }
    let len = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mut done = if resumed { have } else { 0 };
    let total = len.map(|l| l + done);
    let mut file = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part).map_err(|e| e.to_string())?;
    let mut body = resp.into_body().into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut last = 0u64;
    loop {
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if done - last >= 1 << 20 {
            last = done;
            progress(done, total);
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if total.is_some_and(|t| t != done) {
        return Err(format!("download stopped at {done} of {} bytes; try again to resume", total.unwrap_or(0)));
    }
    progress(done, total);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

/// POST `body` to `url` with extra `headers`; any 2xx is success. Redirects are not followed.
pub fn post(url: &str, headers: &[(String, String)], body: &[u8]) -> Result<(), String> {
    let mut req = agent(url, Limits::total(10, 30))?.post(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let status = req.send(body).map_err(|e| e.to_string())?.status().as_u16();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("HTTP {status} from {}", redact(url)))
    }
}

/// POST JSON (see `post`).
pub fn post_json(url: &str, headers: &[(String, String)], body: &serde_json::Value) -> Result<(), String> {
    let mut h = headers.to_vec();
    h.push(("Content-Type".into(), "application/json".into()));
    post(url, &h, body.to_string().as_bytes())
}

/// Environment for a child process (a Python worker, uv, rclone) so it follows the same
/// policy where it can: the proxy, offline mode (a proxy that goes nowhere, plus the
/// offline switches of Hugging Face and uv) and the CA file. `None` removes the variable.
/// The allow and deny lists cannot be enforced inside another program.
pub fn child_env() -> Vec<(&'static str, Option<String>)> {
    const PROXIES: [&str; 6] = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"];
    let p = policy();
    let mut env = Vec::new();
    let local = "localhost,127.0.0.1,::1".to_string();
    let no_proxy = || std::iter::once(local.clone()).chain(p.no_proxy.iter().map(|h| h.trim_start_matches('*').to_string())).collect::<Vec<_>>().join(",");
    if p.offline {
        // Port 9 (discard) on this computer: every proxied request fails at once.
        env.extend(PROXIES.map(|k| (k, Some("http://127.0.0.1:9".to_string()))));
        env.extend(["NO_PROXY", "no_proxy"].map(|k| (k, Some(local.clone()))));
        env.extend(["HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "UV_OFFLINE"].map(|k| (k, Some("1".to_string()))));
    } else if p.proxy == "direct" {
        env.extend(PROXIES.map(|k| (k, None)));
    } else if !p.proxy.is_empty() {
        env.extend(PROXIES.map(|k| (k, Some(p.proxy.clone()))));
        env.extend(["NO_PROXY", "no_proxy"].map(|k| (k, Some(no_proxy()))));
    }
    if let [ca] = p.ca_files.as_slice() {
        let ca = ca.to_string_lossy().into_owned();
        env.extend(["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"].map(|k| (k, Some(ca.clone()))));
    }
    env
}

/// Give `cmd` the policy's environment (see `child_env`).
pub fn apply_env(cmd: &mut std::process::Command) -> &mut std::process::Command {
    for (k, v) in child_env() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_schemes() {
        assert_eq!(split("https://user:pw@Example.COM:8443/a?b"), Some(("https", "example.com".into())));
        assert_eq!(split("http://[::1]:11434/api"), Some(("http", "[::1]".into())));
        assert!(is_local("127.0.0.2") && is_local("localhost") && is_local("[::1]") && !is_local("localhost.evil.com"));
        assert!(host_matches("*.example.com", "example.com") && host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "badexample.com") && !host_matches("example.com", "a.example.com"));
        assert_eq!(resolve("https://a.com/x/y?q=1", "z").as_deref(), Some("https://a.com/x/z"));
        assert_eq!(resolve("https://a.com/x/y", "/r").as_deref(), Some("https://a.com/r"));
        assert_eq!(resolve("https://a.com/x", "//cdn.com/f").as_deref(), Some("https://cdn.com/f"));
    }

    // One test touches the global policy, so the cases run in order.
    #[test]
    fn policy_blocks_and_allows() {
        assert!(check("https://x.example/a").is_ok());
        assert!(check("http://localhost:4318/v1/traces").is_ok());
        assert!(check("http://127.0.0.1/v1").is_ok());
        assert!(check("http://example.com/v1").is_err());
        assert!(check("http://localhost.evil.com/v1").is_err());
        assert!(check("ftp://localhost/").is_err());

        configure(NetConfig { allow: vec!["*.GitHub.com".into(), "huggingface.co".into()], deny: vec!["gist.github.com".into()], ..NetConfig::default() });
        assert!(check("https://objects.github.com/x").is_ok());
        assert!(check("https://huggingface.co/x").is_ok());
        assert!(check("https://gist.github.com/x").unwrap_err().contains("blocked"));
        assert!(check("https://cdn.huggingface.co/x").unwrap_err().contains("allowlist"));

        configure(NetConfig { offline: true, ..NetConfig::default() });
        assert!(check("https://huggingface.co/x").unwrap_err().contains("offline"));
        assert!(check("http://127.0.0.1:11434/api/chat").is_ok());
        let env = child_env();
        assert!(env.iter().any(|(k, v)| *k == "HF_HUB_OFFLINE" && v.as_deref() == Some("1")));

        configure(NetConfig { proxy: "not a proxy".into(), pins: [("*.example.com".to_string(), vec!["/nonexistent.pem".to_string()])].into(), ..NetConfig::default() });
        assert_eq!(policy().proxy, "");
        assert!(agent("https://api.example.com/", Limits::total(1, 1)).unwrap_err().contains("CA file"));
        configure(NetConfig { pins_off: true, ..policy() });
        assert!(agent("https://api.example.com/", Limits::total(1, 1)).is_ok());

        configure(NetConfig::default());
    }

    /// Redirects are followed hop by hop, and a hop to a blocked host stops there.
    #[test]
    fn redirects_are_checked() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (i, s) in listener.incoming().take(2).enumerate() {
                let mut s = s.unwrap();
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let reply = if i == 0 {
                    format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                } else {
                    "HTTP/1.1 302 Found\r\nLocation: http://example.com/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                };
                s.write_all(reply.as_bytes()).unwrap();
            }
        });
        let err = get_bytes(&format!("http://127.0.0.1:{port}/start"), 1024).unwrap_err();
        assert!(err.contains("non-HTTPS"), "{err}");
    }
}
