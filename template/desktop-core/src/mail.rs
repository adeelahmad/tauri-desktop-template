//! Email over SMTP: plain text and HTML bodies, attachments, SSL/TLS (port 465) and
//! STARTTLS (port 587), `AUTH PLAIN` / `AUTH LOGIN`. Written on the standard library and
//! the rustls that every build already carries, so it adds no client crate.
//!
//! Connections follow the network policy ([`crate::net`]): offline mode and the host allow
//! and deny lists apply, and the certificates come from its pins or CA files when set,
//! otherwise from the bundled Mozilla roots. HTTP proxies do not carry SMTP, so mail goes
//! direct. Plain, unencrypted SMTP is only allowed to this computer (a local relay).
//!
//! ```no_run
//! use desktop_core::mail::{send, Attachment, MailConfig, Message, Security};
//!
//! let config = MailConfig {
//!     enabled: true,
//!     host: "smtp.example.com".into(),
//!     port: 587,
//!     security: Security::StartTls,
//!     username: "me@example.com".into(),
//!     from: "Me <me@example.com>".into(),
//!     ..MailConfig::default()
//! };
//! let message = Message {
//!     to: vec!["you@example.com".into()],
//!     subject: "The report".into(),
//!     text: "It is attached.".into(),
//!     attachments: vec![Attachment::from_path("report.pdf".as_ref())?],
//!     ..Message::default()
//! };
//! let sent = send(&config, Some("app password"), &message)?;
//! println!("sent {} to {} recipients", sent.message_id, sent.recipients);
//! # Ok::<(), desktop_core::mail::MailError>(())
//! ```

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// How the connection to the server is protected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the first byte (SMTPS, usually port 465).
    Tls,
    /// Plain connection upgraded with STARTTLS before anything else is sent (usually 587).
    /// Refused if the server does not offer it.
    #[default]
    StartTls,
    /// No encryption: only to this computer (a local relay or a test server).
    None,
}

/// SMTP settings (`mail` in settings). The password is not here: it is kept apart, in an
/// owner-only file (see [`PasswordFile`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MailConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// Login name (empty: send without logging in).
    pub username: String,
    /// The sender, `Name <address>` or a bare address.
    pub from: String,
    /// Connect, read and write timeout.
    pub timeout_secs: u64,
    /// Largest message, attachments included, after encoding.
    pub max_mb: u64,
}

impl Default for MailConfig {
    fn default() -> Self {
        Self { enabled: false, host: String::new(), port: 587, security: Security::StartTls, username: String::new(), from: String::new(), timeout_secs: 30, max_mb: 25 }
    }
}

impl MailConfig {
    pub fn validated(mut self) -> Self {
        self.host = self.host.trim().to_string();
        self.username = self.username.trim().to_string();
        self.from = self.from.trim().to_string();
        if self.port == 0 {
            self.port = if self.security == Security::Tls { 465 } else { 587 };
        }
        self.timeout_secs = self.timeout_secs.clamp(5, 300);
        self.max_mb = self.max_mb.clamp(1, 100);
        self
    }
}

/// One file to attach.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Attachment {
    /// The file name the recipient sees.
    pub name: String,
    /// MIME type; guessed from the name when empty.
    pub content_type: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

impl Attachment {
    /// Read a file to attach, named after it.
    pub fn from_path(path: &Path) -> Result<Self, MailError> {
        let data = std::fs::read(path).map_err(|e| MailError::Message(format!("{}: {e}", path.display())))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into());
        Ok(Self { name, content_type: String::new(), data })
    }
}

/// An email to send.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Message {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// Receive the message without appearing in its headers.
    pub bcc: Vec<String>,
    pub reply_to: Option<String>,
    pub subject: String,
    /// The plain-text body (always sent, for clients that do not show HTML).
    pub text: String,
    /// An HTML body shown instead of `text` where supported.
    pub html: Option<String>,
    #[serde(skip)]
    pub attachments: Vec<Attachment>,
}

