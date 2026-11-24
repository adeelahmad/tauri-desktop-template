//! Sign-in with OAuth 2 and OpenID Connect: the authorization code flow with PKCE in the
//! system browser, tokens kept in the OS keychain.
//!
//! The flow ([RFC 8252](https://www.rfc-editor.org/rfc/rfc8252), OAuth for native apps):
//!
//! 1. [`Oauth::begin`] makes a PKCE verifier, a random `state` and (for OpenID Connect) a
//!    `nonce`, and returns the provider's sign-in URL with a [`Pending`] login.
//! 2. The URL opens in the user's own browser ([`open_browser`]), never in a web view, so
//!    the app never sees the password and the browser's saved sign-ins and passkeys work.
//! 3. The provider sends the browser back to a [`Loopback`] server on `127.0.0.1` at a
//!    random port, or to the app's deep link (`<app>://oauth/callback`).
//! 4. [`Oauth::complete`] checks `state`, swaps the code for tokens with the PKCE verifier,
//!    verifies the ID token (signature from the provider's JWKS, issuer, audience, expiry and
//!    `nonce`) and saves the tokens in the keychain.
//!
//! [`Oauth::login`] runs all four steps with the loopback redirect. Afterwards
//! [`Oauth::access_token`] refreshes the access token shortly before it expires,
//! [`Oauth::current_user`] returns who signed in (from the ID token, or the userinfo
//! endpoint for GitHub), and [`Oauth::logout`] revokes the token where the provider
//! supports it and forgets it.
//!
//! Providers ([`ProviderKind`]): Google, GitHub (OAuth 2 only, no OpenID Connect),
//! Microsoft (the `common` tenant by default) and any OpenID Connect provider through
//! discovery (`<issuer>/.well-known/openid-configuration`).
//!
//! Secrets never go into settings or logs: tokens and optional client secrets live in the OS
//! keychain ([`Keychain`]: macOS Keychain, Windows Credential Manager, the Secret Service on
//! Linux) behind the [`SecretStore`] trait, which [`MemoryStore`] implements for tests.
//! `Debug` output of [`Tokens`] is redacted. Every request goes through [`crate::net`]
//! (its proxy, allow/deny lists and pins); endpoints must be HTTPS, except on this computer.
//!
//! # Example
//!
//! Start a sign-in and check what comes back, without a network (the first steps of the
//! flow are pure):
//!
//! ```
//! use desktop_core::oauth::{parse_callback, MemoryStore, Oauth, OauthConfig, ProviderConfig, ProviderKind};
//!
//! let config = OauthConfig {
//!     enabled: true,
//!     providers: vec![ProviderConfig { kind: ProviderKind::Google, client_id: "123.apps.googleusercontent.com".into(), ..ProviderConfig::default() }],
//!     ..OauthConfig::default()
//! }
//! .validated();
//! let oauth = Oauth::new(config, MemoryStore::default(), "docs");
//! let (url, pending) = oauth.begin("google", "http://127.0.0.1:53682/callback")?;
//! assert!(url.starts_with("https://accounts.google.com/"));
//! assert!(url.contains("code_challenge_method=S256"));
//!
//! // The browser comes back with a code and the same state:
//! let back = format!("/callback?code=4%2FabC&state={}", pending.state);
//! assert_eq!(parse_callback(&back, &pending.state)?, "4/abC");
//! // A reply for another sign-in is refused:
//! assert!(parse_callback("/callback?code=x&state=forged", &pending.state).is_err());
//! # Ok::<(), desktop_core::oauth::OauthError>(())
//! ```
//!
//! The whole sign-in, with tokens in the OS keychain:
//!
//! ```no_run
//! use desktop_core::oauth::{open_browser, Keychain, Oauth, OauthConfig, ProviderConfig, ProviderKind};
//!
//! let config = OauthConfig {
//!     enabled: true,
//!     providers: vec![ProviderConfig { kind: ProviderKind::Github, client_id: "Iv1.0123456789abcdef".into(), ..ProviderConfig::default() }],
//!     ..OauthConfig::default()
//! };
//! let oauth = Oauth::new(config.validated(), Keychain::new("com.example.myapp"), "default");
//! // GitHub wants the client secret even with PKCE; it goes to the keychain, not settings.
//! oauth.set_client_secret("github", Some("from the GitHub app page"))?;
//! let status = oauth.login("github", &mut |url| open_browser(url))?;
//! println!("signed in as {:?}", status.user);
//! let token = oauth.access_token("github")?; // refreshed when it is about to expire
//! # let _ = token;
//! oauth.logout("github")?;
//! # Ok::<(), desktop_core::oauth::OauthError>(())
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Seconds of clock difference tolerated when checking ID token times.
const LEEWAY: u64 = 120;
/// The path the loopback server answers on.
pub const CALLBACK_PATH: &str = "/callback";
/// Largest reply read from a provider.
const MAX_REPLY: u64 = 1 << 20;

/// What can go wrong while signing in. Messages never contain tokens or secrets.
#[derive(Debug, thiserror::Error)]
pub enum OauthError {
    /// Sign-in is off, or a provider is unknown or incomplete.
    #[error("sign-in: {0}")]
    Config(String),
    /// The reply's `state` is not the one this sign-in sent (a stale or forged reply).
    #[error("the sign-in reply does not belong to this sign-in; start again")]
    StateMismatch,
    /// The reply from the browser was missing its code or could not be read.
    #[error("the sign-in reply was not usable: {0}")]
    Callback(String),
    /// The provider said no (`access_denied`, `invalid_grant`, ...).
    #[error("the provider refused: {error}{}", .description.as_deref().map(|d| format!(" ({d})")).unwrap_or_default())]
    Provider {
        /// The OAuth error code.
        error: String,
        /// The provider's explanation, when it gave one.
        description: Option<String>,
    },
    /// A request failed (network, policy or an unexpected reply).
    #[error("network: {0}")]
    Http(String),
    /// The ID token failed a check (signature, issuer, audience, expiry or nonce).
    #[error("ID token: {0}")]
    IdToken(String),
    /// The OS keychain could not be read or written.
    #[error("keychain: {0}")]
    Keychain(String),
    /// Nobody finished signing in within the time allowed.
    #[error("timed out waiting for the sign-in in the browser")]
    Timeout,
    /// There is no saved sign-in for this provider.
    #[error("not signed in to {0}")]
    NotSignedIn(String),
}

/// Results of this module.
pub type Result<T, E = OauthError> = std::result::Result<T, E>;

// ---------------------------------------------------------------------------------------
// Settings

/// Sign-in settings (`oauth` in settings). Client secrets are not here: they are kept in
/// the keychain ([`Oauth::set_client_secret`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OauthConfig {
    /// Allow signing in at all (off by default).
    pub enabled: bool,
    /// The providers offered, each with its own `id`.
    pub providers: Vec<ProviderConfig>,
    /// Where the browser returns: a loopback server, or the app's deep link.
    pub redirect: RedirectMode,
    /// How long a sign-in may take in the browser.
    pub timeout_secs: u64,
    /// Refresh the access token this many seconds before it expires.
    pub refresh_before_secs: u64,
}

impl Default for OauthConfig {
    fn default() -> Self {
        Self { enabled: false, providers: Vec::new(), redirect: RedirectMode::Loopback, timeout_secs: 300, refresh_before_secs: 120 }
    }
}

impl OauthConfig {
    /// Times clamped, names trimmed, empty ids set from the kind, duplicate ids dropped.
    pub fn validated(mut self) -> Self {
        self.timeout_secs = self.timeout_secs.clamp(30, 1800);
        self.refresh_before_secs = self.refresh_before_secs.clamp(10, 3600);
        let mut seen = std::collections::BTreeSet::new();
        self.providers = std::mem::take(&mut self.providers)
            .into_iter()
            .map(ProviderConfig::validated)
            .filter(|p| seen.insert(p.id.clone()))
            .collect();
        self
    }
}

/// Where the provider sends the browser back to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RedirectMode {
    /// `http://127.0.0.1:<random port>/callback`, served by [`Loopback`] (works everywhere).
    #[default]
    Loopback,
    /// `<app>://oauth/callback` through the Tauri deep-link plugin (when it is built in).
    DeepLink,
}

/// Which provider preset to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Google accounts (OpenID Connect).
    #[default]
    Google,
    /// GitHub (OAuth 2 only: the user comes from the REST API, there is no ID token).
    Github,
    /// Microsoft accounts and Entra ID (OpenID Connect, `common` tenant by default).
    Microsoft,
    /// Any OpenID Connect provider, found through discovery from `issuer`.
    Oidc,
}

impl ProviderKind {
    fn name(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Github => "github",
            Self::Microsoft => "microsoft",
            Self::Oidc => "oidc",
        }
    }
}

