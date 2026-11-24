//! The loopback proxy against a real self-signed HTTPS and WebSocket server on this
//! computer: header injection, pinning (a mismatch is refused), the WebSocket echo, and
//! `Set-Cookie` rewritten so the window keeps the cookie across requests.
#![cfg(feature = "web-wrapper")]

use desktop_core::wrapper::pin::{fingerprint, PinStore};
use desktop_core::wrapper::proxy::{tls_config, Proxy, Upstream};
use desktop_core::wrapper::url::WebUrl;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// A test-only self-signed certificate for localhost and 127.0.0.1 (valid until 2126).
const CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIBmzCCAUGgAwIBAgIULYTHS11lt58t/GQoB1gn7ZUCMWwwCgYIKoZIzj0EAwIw\nFDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MTEyNDE4MzA1NFoYDzIxMjYxMDMx\nMTgzMDU0WjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjO\nPQMBBwNCAAS8OugwfDywdS/T1cflEU45jneuougiMN5n8Fo5ehrVFnuFbTWuOvZ2\nRlA6Xgjnbqx/NfIV3OrGWpvFwYMXbMFLo28wbTAdBgNVHQ4EFgQU+FlIXphm+FmV\nMUHHThEXzKPULqIwHwYDVR0jBBgwFoAU+FlIXphm+FmVMUHHThEXzKPULqIwDwYD\nVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwCgYIKoZI\nzj0EAwIDSAAwRQIhAJIvZDd2qPLCpMRsu8dWO9uO5GsNzMPspkvg5MWne6B2AiAk\nsiEqA2h9++o2raWt8UBNVpEh4tDxpxyOlxj6gI9MWw==\n-----END CERTIFICATE-----";
const KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgel3Pi1p6Tlcwz6R+\nIR6NphsGPId+QhVsmDtwhNgtPnGhRANCAAS8OugwfDywdS/T1cflEU45jneuougi\nMN5n8Fo5ehrVFnuFbTWuOvZ2RlA6Xgjnbqx/NfIV3OrGWpvFwYMXbMFL\n-----END PRIVATE KEY-----";

fn cert_der() -> CertificateDer<'static> {
    CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap()
}

