//! Bundled models with the network switched off by the policy (`net.offline`): the first
//! start finds every bundled model in the install, and only models the installer lacks need
//! the network. Its own test binary, so switching the process-wide policy off cannot race
//! with other tests.

use desktop_core::models::{self, Lock, Mode, Resolver, Source, LOCK_FILE};
use desktop_core::net::{configure, NetConfig};
use sha2::Digest;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("models-offline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A tiny fake model: its bytes and its lock line, served from `url`.
fn fake(kind: &str, model: &str, file: &str, data: &[u8], url: &str) -> String {
    format!("{kind} {model} {file} {} {:x} {url}\n", data.len(), sha2::Sha256::digest(data))
}

fn put(dir: &Path, model: &str, file: &str, data: &[u8]) {
    let p = dir.join(model).join(file);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

/// Serves `body` to each of `n` requests on 127.0.0.1 (reachable even offline).
fn serve(body: &'static [u8], n: usize) -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/model.onnx", l.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for s in l.incoming().take(n) {
            let mut s = s.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(body).unwrap();
        }
    });
    url
}

#[test]
fn bundled_models_resolve_offline_and_fetch_checks_checksums() {
    configure(NetConfig { offline: true, ..NetConfig::default() });
    let root = tmp("resolve");
    let bundle = root.join("resources/models");
    let state = root.join("data/models");

    // The installer bundles the voice and an embedding model (Hugging Face cache layout).
    let lock = Lock::parse(&format!(
        "{}{}{}",
        fake("tts", "tiny-voca", "model.onnx", b"voice", "https://huggingface.co/x/model.onnx"),
        fake("tts", "tiny-voca", "lexicon.tsv", b"hello\th", "https://huggingface.co/x/lexicon.tsv"),
        fake("embed", "embed-bge-small-en-v1.5", "models--Xenova--bge-small-en-v1.5/snapshots/abc/onnx/model.onnx", b"embed", "https://huggingface.co/y/model.onnx"),
    ))
    .unwrap();
    lock.save(&bundle.join(LOCK_FILE)).unwrap();
    put(&bundle, "tiny-voca", "model.onnx", b"voice");
    put(&bundle, "tiny-voca", "lexicon.tsv", b"hello\th");
    put(&bundle, "embed-bge-small-en-v1.5", "models--Xenova--bge-small-en-v1.5/snapshots/abc/onnx/model.onnx", b"embed");

    let r = Resolver { mode: Mode::BundleWithFallback, bundle: Some(bundle.clone()), state_dir: state.clone() };
    // First start, offline: everything bundled is found, nothing is downloaded.
    assert_eq!(r.locate("tiny-voca", state.join("tiny-voca")).unwrap(), Source::Bundled(bundle.join("tiny-voca")));
    assert!(r.locate("embed-bge-small-en-v1.5", state.clone()).unwrap().is_bundled());
    assert!(state.join(".bundled/tiny-voca.ok").is_file(), "the checksum is checked once and recorded");
    assert!(r.bundled().iter().all(|m| m.present));
    // Again, from the marker.
    assert!(r.locate("tiny-voca", state.join("tiny-voca")).unwrap().is_bundled());

    // A model the installer does not bundle comes from the user's cache, and downloading it
    // is what the offline policy refuses.
    let user = state.join("whisper-base");
    assert_eq!(r.locate("whisper-base", user.clone()).unwrap(), Source::User(user));
    let err = desktop_core::net::download_to("https://huggingface.co/whisper/encoder.onnx", &root.join("enc"), &mut |_, _| {}).unwrap_err();
    assert!(err.contains("offline"), "{err}");

    // `bundle` with a file missing from the install: a clear error, not a download.
    std::fs::remove_file(bundle.join("tiny-voca/lexicon.tsv")).unwrap();
    let strict = Resolver { mode: Mode::Bundle, ..r.clone() };
    let e = strict.locate("tiny-voca", state.join("tiny-voca")).unwrap_err().to_string();
    assert!(e.contains("should be inside the app") && e.contains("lexicon.tsv"), "{e}");
    // `bundle_with_fallback` uses the user's cache (and downloads when allowed).
    assert!(!r.locate("tiny-voca", state.join("tiny-voca")).unwrap().is_bundled());

    // Fetching (`make models`): a local server stands in for Hugging Face. The right bytes
    // are kept; a server that sends other bytes fails the checksum.
    let url = serve(b"fetched!", 2);
    let fetch_dir = root.join("fetch");
    let good = Lock::parse(&fake("stt", "whisper-tiny", "encoder.onnx", b"fetched!", &url)).unwrap();
    let out = models::fetch(&good, &fetch_dir, &mut |_, _, _| {}).unwrap();
    assert!(out[0].starts_with("fetched"), "{out:?}");
    assert!(models::verify(&good, &fetch_dir, true).is_empty());
    let bad = Lock::parse(&fake("stt", "whisper-tiny", "decoder.onnx", b"something else", &url)).unwrap();
    let e = models::fetch(&bad, &fetch_dir, &mut |_, _, _| {}).unwrap_err().to_string();
    assert!(e.contains("after download"), "{e}");
    assert!(!fetch_dir.join("whisper-tiny/decoder.onnx").exists());

    configure(NetConfig::default());
    let _ = std::fs::remove_dir_all(&root);
}
