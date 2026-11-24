//! Certificate pinning for a site with a self-signed certificate (a PiKVM, a NAS, a
//! router): trust on first use, by the SHA-256 of the certificate.
//!
//! With `insecureTls` on, the first certificate the site shows is pinned (the user sees
//! its fingerprint in Preferences, and `kitchen new --web` shows it when the app is made).
//! After that only the same certificate is accepted; a different one is refused until the
//! user chooses "Forget" in Preferences. Without `insecureTls` the system's normal checks
//! apply and nothing here is used.
//!
//! The per-OS hooks (WKWebView, WebView2, WebKitGTK) and the proxy all ask [`decide`].
//!
//! ```
//! use desktop_core::wrapper::pin::{decide, fingerprint, Decision};
//!
//! let cert = b"the DER bytes of the site's certificate";
//! let fp = fingerprint(cert);
//! assert_eq!(decide(true, None, cert), Decision::PinNow(fp.clone())); // first use: pin it
//! assert_eq!(decide(true, Some(&fp), cert), Decision::Trust); // the same one again
//! assert!(matches!(decide(true, Some(&fp), b"another"), Decision::Mismatch { .. }));
//! assert_eq!(decide(false, None, cert), Decision::SystemCheck); // insecureTls is off
//! ```

use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex, PoisonError};

/// What to do with a certificate the system does not trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// `insecureTls` is off: leave it to the system (which refuses it).
    SystemCheck,
    /// No pin yet: accept it and pin this fingerprint.
    PinNow(String),
    /// It is the pinned certificate.
    Trust,
    /// Another certificate than the pinned one: refuse.
    Mismatch {
        /// The pinned fingerprint.
        expected: String,
        /// What the site showed.
        got: String,
    },
}

/// SHA-256 of `der`, as 64 lower-case hex digits.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect()
}

/// `AB:CD:...` for people (the form browsers show).
pub fn display(fp: &str) -> String {
    fp.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).to_ascii_uppercase()).collect::<Vec<_>>().join(":")
}

/// A fingerprint as given by a person (`AB:CD:..`, spaces, either case) in the stored
/// form, or an error.
pub fn normalise(fp: &str) -> Result<String, String> {
    let hex: String = fp.chars().filter(|c| !matches!(c, ':' | ' ' | '-')).collect::<String>().to_ascii_lowercase();
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(hex)
    } else {
        Err("a SHA-256 fingerprint is 64 hex digits".into())
    }
}

/// The decision for the certificate `der` (the site's own, the first in its chain).
pub fn decide(insecure_tls: bool, pinned: Option<&str>, der: &[u8]) -> Decision {
    if !insecure_tls {
        return Decision::SystemCheck;
    }
    let got = fingerprint(der);
    match pinned {
        None => Decision::PinNow(got),
        Some(p) if p == got => Decision::Trust,
        Some(p) => Decision::Mismatch { expected: p.to_string(), got },
    }
}

/// The DER bytes of the first certificate in a PEM text (WebView2 hands certificates over
/// as PEM).
pub fn der_from_pem(pem: &str) -> Option<Vec<u8>> {
    let body = pem.split("-----BEGIN CERTIFICATE-----").nth(1)?.split("-----END CERTIFICATE-----").next()?;
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in body.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The pin shared by the window's TLS hook and the proxy, with a callback that saves a
/// new pin (to the user's settings).
#[derive(Clone)]
pub struct PinStore {
    inner: Arc<Mutex<Option<String>>>,
    insecure: bool,
    save: Arc<SaveFn>,
}

/// Saves a new pin (`None`: forgotten).
type SaveFn = dyn Fn(Option<&str>) + Send + Sync;

impl std::fmt::Debug for PinStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinStore").field("pin", &self.get()).field("insecure", &self.insecure).finish()
    }
}

impl PinStore {
    /// A store starting at `pin`, calling `save` whenever the pin changes.
    pub fn new(insecure: bool, pin: Option<String>, save: impl Fn(Option<&str>) + Send + Sync + 'static) -> Self {
        Self { inner: Arc::new(Mutex::new(pin)), insecure, save: Arc::new(save) }
    }

