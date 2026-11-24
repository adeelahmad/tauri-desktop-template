//! Reach this app's local endpoints (an MCP or HTTP server on `127.0.0.1`) from your own
//! devices over [Tailscale](https://tailscale.com), without opening a port to the internet.
//!
//! This module only drives the `tailscale` command-line tool that comes with the Tailscale
//! app; it never installs system software. If Tailscale is missing, [`status`] says so and
//! gives [`INSTALL_URL`]. With `serve` on, [`apply`] runs `tailscale serve` so
//! `https://<this-machine>.<tailnet>.ts.net/` forwards to the configured local port; only
//! devices in your tailnet can reach it (this never uses Funnel, which is public). Turning
//! it off removes only the mapping this app made.
//!
//! ```
//! use desktop_core::tailscale::{parse_status, serve_args, TailscaleConfig};
//!
//! let status = parse_status(r#"{"Version": "1.80.0", "BackendState": "Running",
//!     "Self": {"HostName": "laptop", "DNSName": "laptop.tail1234.ts.net.", "TailscaleIPs": ["100.64.0.1"]},
//!     "CurrentTailnet": {"Name": "me@example.com"}}"#).unwrap();
//! assert!(status.running);
//! assert_eq!(status.dns_name, "laptop.tail1234.ts.net");
//!
//! let config = TailscaleConfig { enabled: true, serve: true, port: 8765, ..TailscaleConfig::default() };
//! assert_eq!(serve_args(&config).unwrap(), ["serve", "--bg", "--https=443", "http://127.0.0.1:8765"]);
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where people get Tailscale (shown instead of installing anything).
pub const INSTALL_URL: &str = "https://tailscale.com/download";

/// HTTPS ports `tailscale serve` accepts.
pub const HTTPS_PORTS: [u16; 3] = [443, 8443, 10000];

#[derive(Debug, thiserror::Error)]
pub enum TailscaleError {
    #[error("Tailscale is not installed; get it from {INSTALL_URL}")]
    NotInstalled,
    #[error("{0}")]
    Config(String),
    #[error("tailscale: {0}")]
    Failed(String),
    #[error("could not read tailscale's reply: {0}")]
    Parse(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = TailscaleError> = std::result::Result<T, E>;

/// Tailscale settings (`tailscale` in settings). Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TailscaleConfig {
    pub enabled: bool,
    /// Share `port` with the tailnet through `tailscale serve`.
    pub serve: bool,
    /// The local port to share (on 127.0.0.1); 0 = none.
    pub port: u16,
    /// The HTTPS port on the tailnet: 443, 8443 or 10000.
    pub https_port: u16,
}

impl Default for TailscaleConfig {
    fn default() -> Self {
        Self { enabled: false, serve: false, port: 0, https_port: 443 }
    }
}

impl TailscaleConfig {
    pub fn validated(mut self) -> Self {
        if !HTTPS_PORTS.contains(&self.https_port) {
            self.https_port = 443;
        }
        self
    }

