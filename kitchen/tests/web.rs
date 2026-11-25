//! Web app mode against a real self-signed HTTPS server on this computer: the page's
//! title, the certificate's SHA-256, icons from the manifest, refusal without
//! `--insecure-tls`, redirects and chunked bodies, `kitchen icon fetch`, and `kitchen new
//! --web` end to end (that one needs cargo-generate, and is skipped without it).

use kitchen::cli::{run, Args};
use kitchen::web::{check_url, fingerprint, probe, ICON_SIDE};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;

/// A test-only self-signed certificate for localhost and 127.0.0.1 (valid until 2126).
const CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIBmzCCAUGgAwIBAgIULYTHS11lt58t/GQoB1gn7ZUCMWwwCgYIKoZIzj0EAwIw\nFDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MTEyNDE4MzA1NFoYDzIxMjYxMDMx\nMTgzMDU0WjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjO\nPQMBBwNCAAS8OugwfDywdS/T1cflEU45jneuougiMN5n8Fo5ehrVFnuFbTWuOvZ2\nRlA6Xgjnbqx/NfIV3OrGWpvFwYMXbMFLo28wbTAdBgNVHQ4EFgQU+FlIXphm+FmV\nMUHHThEXzKPULqIwHwYDVR0jBBgwFoAU+FlIXphm+FmVMUHHThEXzKPULqIwDwYD\nVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwCgYIKoZI\nzj0EAwIDSAAwRQIhAJIvZDd2qPLCpMRsu8dWO9uO5GsNzMPspkvg5MWne6B2AiAk\nsiEqA2h9++o2raWt8UBNVpEh4tDxpxyOlxj6gI9MWw==\n-----END CERTIFICATE-----";
const KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgel3Pi1p6Tlcwz6R+\nIR6NphsGPId+QhVsmDtwhNgtPnGhRANCAAS8OugwfDywdS/T1cflEU45jneuougi\nMN5n8Fo5ehrVFnuFbTWuOvZ2RlA6Xgjnbqx/NfIV3OrGWpvFwYMXbMFL\n-----END PRIVATE KEY-----";

fn cert_der() -> CertificateDer<'static> {
    CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap()
}

fn png(side: u32) -> Vec<u8> {
    let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        side,
        side,
        image::Rgba([10, 120, 200, 255]),
    ));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// The site: `/` (title, manifest, apple-touch-icon, key logging script), `/r` (a redirect