/// What a successful send returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pub message_id: String,
    pub recipients: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    /// Settings or the message are not usable (nothing was sent).
    #[error("{0}")]
    Message(String),
    /// The network policy, DNS, TCP or TLS stopped it.
    #[error("connection: {0}")]
    Connect(String),
    /// The server answered with an error.
    #[error("the mail server said {code}: {text}")]
    Server { code: u16, text: String },
}

fn io(e: std::io::Error) -> MailError {
    MailError::Connect(e.to_string())
}

/// The SMTP password, owner-only in `<data>/mail-password`.
pub struct PasswordFile(std::path::PathBuf);

impl PasswordFile {
    pub fn new(data_dir: &Path) -> Self {
        Self(data_dir.join("mail-password"))
    }
    pub fn get(&self) -> Option<String> {
        std::fs::read_to_string(&self.0).ok().filter(|p| !p.is_empty())
    }
    /// Save it, or remove it when empty.
    pub fn set(&self, password: &str) -> std::io::Result<()> {
        if password.is_empty() {
            match std::fs::remove_file(&self.0) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            crate::jobs::write_private(&self.0, password.as_bytes())
        }
    }
}

/// The bare address in `Name <address>` or `address`, checked for the basics and for
/// line breaks (which would let one header become several).
pub fn address(text: &str) -> Result<String, MailError> {
    let t = text.trim();
    let addr = match (t.rfind('<'), t.ends_with('>')) {
        (Some(i), true) => &t[i + 1..t.len() - 1],
        _ => t,
    };
    let ok = !addr.is_empty()
        && addr.len() <= 254
        && !addr.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '"'))
        && addr.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'));
    if ok && !t.contains(['\r', '\n']) {
        Ok(addr.to_string())
    } else {
        Err(MailError::Message(format!("not an email address: {t:?}")))
    }
}

/// A header value: RFC 2047 encoded when it is not plain ASCII.
fn header_text(s: &str) -> Result<String, MailError> {
    if s.contains(['\r', '\n']) {
        return Err(MailError::Message("a header cannot contain line breaks".into()));
    }
    Ok(if s.is_ascii() { s.to_string() } else { format!("=?UTF-8?B?{}?=", crate::audio::base64(s.as_bytes())) })
}

/// `Name <address>` with the name encoded when needed.
fn mailbox(text: &str) -> Result<String, MailError> {
    let addr = address(text)?;
    let t = text.trim();
    match t.rfind('<') {
        Some(i) if t.ends_with('>') && !t[..i].trim().is_empty() => {
            let name = t[..i].trim().trim_matches('"');
            let name = if name.is_ascii() { format!("\"{}\"", name.replace(['"', '\\'], "")) } else { header_text(name)? };
            Ok(format!("{name} <{addr}>"))
        }
        _ => Ok(addr),
    }
}

/// `Tue, 24 Nov 2026 11:05:39 +0000`.
fn rfc5322_date(t: std::time::SystemTime) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let iso = crate::logging::rfc3339(t); // 2026-11-24T11:05:39.123Z
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let month: usize = iso[5..7].parse().unwrap_or(1);
    format!("{}, {} {} {} {} +0000", DAYS[(secs / 86_400 % 7) as usize], &iso[8..10], MONTHS[month - 1], &iso[..4], &iso[11..19])
}

fn guess_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Base64 in lines of 76, as MIME wants.
fn base64_lines(data: &[u8]) -> String {
    let b = crate::audio::base64(data);
    let mut out = String::with_capacity(b.len() + b.len() / 76 * 2 + 2);
    for line in b.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push_str("\r\n");
    }
    out
}