/// One provider the user can sign in with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderConfig {
    /// The name used by the commands and the keychain (default: the kind, e.g. `google`).
    pub id: String,
    /// The preset.
    pub kind: ProviderKind,
    /// The OAuth client id (public; it identifies the app to the provider).
    pub client_id: String,
    /// Scopes to ask for (empty: the preset's, e.g. `openid email profile`).
    pub scopes: Vec<String>,
    /// The issuer URL of a generic OpenID Connect provider (kind `oidc`).
    pub issuer: String,
    /// Microsoft tenant: `common`, `organizations`, `consumers` or a tenant id.
    pub tenant: String,
}

impl ProviderConfig {
    /// Trimmed, with the id and the Microsoft tenant filled in.
    pub fn validated(mut self) -> Self {
        self.id = self.id.trim().to_string();
        if self.id.is_empty() {
            self.id = self.kind.name().into();
        }
        self.client_id = self.client_id.trim().to_string();
        self.issuer = self.issuer.trim().trim_end_matches('/').to_string();
        self.tenant = self.tenant.trim().to_string();
        if self.tenant.is_empty() {
            self.tenant = "common".into();
        }
        self.scopes = self.scopes.iter().flat_map(|s| s.split_whitespace()).map(String::from).collect();
        self
    }

    /// The scopes asked for: the configured ones or the preset's.
    pub fn scopes(&self) -> Vec<String> {
        if !self.scopes.is_empty() {
            return self.scopes.clone();
        }
        let s: &[&str] = match self.kind {
            ProviderKind::Google => &["openid", "email", "profile"],
            ProviderKind::Github => &["read:user", "user:email"],
            ProviderKind::Microsoft => &["openid", "email", "profile", "offline_access"],
            ProviderKind::Oidc => &["openid", "email", "profile", "offline_access"],
        };
        s.iter().map(|s| s.to_string()).collect()
    }

    /// Whether ID tokens (and so `nonce`) are part of this sign-in.
    pub fn is_oidc(&self) -> bool {
        self.kind != ProviderKind::Github && self.scopes().iter().any(|s| s == "openid")
    }

    /// The preset endpoints (`None` for a generic OpenID Connect provider: see [`discover`]).
    pub fn preset(&self) -> Option<Endpoints> {
        let s = |v: &str| Some(v.to_string());
        match self.kind {
            ProviderKind::Google => Some(Endpoints {
                authorization: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token: "https://oauth2.googleapis.com/token".into(),
                userinfo: s("https://openidconnect.googleapis.com/v1/userinfo"),
                revocation: s("https://oauth2.googleapis.com/revoke"),
                jwks: s("https://www.googleapis.com/oauth2/v3/certs"),
                issuer: s("https://accounts.google.com"),
            }),
            ProviderKind::Github => Some(Endpoints {
                authorization: "https://github.com/login/oauth/authorize".into(),
                token: "https://github.com/login/oauth/access_token".into(),
                userinfo: s("https://api.github.com/user"),
                revocation: Some(format!("https://api.github.com/applications/{}/token", self.client_id)),
                jwks: None,
                issuer: None,
            }),
            ProviderKind::Microsoft => {
                let base = format!("https://login.microsoftonline.com/{}", self.tenant);
                Some(Endpoints {
                    authorization: format!("{base}/oauth2/v2.0/authorize"),
                    token: format!("{base}/oauth2/v2.0/token"),
                    userinfo: s("https://graph.microsoft.com/oidc/userinfo"),
                    // Microsoft has no RFC 7009 endpoint; signing out forgets the tokens.
                    revocation: None,
                    jwks: Some(format!("{base}/discovery/v2.0/keys")),
                    // Multi-tenant ID tokens name the user's own tenant (`tid`).
                    issuer: s("https://login.microsoftonline.com/{tenantid}/v2.0"),
                })
            }
            ProviderKind::Oidc => None,
        }
    }

    /// The endpoints: the preset, or discovery for a generic OpenID Connect provider.
    pub fn endpoints(&self) -> Result<Endpoints> {
        match self.preset() {
            Some(e) => Ok(e),
            None if self.issuer.is_empty() => Err(OauthError::Config(format!("{}: set the issuer URL", self.id))),
            None => discover(&self.issuer),
        }
    }
}

/// Where a provider's sign-in, token, userinfo, revocation and key endpoints are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoints {
    /// The sign-in page the browser opens.
    pub authorization: String,
    /// Where codes and refresh tokens are swapped for tokens.
    pub token: String,
    /// Who signed in (OpenID Connect userinfo, or GitHub's `/user`).
    pub userinfo: Option<String>,
    /// Token revocation (RFC 7009), when the provider has one.
    pub revocation: Option<String>,
    /// The provider's signing keys (JWKS) for ID tokens.
    pub jwks: Option<String>,
    /// The expected `iss` of ID tokens (`None`: not OpenID Connect).
    pub issuer: Option<String>,
}

/// Read `<issuer>/.well-known/openid-configuration`.
pub fn discover(issuer: &str) -> Result<Endpoints> {
    let issuer = issuer.trim_end_matches('/');
    let doc = get_json(&format!("{issuer}/.well-known/openid-configuration"), None)?;
    parse_discovery(&doc, issuer)
}

/// The endpoints in a discovery document, checked against the issuer it was fetched from.
pub fn parse_discovery(doc: &Value, issuer: &str) -> Result<Endpoints> {
    let field = |k: &str| doc.get(k).and_then(Value::as_str).map(String::from);
    let need = |k: &str| field(k).ok_or_else(|| OauthError::Config(format!("the discovery document has no {k}")));
    let found = need("issuer")?;
    if found.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return Err(OauthError::Config(format!("discovery names issuer {found}, expected {issuer}")));
    }
    Ok(Endpoints {
        authorization: need("authorization_endpoint")?,
        token: need("token_endpoint")?,
        userinfo: field("userinfo_endpoint"),
        revocation: field("revocation_endpoint"),
        jwks: field("jwks_uri"),
        issuer: Some(found),
    })
}

// ---------------------------------------------------------------------------------------
// PKCE, state and the authorization request

/// A PKCE pair ([RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)): the verifier stays in
/// the app, the S256 challenge goes to the provider.
#[derive(Clone)]
pub struct Pkce {
    /// 43 random URL-safe characters (256 bits).
    pub verifier: String,
    /// `BASE64URL(SHA-256(verifier))`.
    pub challenge: String,
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce").field("challenge", &self.challenge).finish_non_exhaustive()
    }
}

impl Pkce {
    /// A fresh random pair.
    pub fn new() -> Result<Self> {
        Ok(Self::from_verifier(&random_token()?))
    }
    /// The pair for a known verifier.
    pub fn from_verifier(verifier: &str) -> Self {
        use sha2::Digest;
        Self { verifier: verifier.into(), challenge: b64url(&sha2::Sha256::digest(verifier.as_bytes())) }
    }
}

/// 32 bytes from the OS's secure random source, base64url (43 characters).
pub fn random_token() -> Result<String> {
    let mut b = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).map_err(|_| OauthError::Config("no secure random numbers".into()))?;
    Ok(b64url(&b))
}

/// A sign-in that was started and waits for the browser to come back.
#[derive(Clone)]
pub struct Pending {
    /// The provider's id.
    pub provider: String,
    /// The random `state` the reply must carry.
    pub state: String,
    /// The redirect URI sent (it is sent again with the code).
    pub redirect_uri: String,
    nonce: Option<String>,
    verifier: String,
    endpoints: Endpoints,
    started: Instant,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending").field("provider", &self.provider).field("redirect_uri", &self.redirect_uri).finish_non_exhaustive()
    }
}

impl Pending {
    /// How long ago the sign-in started.
    pub fn age(&self) -> Duration {
        self.started.elapsed()
    }
}