    /// The pinned fingerprint.
    pub fn get(&self) -> Option<String> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Forget the pin: the next certificate is pinned anew.
    pub fn forget(&self) {
        *self.inner.lock().unwrap_or_else(PoisonError::into_inner) = None;
        (self.save)(None);
    }

    /// Decide for `der`, pinning it on first use. `true` accepts the certificate.
    pub fn accept(&self, der: &[u8]) -> bool {
        let mut pin = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match decide(self.insecure, pin.as_deref(), der) {
            Decision::SystemCheck => false,
            Decision::Trust => true,
            Decision::PinNow(fp) => {
                log::warn!("pinned the site's certificate on first use: SHA-256 {}", display(&fp));
                (self.save)(Some(&fp));
                *pin = Some(fp);
                true
            }
            Decision::Mismatch { expected, got } => {
                log::error!(
                    "refused the site's certificate: SHA-256 {} is not the pinned {} (Preferences → Forget to trust a new one)",
                    display(&got),
                    display(&expected)
                );
                false
            }
        }
    }
}

/// A rustls verifier for the proxy: the system-independent half of [`PinStore`]. It checks
/// the handshake signature with the certificate's key (so the server holds that key), then
/// accepts the certificate only by its pin.
#[derive(Debug)]
pub struct PinVerifier {
    store: PinStore,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinVerifier {
    /// A verifier over `store`.
    pub fn new(store: PinStore) -> Self {
        Self { store, provider: Arc::new(rustls::crypto::ring::default_provider()) }
    }
}

impl rustls::client::danger::ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if self.store.accept(end_entity.as_ref()) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("the certificate is not the pinned one".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_sha256_hex() {
        let fp = fingerprint(b"abc");
        assert_eq!(fp, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(&display(&fp)[..8], "BA:78:16");
        assert_eq!(normalise(&display(&fp)).unwrap(), fp);
        assert_eq!(normalise(&fp.to_uppercase()).unwrap(), fp);
        assert!(normalise("abc").is_err());
        assert!(normalise(&"g".repeat(64)).is_err());
    }

    #[test]
    fn pem_to_der() {
        let pem = "-----BEGIN CERTIFICATE-----\nAAEC\nAwQ=\n-----END CERTIFICATE-----\n";
        assert_eq!(der_from_pem(pem).unwrap(), [0, 1, 2, 3, 4]);
        assert_eq!(der_from_pem("-----BEGIN CERTIFICATE-----\naGk=\n-----END CERTIFICATE-----").unwrap(), b"hi");
        assert!(der_from_pem("no pem").is_none());
        assert!(der_from_pem("-----BEGIN CERTIFICATE-----\n*!\n-----END CERTIFICATE-----").is_none());
    }

    #[test]
    fn match_mismatch_and_first_use() {
        let fp = fingerprint(b"cert");
        assert_eq!(decide(true, Some(&fp), b"cert"), Decision::Trust);
        assert_eq!(decide(true, Some(&fp), b"other"), Decision::Mismatch { expected: fp.clone(), got: fingerprint(b"other") });
        assert_eq!(decide(true, None, b"cert"), Decision::PinNow(fp));
        assert_eq!(decide(false, None, b"cert"), Decision::SystemCheck);
    }

    #[test]
    fn the_store_pins_once_refuses_others_and_forgets() {
        let saved = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let log = saved.clone();
        let s = PinStore::new(true, None, move |p| log.lock().unwrap().push(p.map(String::from)));
        assert!(s.accept(b"first"), "first use pins");
        assert_eq!(s.get(), Some(fingerprint(b"first")));
        assert!(s.accept(b"first"));
        assert!(!s.accept(b"second"), "a new certificate is refused");
        s.forget();
        assert_eq!(s.get(), None);
        assert!(s.accept(b"second"), "after forget the next one is pinned");
        assert_eq!(*saved.lock().unwrap(), [Some(fingerprint(b"first")), None, Some(fingerprint(b"second"))]);
        let strict = PinStore::new(false, None, |_| {});
        assert!(!strict.accept(b"first"), "insecureTls off: never");
    }
}
