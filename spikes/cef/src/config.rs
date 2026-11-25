//! What the spike opens and how: from the command line, then the environment.
//!
//! ```text
//! kitchen-cef-spike [--url https://site.example/path] [--pin <sha256> | --insecure-tls]
//!                   [--muted] [--allow-media camera,microphone,screen] [--user-agent <ua>]
//! ```
//!
//! Environment fallbacks: `KITCHEN_CEF_URL`, `KITCHEN_CEF_PIN`, `KITCHEN_CEF_INSECURE_TLS=1`,
//! `KITCHEN_CEF_MUTED=1`, `KITCHEN_CEF_MEDIA`, `KITCHEN_CEF_USER_AGENT`.

use crate::wrapper::{pin, url::WebUrl};

/// The site opened when nothing else is given.
pub const DEFAULT_URL: &str = "https://example.com/";

/// Which media a page on the site may use (everything else is refused).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaPolicy {
    /// `getUserMedia({video})`.
    pub camera: bool,
    /// `getUserMedia({audio})`.
    pub microphone: bool,
    /// `getDisplayMedia()`.
    pub screen: bool,
}

impl MediaPolicy {
    /// `camera,microphone,screen` (any subset; `none` or empty for nothing).
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut p = Self::default();
        for item in s.split(',').map(str::trim).filter(|i| !i.is_empty() && *i != "none") {
            match item {
                "camera" => p.camera = true,
                "microphone" | "mic" => p.microphone = true,
                "screen" | "display" => p.screen = true,
                other => return Err(format!("unknown media {other:?} (camera, microphone, screen)")),
            }
        }
        Ok(p)
    }
}

/// The spike's settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// The site (`https`).
    pub site: WebUrl,
    /// Go through the loopback proxy and accept the site's self-signed certificate, pinned.
    pub insecure_tls: bool,
    /// The fingerprint to require (`None` with `insecure_tls`: pin the first one seen).
    pub pin: Option<String>,
    /// Start muted.
    pub muted: bool,
    /// Media the site may use.
    pub media: MediaPolicy,
    /// User-Agent override (sent by the proxy, and by Chromium in direct mode).
    pub user_agent: Option<String>,
}

impl Config {
    /// From `args` (without the program name), falling back to `env`.
    pub fn parse(args: &[String], env: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut url = env("KITCHEN_CEF_URL");
        let mut pin_fp = env("KITCHEN_CEF_PIN");
        let mut insecure = env("KITCHEN_CEF_INSECURE_TLS").is_some_and(|v| v == "1");
        let mut muted = env("KITCHEN_CEF_MUTED").is_some_and(|v| v == "1");
        let mut media = env("KITCHEN_CEF_MEDIA");
        let mut user_agent = env("KITCHEN_CEF_USER_AGENT");
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let (key, inline) = match a.split_once('=') {
                Some((k, v)) if k.starts_with("--") => (k, Some(v.to_string())),
                _ => (a.as_str(), None),
            };
            let mut value = || inline.clone().or_else(|| it.next().cloned()).ok_or(format!("{key} needs a value"));
            match key {
                "--url" => url = Some(value()?),
                "--pin" => pin_fp = Some(value()?),
                "--insecure-tls" => insecure = true,
                "--muted" => muted = true,
                "--allow-media" => media = Some(value()?),
                "--user-agent" => user_agent = Some(value()?),
                // Chromium's own switches (and the helpers' `--type=`) are not ours.
                _ => {}
            }
        }
        let site = WebUrl::parse(url.as_deref().unwrap_or(DEFAULT_URL))?;
        if site.scheme != "https" {
            return Err(format!("{site}: the spike opens https sites only"));
        }
        let pin_fp = pin_fp.map(|p| pin::normalise(&p)).transpose()?;
        Ok(Self {
            insecure_tls: insecure || pin_fp.is_some(),
            pin: pin_fp,
            muted,
            media: media.as_deref().map(MediaPolicy::parse).transpose()?.unwrap_or_default(),
            user_agent: user_agent.filter(|u| !u.trim().is_empty()),
            site,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, String> {
        Config::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &|_| None)
    }

    #[test]
    fn defaults_and_flags() {
        let c = parse(&[]).unwrap();
        assert_eq!(c.site.to_string(), DEFAULT_URL);
        assert!(!c.insecure_tls && !c.muted && c.media == MediaPolicy::default());

        let fp = "AB:".repeat(31) + "AB";
        let c = parse(&["--url=https://kvm.local:8443/x", "--pin", &fp, "--muted", "--allow-media", "camera,mic"]).unwrap();
        assert_eq!(c.site.port, 8443);
        assert!(c.insecure_tls && c.muted);
        assert_eq!(c.pin.as_deref(), Some("ab".repeat(32).as_str()));
        assert_eq!(c.media, MediaPolicy { camera: true, microphone: true, screen: false });
    }

    #[test]
    fn refusals() {
        assert!(parse(&["--url", "http://plain.example"]).is_err());
        assert!(parse(&["--pin", "nope"]).is_err());
        assert!(parse(&["--allow-media", "gps"]).is_err());
        assert!(parse(&["--url"]).is_err());
        // Chromium switches pass through untouched.
        assert!(parse(&["--type=renderer", "--enable-logging"]).is_ok());
    }

    #[test]
    fn environment_fallback() {
        let env = |k: &str| match k {
            "KITCHEN_CEF_URL" => Some("https://env.example/".to_string()),
            "KITCHEN_CEF_INSECURE_TLS" => Some("1".to_string()),
            _ => None,
        };
        let c = Config::parse(&[], &env).unwrap();
        assert_eq!(c.site.host, "env.example");
        assert!(c.insecure_tls && c.pin.is_none());
    }
}