/// The sign-in URL for `provider` and the [`Pending`] login it starts.
pub fn authorize_url(provider: &ProviderConfig, endpoints: Endpoints, redirect_uri: &str) -> Result<(String, Pending)> {
    if provider.client_id.is_empty() {
        return Err(OauthError::Config(format!("{}: set the client id", provider.id)));
    }
    crate::net::check(&endpoints.authorization).map_err(OauthError::Http)?;
    let pkce = Pkce::new()?;
    let state = random_token()?;
    let nonce = if provider.is_oidc() { Some(random_token()?) } else { None };
    let scope = provider.scopes().join(" ");
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", provider.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("scope", scope.as_str()),
        ("state", state.as_str()),
        ("code_challenge", pkce.challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    if let Some(n) = &nonce {
        params.push(("nonce", n));
    }
    if provider.kind == ProviderKind::Google {
        // Google returns a refresh token only for offline access.
        params.push(("access_type", "offline"));
    }
    let sep = if endpoints.authorization.contains('?') { '&' } else { '?' };
    let url = format!("{}{sep}{}", endpoints.authorization, form(&params));
    let pending =
        Pending { provider: provider.id.clone(), state, redirect_uri: redirect_uri.into(), nonce, verifier: pkce.verifier, endpoints, started: Instant::now() };
    Ok((url, pending))
}

/// The `code` in the provider's reply, after checking that `state` is this sign-in's.
///
/// `callback` is the redirect as the app got it: a full URL (`http://127.0.0.1:8123/callback?...`,
/// `myapp://oauth/callback?...`) or an HTTP request target (`/callback?...`).
pub fn parse_callback(callback: &str, expected_state: &str) -> Result<String> {
    let q = query(callback);
    // State first: an error reply for another sign-in is not this sign-in's error.
    match q.get("state") {
        Some(s) if ct_eq(s.as_bytes(), expected_state.as_bytes()) => {}
        _ => return Err(OauthError::StateMismatch),
    }
    if let Some(error) = q.get("error") {
        return Err(OauthError::Provider { error: error.clone(), description: q.get("error_description").cloned() });
    }
    match q.get("code") {
        Some(c) if !c.is_empty() => Ok(c.clone()),
        _ => Err(OauthError::Callback("no code in the reply".into())),
    }
}

/// The `state` of a reply, to find which pending sign-in it belongs to.
pub fn callback_state(callback: &str) -> Option<String> {
    query(callback).remove("state")
}

// ---------------------------------------------------------------------------------------
// The loopback redirect

/// A one-shot HTTP server on `127.0.0.1` at a random port that waits for the browser to
/// come back from the provider (RFC 8252 section 7.3).
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    /// Listen on `127.0.0.1` at a port the OS picks.
    pub fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| OauthError::Callback(format!("could not listen on 127.0.0.1: {e}")))?;
        let port = listener.local_addr().map_err(|e| OauthError::Callback(e.to_string()))?.port();
        Ok(Self { listener, port })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>/callback`, the redirect URI to send.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}{CALLBACK_PATH}", self.port)
    }

    /// Wait for the reply that carries `state` and return its request target
    /// (`/callback?code=...&state=...`), answering the browser with a short page.
    ///
    /// Other paths get a 404 and replies with another `state` a 400; both keep waiting, so a
    /// stray or forged request cannot end the sign-in. A provider error ends it.
    pub fn wait(&self, state: &str, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        self.listener.set_nonblocking(true).map_err(|e| OauthError::Callback(e.to_string()))?;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(done) = self.serve(stream, state) {
                        return done;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(OauthError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(OauthError::Callback(e.to_string())),
            }
        }
    }

    /// Answer one connection; `Some` ends the wait.
    fn serve(&self, mut stream: TcpStream, state: &str) -> Option<Result<String>> {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let target = match read_request_target(&mut stream) {
            Some(t) => t,
            None => {
                respond(&mut stream, 400, "Bad request.");
                return None;
            }
        };
        if target.split(['?', '#']).next() != Some(CALLBACK_PATH) {
            respond(&mut stream, 404, "Not found.");
            return None;
        }
        match parse_callback(&target, state) {
            Ok(_) => {
                respond(&mut stream, 200, "Signed in. You can close this tab and go back to the app.");
                Some(Ok(target))
            }
            Err(OauthError::StateMismatch) => {
                respond(&mut stream, 400, "This sign-in reply is not for the sign-in in progress.");
                None
            }
            Err(e) => {
                respond(&mut stream, 400, "Sign-in did not finish. Go back to the app for details.");
                Some(Err(e))
            }
        }
    }
}

/// The request target of an HTTP `GET` (at most 16 KB of request head is read).
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let line = String::from_utf8_lossy(&head).lines().next()?.to_string();
    let mut parts = line.split(' ');
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Some(target.to_string()),
        _ => None,
    }
}

fn respond(stream: &mut TcpStream, status: u16, text: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!("<!doctype html><meta charset=\"utf-8\"><title>Sign-in</title><p style=\"font:16px system-ui;margin:3em\">{text}</p>");
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// Open `url` in the user's default browser. Only `https://` URLs (and `http://` to this
/// computer) are opened.
pub fn open_browser(url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost")) {
        return Err(OauthError::Config("only https sign-in pages are opened".into()));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        // Not `cmd /c start`: `&` in the URL would split the command.
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android"), not(target_os = "ios")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(any(target_os = "android", target_os = "ios", not(any(unix, windows))))]
    {
        Err(OauthError::Config("open the sign-in page with the app's opener on this platform".into()))
    }
    #[cfg(not(any(target_os = "android", target_os = "ios", not(any(unix, windows)))))]
    {
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().map(|_| ()).map_err(|e| OauthError::Config(format!("could not open the browser: {e}")))
    }
}

// ---------------------------------------------------------------------------------------
// Tokens

/// Who signed in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct User {
    /// The provider's stable id for the user (`sub`, or GitHub's numeric id).
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// Email address, when the provider shared it.
    pub email: Option<String>,
    /// Avatar URL.
    pub picture: Option<String>,
    /// User name (`preferred_username`, or the GitHub login).
    pub login: Option<String>,
}

/// The tokens of one sign-in, as kept in the keychain. `Debug` hides the tokens.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    /// Sent with API calls (`Authorization: Bearer`).
    pub access_token: String,
    /// Gets a new access token without the browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// When the access token expires (Unix seconds; `None`: it does not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Scopes granted, as the provider reported them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Who signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    /// The endpoints signed in with (refresh and revocation use the same ones).
    pub endpoints: Endpoints,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("user", &self.user)
            .finish()
    }
}

impl Tokens {
    /// Whether the access token should be refreshed now: it expires within `before_secs`
    /// and there is a refresh token to do it with.
    pub fn needs_refresh(&self, now: u64, before_secs: u64) -> bool {
        self.refresh_token.is_some() && self.expires_at.is_some_and(|t| now.saturating_add(before_secs) >= t)
    }
    /// Whether the access token has expired.
    pub fn expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|t| now >= t)
    }
}

/// A token endpoint's reply.
#[derive(Deserialize)]
struct TokenReply {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<Value>,
    scope: Option<String>,
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn token_request(url: &str, form_body: &str) -> Result<TokenReply> {
    let (status, json) = post_form(url, form_body)?;
    let reply: TokenReply = serde_json::from_value(json).map_err(|_| OauthError::Http(format!("HTTP {status}: the token reply is not JSON")))?;
    if let Some(error) = reply.error.clone() {
        return Err(OauthError::Provider { error, description: reply.error_description });
    }
    if !(200..300).contains(&status) || reply.access_token.is_none() {
        return Err(OauthError::Http(format!("HTTP {status} from the token endpoint")));
    }
    Ok(reply)
}

fn expires_at(reply: &TokenReply, now: u64) -> Option<u64> {
    // Some providers send the number as a string.
    let secs = match reply.expires_in.as_ref()? {
        Value::Number(n) => n.as_u64()?,
        Value::String(s) => s.parse().ok()?,
        _ => return None,
    };
    Some(now + secs)
}

/// Swap the code from the browser for tokens (with the PKCE verifier), verify the ID token,
/// and find out who signed in.
pub fn exchange_code(provider: &ProviderConfig, pending: &Pending, code: &str, client_secret: Option<&str>) -> Result<Tokens> {
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", pending.redirect_uri.as_str()),
        ("client_id", provider.client_id.as_str()),
        ("code_verifier", pending.verifier.as_str()),
    ];
    if let Some(s) = client_secret {
        params.push(("client_secret", s));
    }
    let reply = token_request(&pending.endpoints.token, &form(&params))?;
    let now = now();
    let e = &pending.endpoints;
    let user = match (&reply.id_token, &e.issuer) {
        (Some(id_token), Some(issuer)) => {
            let jwks_url = e.jwks.as_deref().ok_or_else(|| OauthError::IdToken("the provider has no signing keys (jwks_uri)".into()))?;
            let jwks = get_json(jwks_url, None)?;
            let expect = Expected { issuer, client_id: &provider.client_id, nonce: pending.nonce.as_deref() };
            Some(verify_id_token(id_token, &expect, &jwks, now)?)
        }
        (None, Some(_)) if pending.nonce.is_some() => return Err(OauthError::IdToken("the provider sent no ID token".into())),
        _ => None,
    };
    let access_token = reply.access_token.clone().unwrap_or_default();
    let user = match user {
        Some(u) => Some(u),
        None => e.userinfo.as_deref().map(|url| fetch_user(provider.kind, url, &access_token)).transpose()?,
    };
    Ok(Tokens { access_token, refresh_token: reply.refresh_token.clone(), expires_at: expires_at(&reply, now), scope: reply.scope.clone(), user, endpoints: e.clone() })
}