fn random_token() -> String {
    let mut b = [0u8; 12];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The message as it goes over the wire (RFC 5322 + MIME, CRLF line ends), and its id.
pub fn render(from: &str, msg: &Message) -> Result<(String, String), MailError> {
    if msg.to.is_empty() && msg.cc.is_empty() && msg.bcc.is_empty() {
        return Err(MailError::Message("the message has no recipients".into()));
    }
    let domain = address(from)?.rsplit_once('@').map(|(_, d)| d.to_string()).unwrap_or_default();
    let id = format!("<{}@{domain}>", random_token());
    let list = |v: &[String]| v.iter().map(|a| mailbox(a)).collect::<Result<Vec<_>, _>>().map(|v| v.join(", "));
    let mut h = format!("From: {}\r\n", mailbox(from)?);
    if !msg.to.is_empty() {
        h += &format!("To: {}\r\n", list(&msg.to)?);
    }
    if !msg.cc.is_empty() {
        h += &format!("Cc: {}\r\n", list(&msg.cc)?);
    }
    if let Some(r) = &msg.reply_to {
        h += &format!("Reply-To: {}\r\n", mailbox(r)?);
    }
    h += &format!("Subject: {}\r\nDate: {}\r\nMessage-ID: {id}\r\nMIME-Version: 1.0\r\n", header_text(&msg.subject)?, rfc5322_date(std::time::SystemTime::now()));

    let text_part = format!("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}", base64_lines(msg.text.as_bytes()));
    let body = match &msg.html {
        Some(html) => {
            let b = format!("alt-{}", random_token());
            let html_part = format!("Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}", base64_lines(html.as_bytes()));
            format!("Content-Type: multipart/alternative; boundary=\"{b}\"\r\n\r\n--{b}\r\n{text_part}--{b}\r\n{html_part}--{b}--\r\n")
        }
        None => text_part,
    };
    let full = if msg.attachments.is_empty() {
        format!("{h}{body}")
    } else {
        let b = format!("mix-{}", random_token());
        let mut out = format!("{h}Content-Type: multipart/mixed; boundary=\"{b}\"\r\n\r\n--{b}\r\n{body}");
        for a in &msg.attachments {
            if a.name.contains(['\r', '\n', '"']) {
                return Err(MailError::Message(format!("bad attachment name {:?}", a.name)));
            }
            let ctype = if a.content_type.is_empty() { guess_type(&a.name) } else { a.content_type.as_str() };
            if ctype.contains(['\r', '\n', ';', '"']) {
                return Err(MailError::Message(format!("bad content type {ctype:?}")));
            }
            // RFC 2231 for names that are not plain ASCII.
            let disposition = if a.name.is_ascii() {
                format!("attachment; filename=\"{}\"", a.name)
            } else {
                let pct: String = a.name.bytes().map(|c| if c.is_ascii_alphanumeric() || b".-_".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect();
                format!("attachment; filename*=UTF-8''{pct}")
            };
            out += &format!("--{b}\r\nContent-Type: {ctype}\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: {disposition}\r\n\r\n{}", base64_lines(&a.data));
        }
        out + &format!("--{b}--\r\n")
    };
    Ok((full, id))
}

/// The connection, plain or TLS.
enum Wire {
    Plain(BufReader<TcpStream>),
    Tls(Box<BufReader<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>>),
}

impl Wire {
    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Wire::Plain(r) => r.get_mut().write_all(data),
            Wire::Tls(r) => {
                let s = r.get_mut();
                s.write_all(data)?;
                s.flush()
            }
        }
    }
    fn read_line(&mut self, line: &mut String) -> std::io::Result<usize> {
        match self {
            Wire::Plain(r) => r.read_line(line),
            Wire::Tls(r) => r.read_line(line),
        }
    }
}

struct Session {
    wire: Wire,
}

