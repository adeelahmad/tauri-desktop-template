//! "Open in Browser…": the page the window shows, opened in a real browser. It is the way
//! out for a site that breaks in the web view (WebKit on macOS and Linux).
//!
//! Two rules keep it safe:
//!
//! - **Only the current page of the wrapped site**, and only `http(s)`: [`page_url`]
//!   refuses another origin, other schemes, `about:` pages and ambiguous URLs (user info,
//!   spaces, backslashes), so the page cannot use the menu item to open anything else.
//! - **No shell.** The browser is started with an argument list ([`launch`]). The developer
//!   option `wrapper.openInBrowserApp` names the browser; empty (the default) means the
//!   user's default browser. It is checked in [`WrapperConfig::check`](super::WrapperConfig::check)
//!   with [`check_app`]: plain app name characters only.
//!
//! ```
//! use desktop_core::wrapper::browser::{check_app, launch, page_url};
//! use desktop_core::wrapper::url::WebUrl;
//!
//! let site = WebUrl::parse("https://pikvm.local/")?.origin();
//! assert_eq!(page_url(&site, "https://PiKVM.local:443/kvm/?x=1#top")?, "https://pikvm.local/kvm/?x=1#top");
//! assert!(page_url(&site, "https://github.com/").is_err()); // another site
//! assert!(page_url(&site, "file:///etc/passwd").is_err());
//!
//! assert!(check_app("Google Chrome").is_ok());
//! assert!(check_app("Chrome; rm -rf ~").is_err());
//!
//! // macOS: `open -a <app> <url>`; elsewhere the app is a program on PATH.
//! let l = launch("Google Chrome", "https://pikvm.local/", true).unwrap();
//! assert_eq!((l.program.as_str(), l.args.as_slice()), ("open", &["-a".to_string(), "Google Chrome".into(), "https://pikvm.local/".into()][..]));
//! assert!(launch("", "https://pikvm.local/", true).is_none()); // the default browser
//! # Ok::<(), String>(())
//! ```

use super::url::{Origin, WebUrl};

/// The longest app name `openInBrowserApp` takes.
pub const MAX_APP_NAME: usize = 100;

/// Check `wrapper.openInBrowserApp`: empty (the default browser), or a plain app name such
/// as `Google Chrome`, `Firefox` or `firefox`: letters, digits, spaces, `.`, `-`, `_` and
/// `+`, starting with a letter or digit, at most [`MAX_APP_NAME`] characters. Paths,
/// quotes, shell characters, `..` and Windows batch files (`.bat`, `.cmd`) are refused.
pub fn check_app(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Ok(());
    }
    let plain = name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_' | '+'));
    let lower = name.to_lowercase();
    if name.chars().count() > MAX_APP_NAME
        || !plain
        || !name.starts_with(char::is_alphanumeric)
        || name.ends_with(' ')
        || name.contains("..")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
    {
        return Err(format!(
            "{name:?} is not a plain app name (letters, digits, spaces, . - _ +; at most {MAX_APP_NAME} characters; empty for the default browser)"
        ));
    }
    Ok(())
}

/// The URL to open for the page the window shows: `page` (the site's own URL of the
/// current page, not the proxy's) when it is `http(s)` on the wrapped `origin`, written
/// out again from its parsed parts.
pub fn page_url(origin: &Origin, page: &str) -> Result<String, String> {
    let u = WebUrl::parse(page).map_err(|e| format!("not an http(s) page: {e}"))?;
    if u.origin() != *origin {
        return Err(format!("{} is not the wrapped site", u.origin()));
    }
    Ok(u.to_string())
}

/// A program and its arguments, started without a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The program.
    pub program: String,
    /// Its arguments, one each.
    pub args: Vec<String>,
}

/// How to open `url` in `app` (a name [`check_app`] accepted): on macOS
/// `open -a <app> <url>`, elsewhere `<app> <url>` (a program on `PATH`, such as `firefox`).
/// `None` when `app` is empty: the caller opens the default browser.
pub fn launch(app: &str, url: &str, macos: bool) -> Option<Launch> {
    if app.is_empty() {
        return None;
    }
    Some(if macos {
        Launch { program: "open".into(), args: vec!["-a".into(), app.into(), url.into()] }
    } else {
        Launch { program: app.into(), args: vec![url.into()] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_app_names_only() {
        for ok in ["", "Google Chrome", "Firefox", "firefox", "Microsoft Edge", "Firefox Developer Edition", "Brave Browser", "google-chrome-stable", "Opera GX", "Navigateur Ünï", "chrome.exe"] {
            assert!(check_app(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            " Chrome",
            "Chrome ",
            "-a",
            "/Applications/Safari.app",
            "..\\evil",
            "Chrome; rm -rf ~",
            "Chrome && x",
            "$(id)",
            "`id`",
            "\"Chrome\"",
            "Chrome\n",
            "a..b",
            "run.bat",
            "RUN.CMD",
            "C:evil",
            &"a".repeat(MAX_APP_NAME + 1),
        ] {
            assert!(check_app(bad).is_err(), "{bad:?}");
        }
        assert!(check_app(&"a".repeat(MAX_APP_NAME)).is_ok());
    }

    #[test]
    fn only_the_current_page_of_the_site() {
        let site = WebUrl::parse("https://pikvm.local:8443/kvm/").unwrap().origin();
        assert_eq!(page_url(&site, "https://pikvm.local:8443/kvm/").unwrap(), "https://pikvm.local:8443/kvm/");
        assert_eq!(page_url(&site, "https://pikvm.local:8443").unwrap(), "https://pikvm.local:8443/");
        for bad in [
            "https://pikvm.local/",
            "http://pikvm.local:8443/",
            "https://evil.com/",
            "https://pikvm.local:8443@evil.com/",
            "about:blank",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://pikvm.local:8443/a b",
            "",
        ] {
            assert!(page_url(&site, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn launched_with_an_argument_list() {
        let url = "https://h.example/?a=1&b=$(id)";
        assert_eq!(launch("", url, false), None);
        assert_eq!(launch("firefox", url, false), Some(Launch { program: "firefox".into(), args: vec![url.into()] }));
        assert_eq!(
            launch("Google Chrome", url, true),
            Some(Launch { program: "open".into(), args: vec!["-a".into(), "Google Chrome".into(), url.into()] })
        );
    }
}