/// A new access token from the refresh token. A rotated refresh token replaces the old one;
/// otherwise the old one is kept.
pub fn refresh(provider: &ProviderConfig, tokens: &Tokens, client_secret: Option<&str>) -> Result<Tokens> {
    let rt = tokens.refresh_token.as_deref().ok_or_else(|| OauthError::NotSignedIn(provider.id.clone()))?;
    let mut params = vec![("grant_type", "refresh_token"), ("refresh_token", rt), ("client_id", provider.client_id.as_str())];
    if let Some(s) = client_secret {
        params.push(("client_secret", s));
    }
    let reply = token_request(&tokens.endpoints.token, &form(&params))?;
    Ok(Tokens {
        access_token: reply.access_token.clone().unwrap_or_default(),
        refresh_token: reply.refresh_token.clone().or_else(|| tokens.refresh_token.clone()),
        expires_at: expires_at(&reply, now()),
        scope: reply.scope.clone().or_else(|| tokens.scope.clone()),
        user: tokens.user.clone(),
        endpoints: tokens.endpoints.clone(),
    })
}

/// Tell the provider to revoke the tokens. `Ok(false)`: the provider has no way to (or needs
/// a client secret that is not set), so only the local copy goes.
pub fn revoke(provider: &ProviderConfig, tokens: &Tokens, client_secret: Option<&str>) -> Result<bool> {
    let Some(url) = tokens.endpoints.revocation.as_deref() else { return Ok(false) };
    if provider.kind == ProviderKind::Github {
        // GitHub: DELETE /applications/{client_id}/token, authenticated as the app.
        let Some(secret) = client_secret else { return Ok(false) };
        let agent = crate::net::agent(url, crate::net::Limits::total(15, 30)).map_err(OauthError::Http)?;
        let basic = crate::audio::base64(format!("{}:{secret}", provider.client_id).as_bytes());
        let body = serde_json::json!({ "access_token": tokens.access_token }).to_string();
        let resp = agent
            .delete(url)
            .header("Authorization", &format!("Basic {basic}"))
            .header("Accept", "application/vnd.github+json")
            .force_send_body()
            .send(body.as_bytes())
            .map_err(|e| OauthError::Http(e.to_string()))?;
        let status = resp.status().as_u16();
        return match status {
            204 | 404 => Ok(true),
            _ => Err(OauthError::Http(format!("HTTP {status} revoking the GitHub token"))),
        };
    }
    // RFC 7009: revoking the refresh token ends the whole grant at most providers.
    let (token, hint) = match &tokens.refresh_token {
        Some(r) => (r.as_str(), "refresh_token"),
        None => (tokens.access_token.as_str(), "access_token"),
    };
    let mut params = vec![("token", token), ("token_type_hint", hint), ("client_id", provider.client_id.as_str())];
    if let Some(s) = client_secret {
        params.push(("client_secret", s));
    }
    let (status, _) = post_form(url, &form(&params))?;
    // 400 is what an already revoked or expired token gets; it is gone either way.
    match status {
        200..=299 | 400 => Ok(true),
        _ => Err(OauthError::Http(format!("HTTP {status} from the revocation endpoint"))),
    }
}

/// Who signed in, from the userinfo endpoint (or GitHub's `/user`).
pub fn fetch_user(kind: ProviderKind, url: &str, access_token: &str) -> Result<User> {
    let v = get_json(url, Some(access_token))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    Ok(match kind {
        ProviderKind::Github => User {
            id: v.get("id").map(|i| i.to_string().trim_matches('"').to_string()).unwrap_or_default(),
            name: s("name"),
            email: s("email"),
            picture: s("avatar_url"),
            login: s("login"),
        },
        _ => user_from_claims(&v),
    })
}

fn user_from_claims(c: &Value) -> User {
    let s = |k: &str| c.get(k).and_then(Value::as_str).map(String::from);
    User { id: s("sub").unwrap_or_default(), name: s("name"), email: s("email"), picture: s("picture"), login: s("preferred_username") }
}

// ---------------------------------------------------------------------------------------
// ID tokens

/// What an ID token must say.
pub struct Expected<'a> {
    /// The issuer; `{tenantid}` stands for the token's own `tid` (Microsoft multi-tenant).
    pub issuer: &'a str,
    /// The audience: this app's client id.
    pub client_id: &'a str,
    /// The nonce sent with the sign-in.
    pub nonce: Option<&'a str>,
}

/// Verify an ID token (a signed JWT) against the provider's keys (`jwks`, a JWK set) and
/// return the user it names. Checks the signature (RS256/384/512, PS256/384/512, ES256/384;
/// never `none`), `iss`, `aud` (and `azp`), `exp`, `iat` and `nonce`.
pub fn verify_id_token(token: &str, expected: &Expected, jwks: &Value, now: u64) -> Result<User> {
    let bad = |m: &str| OauthError::IdToken(m.to_string());
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(bad("not a signed JWT"));
    };
    let header: Value = serde_json::from_slice(&b64url_decode(h)?).map_err(|_| bad("bad header"))?;
    let claims: Value = serde_json::from_slice(&b64url_decode(p)?).map_err(|_| bad("bad claims"))?;
    let alg = header.get("alg").and_then(Value::as_str).unwrap_or("none");
    let kid = header.get("kid").and_then(Value::as_str);
    let kty = match alg {
        "RS256" | "RS384" | "RS512" | "PS256" | "PS384" | "PS512" => "RSA",
        "ES256" | "ES384" => "EC",
        other => return Err(OauthError::IdToken(format!("signature algorithm {other} is not accepted"))),
    };
    let keys = jwks.get("keys").and_then(Value::as_array).ok_or_else(|| bad("the provider's key set is empty"))?;
    let usable: Vec<&Value> = keys
        .iter()
        .filter(|k| k.get("kty").and_then(Value::as_str) == Some(kty))
        .filter(|k| k.get("alg").and_then(Value::as_str).is_none_or(|a| a == alg))
        .filter(|k| k.get("use").and_then(Value::as_str).is_none_or(|u| u == "sig"))
        .collect();
    let key = match kid {
        Some(kid) => usable.into_iter().find(|k| k.get("kid").and_then(Value::as_str) == Some(kid)),
        None if usable.len() == 1 => usable.into_iter().next(),
        None => None,
    }
    .ok_or_else(|| bad("no matching signing key"))?;
    verify_signature(alg, key, format!("{h}.{p}").as_bytes(), &b64url_decode(sig)?)?;
    check_claims(&claims, expected, now)
}

fn verify_signature(alg: &str, jwk: &Value, msg: &[u8], sig: &[u8]) -> Result<()> {
    use ring::signature as s;
    let part = |k: &str| -> Result<Vec<u8>> { b64url_decode(jwk.get(k).and_then(Value::as_str).ok_or_else(|| OauthError::IdToken(format!("the key has no {k}")))?) };
    let ok = if kty_is_rsa(alg) {
        let params: &s::RsaParameters = match alg {
            "RS256" => &s::RSA_PKCS1_2048_8192_SHA256,
            "RS384" => &s::RSA_PKCS1_2048_8192_SHA384,
            "RS512" => &s::RSA_PKCS1_2048_8192_SHA512,
            "PS256" => &s::RSA_PSS_2048_8192_SHA256,
            "PS384" => &s::RSA_PSS_2048_8192_SHA384,
            _ => &s::RSA_PSS_2048_8192_SHA512,
        };
        s::RsaPublicKeyComponents { n: part("n")?, e: part("e")? }.verify(params, msg, sig).is_ok()
    } else {
        let (algorithm, crv): (&'static dyn s::VerificationAlgorithm, &str) =
            if alg == "ES256" { (&s::ECDSA_P256_SHA256_FIXED, "P-256") } else { (&s::ECDSA_P384_SHA384_FIXED, "P-384") };
        if jwk.get("crv").and_then(Value::as_str) != Some(crv) {
            return Err(OauthError::IdToken("the key's curve does not match".into()));
        }
        let mut point = vec![4u8];
        point.extend(part("x")?);
        point.extend(part("y")?);
        s::UnparsedPublicKey::new(algorithm, point).verify(msg, sig).is_ok()
    };
    if ok {
        Ok(())
    } else {
        Err(OauthError::IdToken("the signature does not verify".into()))
    }
}

fn kty_is_rsa(alg: &str) -> bool {
    alg.starts_with("RS") || alg.starts_with("PS")
}

fn check_claims(c: &Value, expected: &Expected, now: u64) -> Result<User> {
    let bad = |m: String| Err(OauthError::IdToken(m));
    let s = |k: &str| c.get(k).and_then(Value::as_str);
    let iss = s("iss").unwrap_or_default();
    let want = expected.issuer.replace("{tenantid}", s("tid").unwrap_or("\u{0}"));
    let google = want == "https://accounts.google.com" && iss == "accounts.google.com";
    if iss != want && !google {
        return bad(format!("issued by {iss}, expected {want}"));
    }
    let auds: Vec<&str> = match c.get("aud") {
        Some(Value::String(a)) => vec![a.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    if !auds.contains(&expected.client_id) {
        return bad("issued for another app (aud)".into());
    }
    if auds.len() > 1 && s("azp").is_some_and(|azp| azp != expected.client_id) {
        return bad("issued for another app (azp)".into());
    }
    let exp = c.get("exp").and_then(Value::as_u64).ok_or_else(|| OauthError::IdToken("no expiry".into()))?;
    if now > exp + LEEWAY {
        return bad("expired".into());
    }
    if c.get("iat").and_then(Value::as_u64).is_some_and(|iat| iat > now + LEEWAY) {
        return bad("issued in the future (check the computer's clock)".into());
    }
    if let Some(nonce) = expected.nonce {
        if !s("nonce").is_some_and(|n| ct_eq(n.as_bytes(), nonce.as_bytes())) {
            return bad("the nonce does not match this sign-in".into());
        }
    }
    if s("sub").is_none_or(str::is_empty) {
        return bad("no subject".into());
    }
    Ok(user_from_claims(c))
}

// ---------------------------------------------------------------------------------------
// Secret storage

/// Where secrets live. [`Keychain`] is the OS keychain; [`MemoryStore`] is for tests.
pub trait SecretStore: Send + Sync {
    /// The secret saved under `key`, if any.
    fn get(&self, key: &str) -> Result<Option<String>>;
    /// Save `value` under `key`, replacing what was there.
    fn set(&self, key: &str, value: &str) -> Result<()>;
    /// Remove `key` (no error when it is not there).
    fn delete(&self, key: &str) -> Result<()>;
}

/// The OS keychain through the `keyring` crate: the macOS (and iOS) Keychain, the Windows
/// Credential Manager, or the Secret Service (GNOME Keyring, KWallet) on Linux.
pub struct Keychain {
    service: String,
}

impl Keychain {
    /// Entries under `service` (use the app's bundle identifier).
    pub fn new(service: &str) -> Self {
        Self { service: service.into() }
    }
    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(|e| OauthError::Keychain(e.to_string()))
    }
}

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(OauthError::Keychain(e.to_string())),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?.set_password(value).map_err(|e| OauthError::Keychain(e.to_string()))
    }
    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(OauthError::Keychain(e.to_string())),
        }
    }
}