impl Session {
    /// Read one reply (all its lines) and check its code is `want` (first digit match).
    fn reply(&mut self, want: u16) -> Result<(u16, Vec<String>), MailError> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            if self.wire.read_line(&mut line).map_err(io)? == 0 {
                return Err(MailError::Connect("the server closed the connection".into()));
            }
            if line.len() > 4096 {
                return Err(MailError::Connect("reply line too long".into()));
            }
            let line = line.trim_end().to_string();
            let code: u16 = line.get(..3).and_then(|c| c.parse().ok()).ok_or_else(|| MailError::Connect(format!("not an SMTP reply: {line:?}")))?;
            let last = line.as_bytes().get(3) != Some(&b'-');
            lines.push(line.get(4..).unwrap_or_default().to_string());
            if last {
                if code / 100 != want / 100 {
                    return Err(MailError::Server { code, text: lines.join(" ") });
                }
                return Ok((code, lines));
            }
        }
    }

    fn command(&mut self, cmd: &str, want: u16) -> Result<Vec<String>, MailError> {
        self.wire.write_all(format!("{cmd}\r\n").as_bytes()).map_err(io)?;
        self.reply(want).map(|(_, l)| l)
    }

    fn ehlo(&mut self) -> Result<Vec<String>, MailError> {
        let name = local_name();
        match self.command(&format!("EHLO {name}"), 250) {
            Ok(l) => Ok(l),
            Err(MailError::Server { .. }) => self.command(&format!("HELO {name}"), 250),
            Err(e) => Err(e),
        }
    }

    fn start_tls(self, host: &str) -> Result<Self, MailError> {
        let Wire::Plain(reader) = self.wire else { return Err(MailError::Connect("already encrypted".into())) };
        if !reader.buffer().is_empty() {
            // Bytes sent before the TLS handshake would be read as if they came over it.
            return Err(MailError::Connect("the server sent data before TLS started".into()));
        }
        Ok(Self { wire: Wire::Tls(Box::new(BufReader::new(tls(reader.into_inner(), host)?))) })
    }
}

/// The client name sent in EHLO (no personal data: a fixed name, as many clients do).
fn local_name() -> &'static str {
    "localhost"
}

fn tls(tcp: TcpStream, host: &str) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, MailError> {
    let mut roots = rustls::RootCertStore::empty();
    match crate::net::trusted_roots(host).map_err(MailError::Connect)? {
        Some(certs) => {
            let (added, _) = roots.add_parsable_certificates(certs.iter().map(|c| rustls::pki_types::CertificateDer::from(c.der().to_vec())));
            if added == 0 {
                return Err(MailError::Connect("no usable certificate in the pins or CA files".into()));
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| MailError::Connect(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| MailError::Connect(format!("{host}: {e}")))?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| MailError::Connect(e.to_string()))?;
    let mut tcp = tcp;
    // Finish the handshake now, so a bad certificate is reported as such.
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp).map_err(|e| MailError::Connect(format!("TLS with {host}: {e}")))?;
    }
    Ok(rustls::StreamOwned::new(conn, tcp))
}