/// A tiny HTTPS server: `/headers` echoes the request head, `/login` sets a cookie, `/me`
/// echoes the Cookie header, and an Upgrade request gets 101 and then an echo.
fn server() -> u16 {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let key = PrivateKeyDer::from_pem_slice(KEY.as_bytes()).unwrap();
    let cfg = Arc::new(rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert_der()], key).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            let cfg = cfg.clone();
            std::thread::spawn(move || {
                let conn = rustls::ServerConnection::new(cfg).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, sock);
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if tls.read(&mut b).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(b[0]);
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let path = text.split(' ').nth(1).unwrap_or("/").to_string();
                let cookie = text.lines().find_map(|l| l.strip_prefix("Cookie: ")).unwrap_or("").to_string();
                if text.to_ascii_lowercase().contains("upgrade: websocket") {
                    let _ = tls.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n");
                    let _ = tls.flush();
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = tls.read(&mut buf) {
                        if n == 0 || tls.write_all(&buf[..n]).is_err() || tls.flush().is_err() {
                            break;
                        }
                    }
                    return;
                }
                let (extra, body) = match path.as_str() {
                    "/login" => (
                        format!(
                            "Set-Cookie: sid=abc; Domain=localhost; Path=/; Secure; HttpOnly\r\nLocation: https://localhost:{port}/me\r\n"
                        ),
                        String::new(),
                    ),
                    "/me" => (String::new(), cookie),
                    _ => (String::new(), text),
                };
                let _ = write!(tls, "HTTP/1.1 200 OK\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    port
}

fn proxy(site_port: u16, pin: Option<String>) -> Proxy {
    let store = PinStore::new(true, pin, |_| {});
    Proxy::start(Upstream {
        site: WebUrl::parse(&format!("https://localhost:{site_port}/")).unwrap(),
        tls: Arc::new(tls_config(Some(store))),
        headers: Arc::new(|url: &str| if url.contains("/headers") { vec![("X-Api-Key".to_string(), "k-123".to_string())] } else { vec![] }),
        user_agent: Some("KitchenTest/1".into()),
        port: 0,
    })
    .unwrap()
}

/// Sign in to the proxy (the window's first navigation) and return the token cookie.
fn sign_in(p: &Proxy) -> String {
    let url = p.auth_url("/headers");
    let path = url.strip_prefix(&p.local_origin()).unwrap();
    let reply = send(p, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n", p.port()));
    assert!(reply.starts_with("HTTP/1.1 302"), "{reply}");
    assert!(reply.contains("Location: /headers"), "{reply}");
    let line = reply.lines().find(|l| l.starts_with("Set-Cookie: ")).unwrap();
    line["Set-Cookie: ".len()..].split(';').next().unwrap().to_string()
}

fn send(p: &Proxy, req: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", p.port())).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn headers_are_added_and_strangers_are_refused() {
    let site = server();
    let p = proxy(site, Some(fingerprint(&cert_der())));
    let host = format!("Host: 127.0.0.1:{}", p.port());
    let no_token = send(&p, &format!("GET /headers HTTP/1.1\r\n{host}\r\n\r\n"));
    assert!(no_token.starts_with("HTTP/1.1 403"), "{no_token}");
    let rebinding = send(&p, "GET /headers HTTP/1.1\r\nHost: evil.example\r\n\r\n");
    assert!(rebinding.starts_with("HTTP/1.1 421"), "{rebinding}");
    let token = sign_in(&p);
    let reply = send(
        &p,
        &format!("GET /headers HTTP/1.1\r\n{host}\r\nCookie: {token}\r\nUser-Agent: WebKit\r\nOrigin: {}\r\n\r\n", p.local_origin()),
    );
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("X-Api-Key: k-123"), "{reply}");
    assert!(reply.contains("User-Agent: KitchenTest/1"), "{reply}");
    assert!(reply.contains(&format!("Host: localhost:{site}")), "{reply}");
    assert!(reply.contains(&format!("Origin: https://localhost:{site}")), "{reply}");
    assert!(!reply.contains("__kitchen_proxy"), "the token never goes upstream: {reply}");
}

#[test]
fn a_different_certificate_is_refused() {
    let site = server();
    let p = proxy(site, Some(fingerprint(b"some other certificate")));
    let token = sign_in(&p);
    let reply = send(&p, &format!("GET /headers HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: {token}\r\n\r\n", p.port()));
    assert!(reply.starts_with("HTTP/1.1 502"), "{reply}");
    assert!(!reply.contains("X-Api-Key"), "{reply}");
}

#[test]
fn first_use_pins_the_certificate() {
    let site = server();
    let saved = Arc::new(std::sync::Mutex::new(None));
    let s2 = saved.clone();
    let store = PinStore::new(true, None, move |p| *s2.lock().unwrap() = p.map(String::from));
    let p = Proxy::start(Upstream {
        site: WebUrl::parse(&format!("https://localhost:{site}/")).unwrap(),
        tls: Arc::new(tls_config(Some(store))),
        headers: Arc::new(|_: &str| vec![]),
        user_agent: None,
        port: 0,
    })
    .unwrap();
    let token = sign_in(&p);
    let reply = send(&p, &format!("GET /headers HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: {token}\r\n\r\n", p.port()));
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert_eq!(saved.lock().unwrap().clone(), Some(fingerprint(&cert_der())));
}

#[test]
fn cookies_are_kept_across_requests() {
    let site = server();
    let p = proxy(site, Some(fingerprint(&cert_der())));
    let token = sign_in(&p);
    let host = format!("Host: 127.0.0.1:{}", p.port());
    let login = send(&p, &format!("GET /login HTTP/1.1\r\n{host}\r\nCookie: {token}\r\n\r\n"));
    let set = login.lines().find(|l| l.starts_with("Set-Cookie: ")).unwrap();
    assert_eq!(set, "Set-Cookie: sid=abc; Path=/; HttpOnly", "no Domain, no Secure on the loopback leg");
    assert!(login.contains("Location: /me"), "{login}");
    let me = send(&p, &format!("GET /me HTTP/1.1\r\n{host}\r\nCookie: {token}; sid=abc\r\n\r\n"));
    assert!(me.ends_with("sid=abc"), "the site gets its cookie back, without the token: {me}");
}

#[test]
fn websockets_are_proxied() {
    let site = server();
    let p = proxy(site, Some(fingerprint(&cert_der())));
    let token = sign_in(&p);
    let mut s = TcpStream::connect(("127.0.0.1", p.port())).unwrap();
    write!(s, "GET /api/ws HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: {token}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n", p.port()).unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"));
    for msg in [&b"\x81\x05hello"[..], b"\x81\x05again"] {
        s.write_all(msg).unwrap();
        let mut echo = vec![0u8; msg.len()];
        s.read_exact(&mut echo).unwrap();
        assert_eq!(echo, msg);
    }
}