    /// The local address `serve` forwards to.
    pub fn target(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// What `tailscale status --json` says about this machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub version: String,
    /// `Running`, `NeedsLogin`, `Stopped`, ...
    pub backend_state: String,
    pub running: bool,
    pub host_name: String,
    /// This machine's MagicDNS name, without the trailing dot.
    pub dns_name: String,
    pub ips: Vec<String>,
    pub tailnet: String,
    /// Where to log in, when `NeedsLogin`.
    pub auth_url: String,
}

/// Parse `tailscale status --json` (unknown fields are ignored; missing ones are empty).
pub fn parse_status(json: &str) -> Result<Status> {
    let v: Value = serde_json::from_str(json).map_err(|e| TailscaleError::Parse(e.to_string()))?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let me = v.get("Self").cloned().unwrap_or(Value::Null);
    let ips = me.get("TailscaleIPs").or_else(|| v.get("TailscaleIPs")).and_then(Value::as_array).cloned().unwrap_or_default();
    let backend_state = s(&v, "BackendState");
    Ok(Status {
        version: s(&v, "Version"),
        running: backend_state == "Running",
        backend_state,
        host_name: s(&me, "HostName"),
        dns_name: s(&me, "DNSName").trim_end_matches('.').to_string(),
        ips: ips.iter().filter_map(Value::as_str).map(String::from).collect(),
        tailnet: v.get("CurrentTailnet").map(|t| s(t, "Name")).unwrap_or_default(),
        auth_url: s(&v, "AuthURL"),
    })
}

/// What `tailscale serve status --json` forwards from `https_port`'s root path, if anything.
pub fn parse_served(json: &str, https_port: u16) -> Result<Option<String>> {
    let v: Value = if json.trim().is_empty() { Value::Null } else { serde_json::from_str(json).map_err(|e| TailscaleError::Parse(e.to_string()))? };
    let suffix = format!(":{https_port}");
    let found = v.get("Web").and_then(Value::as_object).and_then(|web| {
        web.iter()
            .filter(|(host, _)| host.ends_with(&suffix))
            .find_map(|(_, site)| site.pointer("/Handlers/~1/Proxy").and_then(Value::as_str).map(String::from))
    });
    Ok(found)
}

/// `tailscale serve` arguments that share `cfg.port` with the tailnet.
pub fn serve_args(cfg: &TailscaleConfig) -> Result<Vec<String>> {
    if cfg.port == 0 {
        return Err(TailscaleError::Config("choose the local port to share".into()));
    }
    Ok(vec!["serve".into(), "--bg".into(), format!("--https={}", cfg.https_port), cfg.target()])
}

/// `tailscale serve` arguments that stop sharing `cfg.https_port`.
pub fn unserve_args(cfg: &TailscaleConfig) -> Vec<String> {
    vec!["serve".into(), format!("--https={}", cfg.https_port), "off".into()]
}

/// The first `https://` link in `text` (tailscale prints one when the tailnet admin has to
/// switch something on first, such as HTTPS certificates).
pub fn find_link(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    Some(text[start..].split(|c: char| c.is_whitespace() || c == '"').next().unwrap_or_default().to_string())
}

/// The `tailscale` command-line tool: on `PATH`, or where the Tailscale app puts it.
pub fn find_cli() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "tailscale.exe" } else { "tailscale" };
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).map(|d| d.join(exe));
    let known = [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/usr/bin/tailscale",
        r"C:\Program Files\Tailscale\tailscale.exe",
    ]
    .map(PathBuf::from);
    on_path.chain(known).find(|p| p.is_file())
}