fn is_local(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// Open a connection, greet, upgrade to TLS as configured and log in.
fn connect(cfg: &MailConfig, password: Option<&str>) -> Result<Session, MailError> {
    if cfg.host.is_empty() {
        return Err(MailError::Message("set the mail server first".into()));
    }
    if !cfg.username.is_empty() && password.is_none_or(str::is_empty) {
        return Err(MailError::Message("set the mail password first".into()));
    }
    crate::net::check_host(&cfg.host).map_err(MailError::Connect)?;
    if cfg.security == Security::None && !is_local(&cfg.host) {
        return Err(MailError::Message("unencrypted SMTP is only allowed to this computer; use TLS or STARTTLS".into()));
    }
    let timeout = Duration::from_secs(cfg.timeout_secs.max(1));
    let addrs = (cfg.host.as_str(), cfg.port).to_socket_addrs().map_err(|e| MailError::Connect(format!("{}: {e}", cfg.host)))?;
    let mut last = None;
    let tcp = addrs
        .into_iter()
        .find_map(|a| TcpStream::connect_timeout(&a, timeout).map_err(|e| last = Some(e)).ok())
        .ok_or_else(|| MailError::Connect(format!("{}:{}: {}", cfg.host, cfg.port, last.map(|e| e.to_string()).unwrap_or_else(|| "no address".into()))))?;
    tcp.set_read_timeout(Some(timeout)).map_err(io)?;
    tcp.set_write_timeout(Some(timeout)).map_err(io)?;
    let wire = match cfg.security {
        Security::Tls => Wire::Tls(Box::new(BufReader::new(tls(tcp, &cfg.host)?))),
        _ => Wire::Plain(BufReader::new(tcp)),
    };
    let mut s = Session { wire };
    s.reply(220)?;
    let mut caps = s.ehlo()?;
    if cfg.security == Security::StartTls {
        if !caps.iter().any(|c| c.eq_ignore_ascii_case("STARTTLS")) {
            return Err(MailError::Connect(format!("{} does not offer STARTTLS; use TLS (port 465)", cfg.host)));
        }
        s.command("STARTTLS", 220)?;
        s = s.start_tls(&cfg.host)?;
        caps = s.ehlo()?;
    }
    auth(&mut s, cfg, password, &caps)?;
    Ok(s)
}

/// Log in with `AUTH PLAIN` when offered, else `AUTH LOGIN`. Nothing happens without a
/// username.
fn auth(s: &mut Session, cfg: &MailConfig, password: Option<&str>, caps: &[String]) -> Result<(), MailError> {
    if cfg.username.is_empty() {
        return Ok(());
    }
    let password = password.unwrap_or_default();
    if matches!(s.wire, Wire::Plain(_)) && !is_local(&cfg.host) {
        return Err(MailError::Message("refusing to send a password over an unencrypted connection".into()));
    }
    let mechs: Vec<String> = caps
        .iter()
        .filter_map(|c| {
            let up = c.to_ascii_uppercase();
            up.strip_prefix("AUTH ").or_else(|| up.strip_prefix("AUTH=")).map(str::to_string)
        })
        .flat_map(|m| m.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .collect();
    let b64 = |s: &str| crate::audio::base64(s.as_bytes());
    if mechs.iter().any(|m| m == "PLAIN") || mechs.is_empty() {
        s.command(&format!("AUTH PLAIN {}", b64(&format!("\0{}\0{password}", cfg.username))), 235)?;
    } else if mechs.iter().any(|m| m == "LOGIN") {
        s.command("AUTH LOGIN", 334)?;
        s.command(&b64(&cfg.username), 334)?;
        s.command(&b64(password), 235)?;
    } else {
        return Err(MailError::Message(format!("the server offers no login this app supports ({})", mechs.join(" "))));
    }
    Ok(())
}

/// Lines that start with a dot get one more (RFC 5321 §4.5.2), and the end marker follows.
fn dot_stuff(wire: &str) -> String {
    let mut out = String::with_capacity(wire.len() + 64);
    for line in wire.split_inclusive("\r\n") {
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
    }
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out.push_str(".\r\n");
    out
}

/// Send one message. `password` is needed when `cfg.username` is set.
///
/// ```no_run
/// # use desktop_core::mail::*;
/// # let cfg = MailConfig::default();
/// let msg = Message { to: vec!["you@example.com".into()], subject: "Hi".into(), text: "Hello".into(), ..Message::default() };
/// let sent = send(&cfg, Some("secret"), &msg)?;
/// assert_eq!(sent.recipients, 1);
/// # Ok::<(), MailError>(())
/// ```
pub fn send(cfg: &MailConfig, password: Option<&str>, msg: &Message) -> Result<Sent, MailError> {
    if !cfg.enabled {
        return Err(MailError::Message("email is turned off in settings".into()));
    }
    let from = address(&cfg.from).map_err(|_| MailError::Message("set a valid sender (From) first".into()))?;
    let (wire, message_id) = render(&cfg.from, msg)?;
    if wire.len() as u64 > cfg.max_mb * 1024 * 1024 {
        return Err(MailError::Message(format!("the message is {} MB, over the {} MB limit", wire.len() / (1024 * 1024), cfg.max_mb)));
    }
    let rcpts = msg.to.iter().chain(&msg.cc).chain(&msg.bcc).map(|a| address(a)).collect::<Result<Vec<_>, _>>()?;
    let mut s = connect(cfg, password)?;
    s.command(&format!("MAIL FROM:<{from}>"), 250)?;
    for r in &rcpts {
        s.command(&format!("RCPT TO:<{r}>"), 250)?;
    }
    s.command("DATA", 354)?;
    s.wire.write_all(dot_stuff(&wire).as_bytes()).map_err(io)?;
    s.reply(250)?;
    let _ = s.command("QUIT", 221);
    Ok(Sent { message_id, recipients: rcpts.len() })
}

/// Connect and log in without sending anything: checks the server, TLS and password.
pub fn test(cfg: &MailConfig, password: Option<&str>) -> Result<(), MailError> {
    let mut s = connect(cfg, password)?;
    let _ = s.command("QUIT", 221);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A tiny SMTP server: answers each command in turn and records what it got.
    fn fake_server(auth: &'static str) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut w = stream;
            let mut got = Vec::new();
            w.write_all(b"220 test ESMTP\r\n").unwrap();
            let mut data = false;
            let mut login_step = 0;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                got.push(line.clone());
                if data {
                    if line == ".\r\n" {
                        data = false;
                        w.write_all(b"250 queued\r\n").unwrap();
                    }
                    continue;
                }
                let up = line.to_ascii_uppercase();
                let reply: String = if up.starts_with("EHLO") {
                    format!("250-test\r\n250-SIZE 1000000\r\n250 AUTH {auth}\r\n")
                } else if up.starts_with("AUTH PLAIN") {
                    "235 ok\r\n".into()
                } else if up.starts_with("AUTH LOGIN") {
                    login_step = 1;
                    "334 VXNlcm5hbWU6\r\n".into()
                } else if login_step == 1 {
                    login_step = 2;
                    "334 UGFzc3dvcmQ6\r\n".into()
                } else if login_step == 2 {
                    login_step = 0;
                    "235 ok\r\n".into()
                } else if up.starts_with("RCPT TO:<NOBODY") {
                    "550 no such user\r\n".into()
                } else if up.starts_with("DATA") {
                    data = true;
                    "354 go\r\n".into()
                } else if up.starts_with("QUIT") {
                    w.write_all(b"221 bye\r\n").unwrap();
                    break;
                } else {
                    "250 ok\r\n".into()
                };
                w.write_all(reply.as_bytes()).unwrap();
            }
            got
        });
        (port, h)
    }

    fn cfg(port: u16) -> MailConfig {
        MailConfig { enabled: true, host: "127.0.0.1".into(), port, security: Security::None, username: "me".into(), from: "Me <me@example.com>".into(), ..MailConfig::default() }
    }

    fn msg() -> Message {
        Message {
            to: vec!["You <you@example.com>".into()],
            bcc: vec!["hidden@example.com".into()],
            subject: "Grüße".into(),
            text: "line one\r\n.starts with a dot".into(),
            attachments: vec![Attachment { name: "a.txt".into(), content_type: String::new(), data: b"hello".to_vec() }],
            ..Message::default()
        }
    }

    #[test]
    fn sends_with_auth_plain_and_attachment() {
        let (port, h) = fake_server("PLAIN LOGIN");
        let sent = send(&cfg(port), Some("pw"), &msg()).unwrap();
        assert_eq!(sent.recipients, 2);
        let got = h.join().unwrap().concat();
        assert!(got.contains(&format!("AUTH PLAIN {}", crate::audio::base64(b"\0me\0pw"))));
        assert!(got.contains("MAIL FROM:<me@example.com>"));
        assert!(got.contains("RCPT TO:<you@example.com>") && got.contains("RCPT TO:<hidden@example.com>"));
        assert!(got.contains("Subject: =?UTF-8?B?"));
        assert!(got.contains("filename=\"a.txt\"") && got.contains("Content-Type: text/plain"));
        assert!(!got.contains("Bcc"), "bcc must not appear in headers");
        assert!(got.contains(&sent.message_id));
    }

    #[test]
    fn falls_back_to_auth_login() {
        let (port, h) = fake_server("LOGIN");
        test(&cfg(port), Some("pw")).unwrap();
        let got = h.join().unwrap().concat();
        assert!(got.contains("AUTH LOGIN") && got.contains(&crate::audio::base64(b"me")) && got.contains(&crate::audio::base64(b"pw")));
    }

    #[test]
    fn server_errors_come_back() {
        let (port, _h) = fake_server("PLAIN");
        let m = Message { to: vec!["nobody@example.com".into()], text: "x".into(), ..Message::default() };
        match send(&cfg(port), Some("pw"), &m) {
            Err(MailError::Server { code: 550, .. }) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refuses_unsafe_setups() {
        let mut c = cfg(25);
        assert!(matches!(send(&c, None, &msg()), Err(MailError::Message(_))), "needs a password");
        c.host = "smtp.example.com".into();
        assert!(matches!(test(&c, Some("pw")), Err(MailError::Message(_))), "plain SMTP off this computer");
        c.enabled = false;
        assert!(send(&c, Some("pw"), &msg()).is_err());
    }

    #[test]
    fn addresses_and_headers() {
        assert_eq!(address("Me <me@example.com>").unwrap(), "me@example.com");
        assert_eq!(address(" me@example.com ").unwrap(), "me@example.com");
        for bad in ["", "me", "me@local", "a@b.c\r\nBcc: x@y.z", "<a@b.c>\nX", "a b@c.d"] {
            assert!(address(bad).is_err(), "{bad:?}");
        }
        assert!(header_text("x\r\nBcc: evil@example.com").is_err());
        assert_eq!(mailbox("Zoë <z@example.com>").unwrap(), format!("=?UTF-8?B?{}?= <z@example.com>", crate::audio::base64("Zoë".as_bytes())));
    }

    #[test]
    fn renders_html_alternative_and_dot_stuffing() {
        let m = Message { to: vec!["a@example.com".into()], html: Some("<b>hi</b>".into()), text: "hi".into(), ..Message::default() };
        let (wire, id) = render("me@example.com", &m).unwrap();
        assert!(wire.contains("multipart/alternative") && wire.contains("text/html") && wire.contains(&id));
        assert!(id.ends_with("@example.com>"));
        assert_eq!(dot_stuff("a\r\n.b\r\n"), "a\r\n..b\r\n.\r\n");
        assert!(render("me@example.com", &Message::default()).is_err());
    }

    #[test]
    fn date_is_rfc5322() {
        let t = std::time::UNIX_EPOCH + Duration::from_secs(1_795_000_000);
        let d = rfc5322_date(t);
        assert!(d.ends_with(" +0000") && d.len() == 31, "{d}");
        assert_eq!(rfc5322_date(std::time::UNIX_EPOCH), "Thu, 01 Jan 1970 00:00:00 +0000");
    }

    #[test]
    fn password_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = PasswordFile::new(dir.path());
        assert_eq!(p.get(), None);
        p.set("s3cret").unwrap();
        assert_eq!(p.get().as_deref(), Some("s3cret"));
        p.set("").unwrap();
        assert_eq!(p.get(), None);
    }

    /// Real servers: `cargo test -p desktop-core --features mail -- --ignored mail`.
    #[test]
    #[ignore]
    fn real_tls_and_starttls_handshakes() {
        for (port, security) in [(465, Security::Tls), (587, Security::StartTls)] {
            let c = MailConfig { enabled: true, host: "smtp.gmail.com".into(), port, security, ..MailConfig::default() };
            test(&c, None).unwrap();
        }
    }
}