/// Secrets in memory, gone when it is dropped: for tests and examples.
#[derive(Default)]
pub struct MemoryStore(Mutex<BTreeMap<String, String>>);

impl MemoryStore {
    /// The keys stored, sorted.
    pub fn keys(&self) -> Vec<String> {
        self.0.lock().map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().map_err(|_| OauthError::Keychain("poisoned".into()))?.get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().map_err(|_| OauthError::Keychain("poisoned".into()))?.insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().map_err(|_| OauthError::Keychain("poisoned".into()))?.remove(key);
        Ok(())
    }
}

/// Keychains limit the size of one entry (the Windows Credential Manager to 2560 bytes), and
/// Microsoft tokens are larger, so long values are split into numbered entries.
const CHUNK: usize = 1000;

/// A [`SecretStore`] with long values split across entries, all under one prefix.
struct Vault<S> {
    store: S,
    prefix: String,
}

impl<S: SecretStore> Vault<S> {
    fn key(&self, name: &str) -> String {
        format!("{}/{name}", self.prefix)
    }
    fn put(&self, name: &str, value: &str) -> Result<()> {
        self.remove(name)?;
        let key = self.key(name);
        if value.len() <= CHUNK {
            return self.store.set(&key, &format!("1:{value}"));
        }
        let mut chunks = Vec::new();
        let mut rest = value;
        while !rest.is_empty() {
            let mut cut = rest.len().min(CHUNK);
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            chunks.push(&rest[..cut]);
            rest = &rest[cut..];
        }
        for (i, c) in chunks.iter().enumerate() {
            self.store.set(&format!("{key}#{i}"), c)?;
        }
        self.store.set(&key, &format!("n:{}", chunks.len()))
    }
    fn read(&self, name: &str) -> Result<Option<String>> {
        let key = self.key(name);
        let Some(head) = self.store.get(&key)? else { return Ok(None) };
        if let Some(v) = head.strip_prefix("1:") {
            return Ok(Some(v.to_string()));
        }
        let n: usize = head.strip_prefix("n:").and_then(|n| n.parse().ok()).ok_or_else(|| OauthError::Keychain("unreadable entry".into()))?;
        let mut out = String::new();
        for i in 0..n {
            out.push_str(&self.store.get(&format!("{key}#{i}"))?.ok_or_else(|| OauthError::Keychain("an entry is incomplete".into()))?);
        }
        Ok(Some(out))
    }
    fn remove(&self, name: &str) -> Result<()> {
        let key = self.key(name);
        if let Some(n) = self.store.get(&key)?.and_then(|h| h.strip_prefix("n:").and_then(|n| n.parse::<usize>().ok())) {
            for i in 0..n {
                self.store.delete(&format!("{key}#{i}"))?;
            }
        }
        self.store.delete(&key)
    }
}

/// A short, stable name for a data folder, so each profile keeps its own sign-ins.
pub fn scope_for(data_dir: &Path) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(data_dir.to_string_lossy().as_bytes());
    d[..6].iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------------------
// The facade

/// One provider's sign-in, as the Settings screen shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    /// The provider's id.
    pub id: String,
    /// Its preset.
    pub kind: ProviderKind,
    /// A client id is set.
    pub ready: bool,
    /// A sign-in is saved.
    pub signed_in: bool,
    /// Who signed in.
    pub user: Option<User>,
    /// When the access token expires (Unix seconds).
    pub expires_at: Option<u64>,
    /// A client secret is saved in the keychain.
    pub client_secret: bool,
    /// Why the keychain could not be read, if it could not.
    pub error: Option<String>,
}

/// Sign-in for an app: the settings plus a [`SecretStore`] for tokens and client secrets.
pub struct Oauth<S: SecretStore = Keychain> {
    config: OauthConfig,
    vault: Vault<S>,
}

impl<S: SecretStore> Oauth<S> {
    /// Sign-in with `config`, keeping secrets in `store` under `scope` (one per profile:
    /// see [`scope_for`]).
    pub fn new(config: OauthConfig, store: S, scope: &str) -> Self {
        Self { config, vault: Vault { store, prefix: format!("oauth/{scope}") } }
    }

    /// The settings in use.
    pub fn config(&self) -> &OauthConfig {
        &self.config
    }

    /// The provider with this id.
    pub fn provider(&self, id: &str) -> Result<&ProviderConfig> {
        self.config.providers.iter().find(|p| p.id == id).ok_or_else(|| OauthError::Config(format!("no provider {id:?} in settings")))
    }

    /// Start signing in: the URL to open in the browser, and the [`Pending`] login to finish
    /// with [`complete`](Self::complete) when the browser comes back to `redirect_uri`.
    pub fn begin(&self, id: &str, redirect_uri: &str) -> Result<(String, Pending)> {
        if !self.config.enabled {
            return Err(OauthError::Config("sign-in is turned off in Settings".into()));
        }
        let p = self.provider(id)?;
        if p.client_id.is_empty() {
            return Err(OauthError::Config(format!("{id}: set the client id")));
        }
        authorize_url(p, p.endpoints()?, redirect_uri)
    }

    /// Finish a sign-in with the redirect the browser came back with: check `state`, swap
    /// the code for tokens, verify the ID token and save the tokens in the keychain.
    pub fn complete(&self, pending: &Pending, callback: &str) -> Result<ProviderStatus> {
        if pending.age() > Duration::from_secs(self.config.timeout_secs) {
            return Err(OauthError::Timeout);
        }
        let code = parse_callback(callback, &pending.state)?;
        let p = self.provider(&pending.provider)?;
        let secret = self.client_secret(&p.id)?;
        let tokens = exchange_code(p, pending, &code, secret.as_deref())?;
        self.save(&p.id, &tokens)?;
        log::info!("signed in with {}", p.id);
        Ok(self.status_of(p))
    }

    /// The whole sign-in with a loopback redirect: start, `open` the URL (usually
    /// [`open_browser`]), wait for the browser and finish.
    pub fn login(&self, id: &str, open: &mut dyn FnMut(&str) -> Result<()>) -> Result<ProviderStatus> {
        let lb = Loopback::bind()?;
        let (url, pending) = self.begin(id, &lb.redirect_uri())?;
        open(&url)?;
        let target = lb.wait(&pending.state, Duration::from_secs(self.config.timeout_secs))?;
        self.complete(&pending, &target)
    }

    /// A valid access token, refreshed first when it expires within `refreshBeforeSecs`.
    pub fn access_token(&self, id: &str) -> Result<String> {
        let p = self.provider(id)?;
        let tokens = self.tokens(id)?.ok_or_else(|| OauthError::NotSignedIn(id.into()))?;
        let now = now();
        if tokens.needs_refresh(now, self.config.refresh_before_secs) {
            match self.refresh_now(p, &tokens) {
                Ok(t) => return Ok(t.access_token),
                // A still-valid token is used when the refresh fails for a passing reason.
                Err(e @ OauthError::Provider { .. }) => return Err(e),
                Err(e) if tokens.expired(now) => return Err(e),
                Err(e) => log::warn!("token refresh for {id} failed; using the current token: {e}"),
            }
        } else if tokens.expired(now) {
            return Err(OauthError::NotSignedIn(id.into()));
        }
        Ok(tokens.access_token)
    }