/// Run the CLI with a time limit; returns `(succeeded, stdout, stderr)`. A command still
/// running at the limit (for example waiting for an admin to approve) is stopped.
fn run(cli: &Path, args: &[String], limit: Duration) -> Result<(bool, String, String)> {
    let mut cmd = std::process::Command::new(cli);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn()?;
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    };
    let out = pipe(child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let err = pipe(child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let deadline = Instant::now() + limit;
    let ok = loop {
        if let Some(st) = child.try_wait()? {
            break st.success();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok((ok, out.join().unwrap_or_default(), err.join().unwrap_or_default()))
}

/// What Settings shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub installed: bool,
    pub install_url: String,
    pub status: Option<Status>,
    /// What the configured HTTPS port forwards to now.
    pub served: Option<String>,
    /// The tailnet address of the shared port, when this app's mapping is active.
    pub url: Option<String>,
    /// A link to follow (log in, or allow HTTPS in the tailnet), or a problem.
    pub note: Option<String>,
}

/// Look for Tailscale and read its state. Never fails: problems go in `note`.
pub fn status(cfg: &TailscaleConfig) -> Report {
    let mut report = Report { install_url: INSTALL_URL.into(), ..Report::default() };
    let Some(cli) = find_cli() else { return report };
    report.installed = true;
    let limit = Duration::from_secs(10);
    match run(&cli, &["status".into(), "--json".into()], limit) {
        // `status` exits non-zero when logged out but still prints the JSON.
        Ok((_, out, err)) => match parse_status(&out) {
            Ok(st) => {
                if !st.auth_url.is_empty() {
                    report.note = Some(format!("Log in to Tailscale: {}", st.auth_url));
                }
                report.status = Some(st);
            }
            Err(e) => report.note = Some(if err.trim().is_empty() { e.to_string() } else { err.trim().to_string() }),
        },
        Err(e) => report.note = Some(e.to_string()),
    }
    if let Ok((true, out, _)) = run(&cli, &["serve".into(), "status".into(), "--json".into()], limit) {
        report.served = parse_served(&out, cfg.https_port).ok().flatten();
    }
    if let (Some(st), Some(served)) = (&report.status, &report.served) {
        if *served == cfg.target() && !st.dns_name.is_empty() {
            let port = if cfg.https_port == 443 { String::new() } else { format!(":{}", cfg.https_port) };
            report.url = Some(format!("https://{}{port}/", st.dns_name));
        }
    }
    report
}

/// Make `tailscale serve` match the settings: share the port when `enabled` and `serve`
/// are on, otherwise stop sharing it (only if this app's mapping is the one there).
pub fn apply(cfg: &TailscaleConfig) -> Result<Report> {
    let cli = find_cli().ok_or(TailscaleError::NotInstalled)?;
    let before = status(cfg);
    let ours = before.served.as_deref() == Some(cfg.target().as_str());
    let args = if cfg.enabled && cfg.serve {
        if before.served.is_some() && !ours {
            return Err(TailscaleError::Config(format!("HTTPS port {} already forwards elsewhere; pick another port", cfg.https_port)));
        }
        serve_args(cfg)?
    } else if ours {
        unserve_args(cfg)
    } else {
        return Ok(before);
    };
    let (ok, out, err) = run(&cli, &args, Duration::from_secs(20))?;
    if !ok {
        let text = format!("{out}\n{err}");
        return Err(TailscaleError::Failed(match find_link(&text) {
            Some(link) => format!("approve this in your tailnet first: {link}"),
            None if text.contains("Access denied") || text.contains("operator") => {
                "not allowed; on Linux run `sudo tailscale set --operator=$USER` once".into()
            }
            None => text.trim().to_string(),
        }));
    }
    Ok(status(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: &str = r#"{
      "Version": "1.80.2-t1234", "BackendState": "Running", "AuthURL": "",
      "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0::1"],
      "Self": {"ID": "n1", "HostName": "studio", "DNSName": "studio.tail9f00.ts.net.", "OS": "macOS",
               "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0::1"], "Online": true},
      "MagicDNSSuffix": "tail9f00.ts.net",
      "CurrentTailnet": {"Name": "me@example.com", "MagicDNSSuffix": "tail9f00.ts.net", "MagicDNSEnabled": true},
      "Peer": {"k": {"HostName": "phone"}}
    }"#;

    #[test]
    fn parses_a_running_node() {
        let s = parse_status(RUNNING).unwrap();
        assert!(s.running);
        assert_eq!(s.dns_name, "studio.tail9f00.ts.net");
        assert_eq!(s.host_name, "studio");
        assert_eq!(s.ips, ["100.101.102.103", "fd7a:115c:a1e0::1"]);
        assert_eq!(s.tailnet, "me@example.com");
        assert!(s.version.starts_with("1.80"));
    }

    #[test]
    fn parses_a_logged_out_node() {
        let s = parse_status(r#"{"BackendState": "NeedsLogin", "AuthURL": "https://login.tailscale.com/a/abc", "Self": null}"#).unwrap();
        assert!(!s.running);
        assert_eq!(s.auth_url, "https://login.tailscale.com/a/abc");
        assert!(s.dns_name.is_empty() && s.ips.is_empty());
        assert!(parse_status("not json").is_err());
    }

    #[test]
    fn reads_what_is_served() {
        let json = r#"{"TCP": {"443": {"HTTPS": true}},
          "Web": {"studio.tail9f00.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8765"}}},
                  "studio.tail9f00.ts.net:8443": {"Handlers": {"/docs": {"Path": "/srv"}}}}}"#;
        assert_eq!(parse_served(json, 443).unwrap().as_deref(), Some("http://127.0.0.1:8765"));
        assert_eq!(parse_served(json, 8443).unwrap(), None);
        assert_eq!(parse_served("{}", 443).unwrap(), None);
        assert_eq!(parse_served("", 443).unwrap(), None);
    }

    #[test]
    fn builds_serve_commands() {
        let c = TailscaleConfig { enabled: true, serve: true, port: 3141, https_port: 8443 };
        assert_eq!(serve_args(&c).unwrap(), ["serve", "--bg", "--https=8443", "http://127.0.0.1:3141"]);
        assert_eq!(unserve_args(&c), ["serve", "--https=8443", "off"]);
        assert!(serve_args(&TailscaleConfig { port: 0, ..c }).is_err());
        assert_eq!(TailscaleConfig { https_port: 80, ..c }.validated().https_port, 443);
    }

    #[test]
    fn finds_approval_links() {
        let out = "Serve is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/serve?node=abc\n";
        assert_eq!(find_link(out).as_deref(), Some("https://login.tailscale.com/f/serve?node=abc"));
        assert_eq!(find_link("nothing here"), None);
    }

    #[test]
    fn status_never_fails() {
        let r = status(&TailscaleConfig::default());
        assert_eq!(r.install_url, INSTALL_URL);
        assert_eq!(r.installed, find_cli().is_some());
    }
}