/// to `/`), `/site.webmanifest`, `/i512.png` (sent chunked), `/apple.png` and `/favicon.ico`.
fn server() -> u16 {
    let key = PrivateKeyDer::from_pem_slice(KEY.as_bytes()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der()], key)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (big, apple, small) = (png(512), png(180), png(32));
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            let (cfg, big, apple, small) = (cfg.clone(), big.clone(), apple.clone(), small.clone());
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
                let page = "<!doctype html><html><head><title>Test &amp; KVM</title>\
                    <link rel=\"manifest\" href=\"/site.webmanifest\">\
                    <link rel=\"apple-touch-icon\" href=\"apple.png\" sizes=\"180x180\"></head>\
                    <body><pre id=log></pre><script>for (const t of ['keydown','keyup']) addEventListener(t, e => log.textContent += t + ' ' + e.code + '\\n');</script></body></html>";
                let manifest = r#"{"name":"Test","icons":[{"src":"/i512.png","sizes":"512x512","type":"image/png"}]}"#;
                let (status, extra, body): (&str, String, Vec<u8>) = match path.as_str() {
                    "/" => ("200 OK", String::new(), page.as_bytes().to_vec()),
                    "/r" => ("302 Found", "Location: /\r\n".into(), Vec::new()),
                    "/site.webmanifest" => ("200 OK", String::new(), manifest.as_bytes().to_vec()),
                    "/apple.png" => ("200 OK", String::new(), apple),
                    "/favicon.ico" => ("200 OK", String::new(), small),
                    "/i512.png" => {
                        let _ = write!(tls, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        for chunk in big.chunks(1000) {
                            let _ = write!(tls, "{:x}\r\n", chunk.len());
                            let _ = tls.write_all(chunk);
                            let _ = tls.write_all(b"\r\n");
                        }
                        let _ = tls.write_all(b"0\r\n\r\n");
                        let _ = tls.flush();
                        tls.conn.send_close_notify();
                        let _ = tls.flush();
                        return;
                    }
                    _ => ("404 Not Found", String::new(), Vec::new()),
                };
                let _ = write!(
                    tls,
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tls.write_all(&body);
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    port
}

#[test]
fn a_self_signed_site_gives_its_title_certificate_and_largest_icon() {
    let port = server();
    let url = check_url(&format!("https://localhost:{port}/r")).unwrap();
    let p = probe(&url, true).unwrap();
    assert_eq!(p.title.as_deref(), Some("Test & KVM"));
    assert_eq!(p.url.unwrap().path, "/", "the redirect was followed");
    assert_eq!(
        p.sha256.as_deref(),
        Some(fingerprint(cert_der().as_ref()).as_str())
    );
    assert!(!p.trusted);
    let (from, side) = p.icon_from.unwrap();
    assert!(
        from.ends_with("/i512.png"),
        "the manifest's 512px icon wins: {from}"
    );
    assert_eq!(side, 512);
    let icon = image::load_from_memory(&p.icon.unwrap()).unwrap();
    assert_eq!((icon.width(), icon.height()), (ICON_SIDE, ICON_SIDE));
}

#[test]
fn without_insecure_tls_the_certificate_is_refused() {
    let port = server();
    let url = check_url(&format!("https://localhost:{port}/")).unwrap();
    let e = probe(&url, false).unwrap_err().to_string();
    assert!(
        e.contains("not trusted") && e.contains("--insecure-tls"),
        "{e}"
    );
}

#[test]
fn icon_fetch_writes_a_square_png() {
    let port = server();
    let dir = std::env::temp_dir().join(format!("kitchen-icon-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let r = run(
        &Args::parse([
            "icon",
            "fetch",
            &format!("https://127.0.0.1:{port}/"),
            "--insecure-tls",
            "--out",
            "icon.png",
        ]),
        &dir,
    )
    .unwrap();
    assert!(r.lines.iter().any(|l| l.contains("i512.png")), "{r}");
    let img = image::open(dir.join("icon.png")).unwrap();
    assert_eq!(img.width(), ICON_SIDE);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_web_app_end_to_end() {
    let have = |p: &str, a: &[&str]| {
        std::process::Command::new(p)
            .args(a)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !have("git", &["--version"]) || !have("cargo", &["generate", "--version"]) {
        eprintln!("skipped: needs git and cargo-generate");
        return;
    }
    let port = server();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let work = std::env::temp_dir().join(format!("kitchen-web-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let url = format!("https://localhost:{port}/");
    let r = run(
        &Args::parse([
            "new",
            "kvm",
            "--source",
            &repo.display().to_string(),
            "--no-git",
            "--web",
            &url,
            "--insecure-tls",
            "--capture-keys",
            "--release-shortcut",
            "Ctrl+Alt+F12",
            "--muted",
        ]),
        &work,
    )
    .unwrap();
    let text = r.to_string();
    assert!(text.contains("certificate SHA-256"), "{text}");
    let app = work.join("kvm");
    let toml = std::fs::read_to_string(app.join("kitchen.toml")).unwrap();
    for want in [
        format!("web_url = \"{url}\""),
        format!("web_cert_sha256 = \"{}\"", fingerprint(cert_der().as_ref())),
        "app_name = \"Test KVM\"".to_string(),
        "identifier = \"com.example.kvm\"".to_string(),
        "web_wrapper = true".into(),
        "capture_keys = true".into(),
        "web_muted = true".into(),
        "web_microphone = false".into(),
        "release_shortcut = \"Ctrl+Alt+F12\"".into(),
    ] {
        assert!(toml.contains(&want), "{want} not in\n{toml}");
    }
    let settings = std::fs::read_to_string(app.join("config/settings.json")).unwrap();
    assert!(
        settings.contains(&fingerprint(cert_der().as_ref())),
        "the pin is in the settings"
    );
    // The flags are the developer's settings, built in from config/settings.json.
    let wrapper: serde_json::Value =
        serde_json::from_str::<serde_json::Value>(&settings).unwrap()["wrapper"].clone();
    assert_eq!(wrapper["url"], url.as_str());
    assert_eq!(wrapper["insecureTls"], true);
    assert_eq!(wrapper["captureKeys"], true);
    assert_eq!(wrapper["releaseShortcut"], "Ctrl+Alt+F12");
    assert_eq!(wrapper["muted"], true);
    assert_eq!(wrapper["microphone"], false);
    assert_eq!(
        wrapper["userEditable"],
        serde_json::json!(["releaseShortcut", "zoom", "muted", "startCaptured"]),
        "the user may change only these"
    );
    assert!(app.join("src-tauri/src/wrapper/mod.rs").is_file());
    assert!(app.join("src-tauri/icons/source-1024.png").is_file());
    let _ = std::fs::remove_dir_all(&work);
}