    /// Refresh every token that expires soon (run this every minute or so). Returns the
    /// providers refreshed, and those that failed with why.
    pub fn refresh_due(&self) -> Vec<(String, Result<()>)> {
        let now = now();
        let mut out = Vec::new();
        for p in &self.config.providers {
            if let Ok(Some(t)) = self.tokens(&p.id) {
                if t.needs_refresh(now, self.config.refresh_before_secs) {
                    out.push((p.id.clone(), self.refresh_now(p, &t).map(|_| ())));
                }
            }
        }
        out
    }

    fn refresh_now(&self, p: &ProviderConfig, tokens: &Tokens) -> Result<Tokens> {
        let secret = self.client_secret(&p.id)?;
        match refresh(p, tokens, secret.as_deref()) {
            Ok(t) => {
                self.save(&p.id, &t)?;
                log::debug!("refreshed the {} token", p.id);
                Ok(t)
            }
            Err(e @ OauthError::Provider { .. }) => {
                // invalid_grant and friends: the grant is gone, so is the sign-in.
                log::warn!("the {} sign-in ended: {e}", p.id);
                self.vault.remove(&format!("{}/tokens", p.id))?;
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// Who is signed in with this provider.
    pub fn current_user(&self, id: &str) -> Result<Option<User>> {
        Ok(self.tokens(id)?.and_then(|t| t.user))
    }

    /// Sign out: revoke the tokens where the provider supports it, then forget them. Returns
    /// whether the provider revoked them. A failed revocation still signs out here.
    pub fn logout(&self, id: &str) -> Result<bool> {
        let p = self.provider(id)?;
        let revoked = match self.tokens(id)? {
            Some(t) => {
                let secret = self.client_secret(id)?;
                revoke(p, &t, secret.as_deref()).unwrap_or_else(|e| {
                    log::warn!("could not revoke the {id} token: {e}");
                    false
                })
            }
            None => false,
        };
        self.vault.remove(&format!("{id}/tokens"))?;
        log::info!("signed out of {id}");
        Ok(revoked)
    }

    /// Every provider's sign-in.
    pub fn status(&self) -> Vec<ProviderStatus> {
        self.config.providers.iter().map(|p| self.status_of(p)).collect()
    }

    fn status_of(&self, p: &ProviderConfig) -> ProviderStatus {
        let (tokens, error) = match self.tokens(&p.id) {
            Ok(t) => (t, None),
            Err(e) => (None, Some(e.to_string())),
        };
        ProviderStatus {
            id: p.id.clone(),
            kind: p.kind,
            ready: !p.client_id.is_empty(),
            signed_in: tokens.is_some(),
            expires_at: tokens.as_ref().and_then(|t| t.expires_at),
            user: tokens.and_then(|t| t.user),
            client_secret: self.client_secret(&p.id).ok().flatten().is_some(),
            error,
        }
    }

    /// The saved tokens for a provider.
    pub fn tokens(&self, id: &str) -> Result<Option<Tokens>> {
        match self.vault.read(&format!("{id}/tokens"))? {
            Some(json) => Ok(serde_json::from_str(&json).ok()),
            None => Ok(None),
        }
    }

    fn save(&self, id: &str, tokens: &Tokens) -> Result<()> {
        let json = serde_json::to_string(tokens).map_err(|e| OauthError::Keychain(e.to_string()))?;
        self.vault.put(&format!("{id}/tokens"), &json)
    }

    /// The client secret saved for a provider.
    pub fn client_secret(&self, id: &str) -> Result<Option<String>> {
        self.vault.read(&format!("{id}/client-secret"))
    }

    /// Save a provider's client secret in the keychain (`None` or empty removes it).
    pub fn set_client_secret(&self, id: &str, secret: Option<&str>) -> Result<()> {
        match secret.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => self.vault.put(&format!("{id}/client-secret"), s),
            None => self.vault.remove(&format!("{id}/client-secret")),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Small helpers: time, encoding, HTTP

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn b64url(bytes: &[u8]) -> String {
    crate::audio::base64(bytes).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

fn b64url_decode(s: &str) -> Result<Vec<u8>> {
    crate::audio::base64_decode(s).map_err(|_| OauthError::IdToken("bad base64".into()))
}

/// Percent-encode everything but RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'+', _, _) => out.push(b' '),
            (b'%', Some(h), Some(l)) => {
                out.push(h << 4 | l);
                i += 2;
            }
            (c, _, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

/// The query parameters of a URL or request target (the first of each name).
fn query(url: &str) -> BTreeMap<String, String> {
    let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let q = q.split('#').next().unwrap_or("");
    let mut out = BTreeMap::new();
    for pair in q.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.entry(decode(k)).or_insert_with(|| decode(v));
    }
    out
}

fn post_form(url: &str, body: &str) -> Result<(u16, Value)> {
    let agent = crate::net::agent(url, crate::net::Limits::total(15, 30)).map_err(OauthError::Http)?;
    let mut resp = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body.as_bytes())
        .map_err(|e| OauthError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(MAX_REPLY).read_to_string().map_err(|e| OauthError::Http(e.to_string()))?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// GET JSON. Redirects are not followed, so a bearer token never reaches another host.
fn get_json(url: &str, bearer: Option<&str>) -> Result<Value> {
    let agent = crate::net::agent(url, crate::net::Limits::total(15, 30)).map_err(OauthError::Http)?;
    let mut req = agent.get(url).header("Accept", "application/json");
    let auth;
    if let Some(t) = bearer {
        auth = format!("Bearer {t}");
        req = req.header("Authorization", &auth);
    }
    let mut resp = req.call().map_err(|e| OauthError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(OauthError::Http(format!("HTTP {status} from {}", url.split('?').next().unwrap_or(url))));
    }
    let text = resp.body_mut().with_config().limit(MAX_REPLY).read_to_string().map_err(|e| OauthError::Http(e.to_string()))?;
    serde_json::from_str(&text).map_err(|_| OauthError::Http("the reply is not JSON".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::BufRead;
    use std::sync::Arc;

    type Log = Arc<Mutex<Vec<(String, String, String)>>>;
    type Handler = dyn Fn(&str, &str, &str) -> (u16, String) + Send + Sync;

    /// A tiny HTTP server on 127.0.0.1 that plays the provider: `handler(method, target,
    /// body)` answers each request, and every request is logged.
    fn fake_provider(handler: Arc<Handler>) -> (String, Log) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let log: Log = Arc::default();
        let l = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let body = String::from_utf8(body).unwrap();
                let (status, reply) = handler(&method, &target, &body);
                l.lock().unwrap().push((method, target, body));
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            }
        });
        (base, log)
    }

    /// An ES256 key pair: the signer and its JWK set.
    fn es256() -> (ring::signature::EcdsaKeyPair, Value) {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let p = pair.public_key().as_ref().to_vec();
        let jwks = json!({"keys": [{"kty": "EC", "crv": "P-256", "kid": "k1", "use": "sig", "x": b64url(&p[1..33]), "y": b64url(&p[33..65])}]});
        (pair, jwks)
    }

    fn sign(pair: &ring::signature::EcdsaKeyPair, claims: &Value) -> String {
        let h = b64url(json!({"alg": "ES256", "kid": "k1", "typ": "JWT"}).to_string().as_bytes());
        let p = b64url(claims.to_string().as_bytes());
        let sig = pair.sign(&ring::rand::SystemRandom::new(), format!("{h}.{p}").as_bytes()).unwrap();
        format!("{h}.{p}.{}", b64url(sig.as_ref()))
    }

    fn claims(issuer: &str, nonce: &str) -> Value {
        json!({"iss": issuer, "aud": "client-1", "sub": "user-42", "exp": now() + 3600, "iat": now(), "nonce": nonce, "email": "ann@example.com", "name": "Ann"})
    }

    #[test]
    fn pkce_matches_the_rfc_7636_example() {
        let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert!(!format!("{p:?}").contains("dBjft"), "Debug must not show the verifier");
    }

    #[test]
    fn random_tokens_are_url_safe_and_unique() {
        let (a, b) = (random_token().unwrap(), random_token().unwrap());
        assert_eq!(a.len(), 43);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    }

    #[test]
    fn authorize_url_carries_pkce_state_and_nonce() {
        let google = ProviderConfig { kind: ProviderKind::Google, client_id: "client-1".into(), ..Default::default() }.validated();
        let (url, pending) = authorize_url(&google, google.preset().unwrap(), "http://127.0.0.1:9/callback").unwrap();
        let q = query(&url);
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "client-1");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:9/callback");
        assert_eq!(q["scope"], "openid email profile");
        assert_eq!(q["state"], pending.state);
        assert_eq!(q["code_challenge"], Pkce::from_verifier(&pending.verifier).challenge);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q.get("nonce"), pending.nonce.as_ref());
        assert!(pending.nonce.is_some());
        // GitHub is OAuth 2 only: no nonce.
        let gh = ProviderConfig { kind: ProviderKind::Github, client_id: "c".into(), ..Default::default() }.validated();
        let (url, pending) = authorize_url(&gh, gh.preset().unwrap(), "http://127.0.0.1:9/callback").unwrap();
        assert!(!url.contains("nonce=") && pending.nonce.is_none());
        assert!(url.contains("scope=read%3Auser%20user%3Aemail"));
    }

    #[test]
    fn callbacks_are_parsed_and_state_is_checked() {
        assert_eq!(parse_callback("/callback?code=a%2Fb+c&state=s1", "s1").unwrap(), "a/b c");
        assert_eq!(parse_callback("http://127.0.0.1:5000/callback?state=s1&code=x#frag", "s1").unwrap(), "x");
        assert_eq!(parse_callback("myapp://oauth/callback?code=y&state=s1", "s1").unwrap(), "y");
        assert!(matches!(parse_callback("/callback?code=x&state=s2", "s1"), Err(OauthError::StateMismatch)));
        assert!(matches!(parse_callback("/callback?code=x", "s1"), Err(OauthError::StateMismatch)));
        assert!(matches!(parse_callback("/callback?state=s1", "s1"), Err(OauthError::Callback(_))));
        match parse_callback("/callback?error=access_denied&error_description=User+said+no&state=s1", "s1") {
            Err(OauthError::Provider { error, description }) => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("User said no"));
            }
            other => panic!("{other:?}"),
        }
        // An error for another sign-in is a state mismatch, not this sign-in's error.
        assert!(matches!(parse_callback("/callback?error=access_denied&state=other", "s1"), Err(OauthError::StateMismatch)));
        assert_eq!(callback_state("/callback?code=x&state=abc").as_deref(), Some("abc"));
    }

    fn browser_get(port: u16, target: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn loopback_ignores_stray_requests_and_returns_the_callback() {
        let lb = Loopback::bind().unwrap();
        assert!(lb.redirect_uri().starts_with("http://127.0.0.1:") && lb.redirect_uri().ends_with("/callback"));
        let port = lb.port();
        let browser = std::thread::spawn(move || {
            let a = browser_get(port, "/favicon.ico");
            let b = browser_get(port, "/callback?code=x&state=forged");
            let c = browser_get(port, "/callback?code=good&state=s1");
            (a, b, c)
        });
        let target = lb.wait("s1", Duration::from_secs(10)).unwrap();
        assert_eq!(target, "/callback?code=good&state=s1");
        let (a, b, c) = browser.join().unwrap();
        assert!(a.starts_with("HTTP/1.1 404"));
        assert!(b.starts_with("HTTP/1.1 400"));
        assert!(c.starts_with("HTTP/1.1 200") && c.contains("Signed in"));
    }

    #[test]
    fn loopback_times_out_and_reports_provider_errors() {
        let lb = Loopback::bind().unwrap();
        assert!(matches!(lb.wait("s", Duration::from_millis(100)), Err(OauthError::Timeout)));
        let port = lb.port();
        let t = std::thread::spawn(move || browser_get(port, "/callback?error=access_denied&state=s"));
        assert!(matches!(lb.wait("s", Duration::from_secs(10)), Err(OauthError::Provider { .. })));
        assert!(t.join().unwrap().starts_with("HTTP/1.1 400"));
    }

    #[test]
    fn id_tokens_are_verified() {
        let (pair, jwks) = es256();
        let iss = "https://id.example.com";
        let expect = Expected { issuer: iss, client_id: "client-1", nonce: Some("n1") };
        let user = verify_id_token(&sign(&pair, &claims(iss, "n1")), &expect, &jwks, now()).unwrap();
        assert_eq!((user.id.as_str(), user.email.as_deref()), ("user-42", Some("ann@example.com")));
        let fails = |c: Value| verify_id_token(&sign(&pair, &c), &expect, &jwks, now()).unwrap_err().to_string();
        assert!(fails(claims(iss, "other")).contains("nonce"));
        assert!(fails(claims("https://evil.example", "n1")).contains("issued by"));
        let mut c = claims(iss, "n1");
        c["aud"] = json!("someone-else");
        assert!(fails(c).contains("aud"));
        let mut c = claims(iss, "n1");
        c["exp"] = json!(now() - 3600);
        assert!(fails(c).contains("expired"));
        // Another key's signature, a tampered payload, and `alg: none` are refused.
        let (other, _) = es256();
        assert!(verify_id_token(&sign(&other, &claims(iss, "n1")), &expect, &jwks, now()).is_err());
        let good = sign(&pair, &claims(iss, "n1"));
        let mut parts: Vec<&str> = good.split('.').collect();
        let forged = b64url(claims(iss, "n1").to_string().replace("user-42", "admin").as_bytes());
        parts[1] = &forged;
        assert!(verify_id_token(&parts.join("."), &expect, &jwks, now()).unwrap_err().to_string().contains("signature"));
        let none = format!("{}.{}.", b64url(br#"{"alg":"none"}"#), b64url(claims(iss, "n1").to_string().as_bytes()));
        assert!(verify_id_token(&none, &expect, &jwks, now()).unwrap_err().to_string().contains("none"));
    }

    #[test]
    fn microsoft_multi_tenant_issuer_uses_the_tokens_tenant() {
        let (pair, jwks) = es256();
        let ms = ProviderConfig { kind: ProviderKind::Microsoft, client_id: "client-1".into(), ..Default::default() }.validated();
        let e = ms.preset().unwrap();
        assert!(e.authorization.starts_with("https://login.microsoftonline.com/common/"));
        let expect = Expected { issuer: e.issuer.as_deref().unwrap(), client_id: "client-1", nonce: Some("n1") };
        let mut c = claims("https://login.microsoftonline.com/t-123/v2.0", "n1");
        c["tid"] = json!("t-123");
        assert!(verify_id_token(&sign(&pair, &c), &expect, &jwks, now()).is_ok());
        c["tid"] = json!("t-999");
        assert!(verify_id_token(&sign(&pair, &c), &expect, &jwks, now()).is_err());
    }

    /// A fake OpenID Connect provider: discovery, JWKS, token (code and refresh grants) and
    /// revocation. The test signs the ID token once it knows the nonce (`id_token`).
    struct Fake {
        base: String,
        log: Log,
        pair: ring::signature::EcdsaKeyPair,
        id_token: Arc<Mutex<String>>,
        refreshes: Arc<Mutex<u32>>,
    }

    fn fake_oidc() -> Fake {
        let (pair, jwks) = es256();
        let id_token: Arc<Mutex<String>> = Arc::default();
        let refreshes: Arc<Mutex<u32>> = Arc::default();
        let base: Arc<Mutex<String>> = Arc::default();
        let (b, t, r) = (base.clone(), id_token.clone(), refreshes.clone());
        let handler: Arc<Handler> = Arc::new(move |method, target, body| {
            let base = b.lock().unwrap().clone();
            let f = query(&format!("?{body}"));
            match (method, target.split('?').next().unwrap_or("")) {
                ("GET", "/.well-known/openid-configuration") => (
                    200,
                    json!({"issuer": base, "authorization_endpoint": format!("{base}/authorize"), "token_endpoint": format!("{base}/token"),
                           "revocation_endpoint": format!("{base}/revoke"), "jwks_uri": format!("{base}/jwks")})
                    .to_string(),
                ),
                ("GET", "/jwks") => (200, jwks.to_string()),
                ("POST", "/token") if f["grant_type"] == "authorization_code" => match f["code"].as_str() {
                    "the-code" => (200, json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600, "token_type": "Bearer", "id_token": *t.lock().unwrap()}).to_string()),
                    _ => (400, json!({"error": "invalid_grant"}).to_string()),
                },
                ("POST", "/token") => {
                    *r.lock().unwrap() += 1;
                    match f["refresh_token"].as_str() {
                        // Some providers send expires_in as a string.
                        "rt-1" => (200, json!({"access_token": "at-2", "expires_in": "3600", "token_type": "Bearer"}).to_string()),
                        _ => (400, json!({"error": "invalid_grant", "error_description": "revoked"}).to_string()),
                    }
                }
                ("POST", "/revoke") => (200, String::new()),
                _ => (404, "{}".into()),
            }
        });
        let (url, log) = fake_provider(handler);
        *base.lock().unwrap() = url.clone();
        Fake { base: url, log, pair, id_token, refreshes }
    }

    fn oidc(base: &str) -> Oauth<MemoryStore> {
        let config = OauthConfig {
            enabled: true,
            providers: vec![ProviderConfig { id: "corp".into(), kind: ProviderKind::Oidc, client_id: "client-1".into(), issuer: base.into(), ..Default::default() }],
            ..OauthConfig::default()
        }
        .validated();
        Oauth::new(config, MemoryStore::default(), "test")
    }

    /// Sign in through the loopback server as a browser would. The ID token carries the
    /// sign-in's nonce, or `nonce` when given (a replayed token).
    fn sign_in(oauth: &Oauth<MemoryStore>, fake: &Fake, nonce: Option<&str>) -> Result<ProviderStatus> {
        oauth.login("corp", &mut |url| {
            let q = query(url);
            assert!(url.starts_with(&format!("{}/authorize?", fake.base)));
            let n = nonce.unwrap_or(&q["nonce"]);
            *fake.id_token.lock().unwrap() = sign(&fake.pair, &json!({"iss": fake.base, "aud": "client-1", "sub": "user-42", "exp": now() + 3600, "nonce": n, "name": "Ann"}));
            let port: u16 = q["redirect_uri"].trim_start_matches("http://127.0.0.1:").trim_end_matches(CALLBACK_PATH).parse().unwrap();
            let state = q["state"].clone();
            std::thread::spawn(move || browser_get(port, &format!("/callback?code=the-code&state={state}")));
            Ok(())
        })
    }

    #[test]
    fn full_sign_in_against_a_fake_token_endpoint() {
        let fake = fake_oidc();
        let oauth = oidc(&fake.base);
        let st = sign_in(&oauth, &fake, None).unwrap();
        let log = fake.log.clone();
        assert!(st.signed_in);
        assert_eq!(st.user.as_ref().unwrap().id, "user-42");
        assert_eq!(oauth.current_user("corp").unwrap().unwrap().name.as_deref(), Some("Ann"));
        // The code went with the PKCE verifier whose challenge was in the sign-in URL.
        let requests = log.lock().unwrap().clone();
        let token_req = requests.iter().find(|(m, t, _)| m == "POST" && t == "/token").unwrap();
        let f = query(&format!("?{}", token_req.2));
        assert_eq!(f["code"], "the-code");
        assert_eq!(f["code_verifier"].len(), 43);
        assert!(f["redirect_uri"].starts_with("http://127.0.0.1:"));
        assert!(!f.contains_key("client_secret"));
        assert_eq!(oauth.access_token("corp").unwrap(), "at-1");
        // Signing out revokes the refresh token and forgets everything.
        assert!(oauth.logout("corp").unwrap());
        let revoke = log.lock().unwrap().iter().find(|(_, t, _)| t == "/revoke").cloned().unwrap();
        assert!(revoke.2.contains("token=rt-1") && revoke.2.contains("token_type_hint=refresh_token"));
        assert!(oauth.tokens("corp").unwrap().is_none());
        assert!(matches!(oauth.access_token("corp"), Err(OauthError::NotSignedIn(_))));
    }

    #[test]
    fn a_wrong_nonce_fails_the_sign_in() {
        let fake = fake_oidc();
        let oauth = oidc(&fake.base);
        let err = sign_in(&oauth, &fake, Some("replayed")).unwrap_err();
        assert!(err.to_string().contains("nonce"), "{err}");
        assert!(oauth.tokens("corp").unwrap().is_none());
    }

    #[test]
    fn tokens_refresh_before_they_expire() {
        let t = |exp: Option<u64>, rt: bool| Tokens {
            access_token: "a".into(),
            refresh_token: rt.then(|| "r".into()),
            expires_at: exp,
            scope: None,
            user: None,
            endpoints: Endpoints { authorization: String::new(), token: String::new(), userinfo: None, revocation: None, jwks: None, issuer: None },
        };
        assert!(t(Some(1100), true).needs_refresh(1000, 120));
        assert!(!t(Some(1200), true).needs_refresh(1000, 120));
        assert!(!t(Some(1100), false).needs_refresh(1000, 120), "no refresh token, nothing to do");
        assert!(!t(None, true).needs_refresh(1000, 120), "tokens without expiry are not refreshed");
        assert!(t(Some(1000), false).expired(1000));

        let fake = fake_oidc();
        let oauth = oidc(&fake.base);
        sign_in(&oauth, &fake, None).unwrap();
        // Not due yet: no refresh.
        assert_eq!(oauth.access_token("corp").unwrap(), "at-1");
        assert!(oauth.refresh_due().is_empty());
        // About to expire: refreshed, the refresh token kept (none came back), the user kept.
        let mut tokens = oauth.tokens("corp").unwrap().unwrap();
        tokens.expires_at = Some(now() + 30);
        oauth.save("corp", &tokens).unwrap();
        assert_eq!(oauth.access_token("corp").unwrap(), "at-2");
        let after = oauth.tokens("corp").unwrap().unwrap();
        assert_eq!(after.refresh_token.as_deref(), Some("rt-1"));
        assert!(after.expires_at.unwrap() > now() + 3000);
        assert_eq!(after.user.unwrap().id, "user-42");
        assert_eq!(*fake.refreshes.lock().unwrap(), 1);
        // A refresh token the provider no longer accepts ends the sign-in.
        let mut tokens = oauth.tokens("corp").unwrap().unwrap();
        tokens.expires_at = Some(now() + 30);
        tokens.refresh_token = Some("rt-revoked".into());
        oauth.save("corp", &tokens).unwrap();
        let due = oauth.refresh_due();
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0].1, Err(OauthError::Provider { .. })));
        assert!(oauth.tokens("corp").unwrap().is_none());
    }

    #[test]
    fn long_secrets_are_split_across_keychain_entries() {
        let oauth = Oauth::new(OauthConfig::default(), MemoryStore::default(), "p1");
        let long: String = "ü".repeat(1500) + &"x".repeat(1001);
        oauth.vault.put("big", &long).unwrap();
        assert_eq!(oauth.vault.read("big").unwrap().unwrap(), long);
        assert!(oauth.vault.store.keys().len() > 3);
        oauth.vault.put("big", "short").unwrap();
        assert_eq!(oauth.vault.store.keys(), ["oauth/p1/big"], "old chunks are removed");
        oauth.vault.remove("big").unwrap();
        assert!(oauth.vault.store.keys().is_empty());
        oauth.set_client_secret("google", Some(" s3cret ")).unwrap();
        assert_eq!(oauth.client_secret("google").unwrap().as_deref(), Some("s3cret"));
        oauth.set_client_secret("google", None).unwrap();
        assert!(oauth.client_secret("google").unwrap().is_none());
    }

    #[test]
    fn settings_are_validated_and_secrets_stay_out_of_debug() {
        let c: OauthConfig = serde_json::from_value(json!({"enabled": true, "timeoutSecs": 1, "providers": [
            {"kind": "microsoft", "clientId": " abc ", "scopes": ["openid profile"]},
            {"kind": "microsoft", "clientId": "dup"},
            {"id": "work", "kind": "oidc", "issuer": "https://id.example.com/"}
        ]}))
        .unwrap();
        let c = c.validated();
        assert_eq!(c.timeout_secs, 30);
        assert_eq!(c.redirect, RedirectMode::Loopback);
        assert_eq!(c.providers.len(), 2);
        assert_eq!((c.providers[0].id.as_str(), c.providers[0].client_id.as_str(), c.providers[0].tenant.as_str()), ("microsoft", "abc", "common"));
        assert_eq!(c.providers[0].scopes, ["openid", "profile"]);
        assert_eq!(c.providers[1].issuer, "https://id.example.com");
        assert!(!OauthConfig::default().enabled);
        let oauth = Oauth::new(OauthConfig::default(), MemoryStore::default(), "x");
        assert!(matches!(oauth.begin("google", "http://127.0.0.1:1/callback"), Err(OauthError::Config(_))));
        let t = Tokens {
            access_token: "SECRET-AT".into(),
            refresh_token: Some("SECRET-RT".into()),
            expires_at: None,
            scope: None,
            user: None,
            endpoints: ProviderConfig::default().preset().unwrap(),
        };
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("SECRET"), "{dbg}");
    }

    #[test]
    fn discovery_must_name_the_issuer_it_came_from() {
        let doc = json!({"issuer": "https://id.example.com", "authorization_endpoint": "https://id.example.com/a", "token_endpoint": "https://id.example.com/t", "jwks_uri": "https://id.example.com/k"});
        let e = parse_discovery(&doc, "https://id.example.com/").unwrap();
        assert_eq!(e.token, "https://id.example.com/t");
        assert!(e.revocation.is_none());
        assert!(parse_discovery(&doc, "https://other.example.com").is_err());
        assert_eq!(scope_for(Path::new("/a")).len(), 12);
        assert_ne!(scope_for(Path::new("/a")), scope_for(Path::new("/b")));
    }
}
