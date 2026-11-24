//! Header and auto-fill rules: URL patterns, values from the environment or the OS
//! keychain, and masking.
//!
//! **Patterns.** A glob (`*` any run of characters, `?` one) or, with the prefix `re:`, a
//! regular expression. A pattern that starts with `/` is relative to the site's base URL;
//! anything else is matched against the whole URL (`https://cdn.example.com/*`).
//!
//! **Values.** Literal text, `${NAME}` taken from the user's environment when the app
//! starts (on macOS from the login shell, since apps started from the Dock do not get the
//! shell's variables), `config:<path>` from the app's build-time `config/`, or
//! `keychain:<name>` from the OS keychain (never stored in a file). `$$` is a literal `$`.
//!
//! **Masking.** Values are never logged; Preferences shows literals as [`MASK`] and
//! references (`${TOKEN}`, `keychain:pw`) as they are, since those are not secrets.
//!
//! ```
//! use desktop_core::wrapper::rules::{interpolate, mask, Pattern};
//!
//! let p = Pattern::parse("/api/*", "https://pikvm.local").unwrap();
//! assert!(p.matches("https://pikvm.local/api/hid"));
//! assert!(!p.matches("https://pikvm.local/login"));
//! assert!(Pattern::parse("re:^https://[a-z]+\\.example\\.com/", "").unwrap().matches("https://cdn.example.com/x"));
//!
//! let env = |k: &str| (k == "KVM_TOKEN").then(|| "t0ken".to_string());
//! assert_eq!(interpolate("Bearer ${KVM_TOKEN}", &env).unwrap(), "Bearer t0ken");
//! assert!(interpolate("${MISSING}", &env).unwrap_err().contains("MISSING is not set"));
//!
//! assert_eq!(mask("Bearer abc"), "••••••••");
//! assert_eq!(mask("${KVM_TOKEN}"), "${KVM_TOKEN}");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What Preferences shows instead of a literal value; sending it back keeps the old value.
pub const MASK: &str = "••••••••";

/// A compiled URL pattern.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// A glob over the whole URL.
    Glob(String),
    /// A regular expression over the whole URL.
    Regex(regex::Regex),
}

impl Pattern {
    /// Compile `text`; `base` is the site's base URL for relative patterns.
    pub fn parse(text: &str, base: &str) -> Result<Self, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("empty pattern".into());
        }
        if let Some(re) = t.strip_prefix("re:") {
            if re.len() > 1000 {
                return Err("pattern too long".into());
            }
            return regex::RegexBuilder::new(re)
                .size_limit(1 << 20)
                .build()
                .map(Pattern::Regex)
                .map_err(|e| format!("bad regular expression: {e}"));
        }
        if t.chars().any(char::is_control) || t.len() > 1000 {
            return Err("bad pattern".into());
        }
        Ok(Pattern::Glob(if t.starts_with('/') { format!("{}{t}", base.trim_end_matches('/')) } else { t.to_string() }))
    }

    /// Whether `url` matches.
    pub fn matches(&self, url: &str) -> bool {
        match self {
            Pattern::Glob(g) => glob(g.as_bytes(), url.as_bytes()),
            Pattern::Regex(r) => r.is_match(url),
        }
    }
}

/// `*` and `?` wildcard match (iterative, no exponential backtracking).
fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == b'*')
}

/// Replace `${NAME}` with `env(NAME)`; a missing variable is an error that names it.
pub fn interpolate(spec: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = spec;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        if let Some(t) = tail.strip_prefix('$') {
            out.push('$');
            rest = t;
        } else if let Some(t) = tail.strip_prefix('{') {
            let end = t.find('}').ok_or("unclosed ${")?;
            let name = &t[..end];
            if !valid_var(name) {
                return Err(format!("{name:?} is not an environment variable name"));
            }
            out.push_str(&env(name).ok_or_else(|| format!("environment variable {name} is not set"))?);
            rest = &t[end + 1..];
        } else {
            out.push('$');
            rest = tail;
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn valid_var(name: &str) -> bool {
    let mut c = name.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// Where a value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source<'a> {
    /// Text, with `${NAME}` taken from the environment.
    Text(&'a str),
    /// A dot path in the app's build-time config.
    Config(&'a str),
    /// An entry in the OS keychain.
    Keychain(&'a str),
}

/// Which kind of value `spec` is.
pub fn source(spec: &str) -> Source<'_> {
    if let Some(p) = spec.strip_prefix("config:") {
        Source::Config(p.trim())
    } else if let Some(k) = spec.strip_prefix("keychain:") {
        Source::Keychain(k.trim())
    } else {
        Source::Text(spec)
    }
}

/// Check a value spec without resolving it.
pub fn check_value(spec: &str) -> Result<(), String> {
    match source(spec) {
        Source::Config("") => Err("config: needs a path".into()),
        Source::Keychain(k) if k.is_empty() || k.len() > 128 => Err("keychain: needs a name".into()),
        Source::Text(t) => interpolate(t, &|_| Some(String::new())).map(|_| ()),
        _ => Ok(()),
    }
}

/// How Preferences shows `spec`.
pub fn mask(spec: &str) -> String {
    match source(spec) {
        Source::Text(t) if !t.contains("${") => MASK.into(),
        _ => spec.to_string(),
    }
}

/// `new` from Preferences: [`MASK`] means "unchanged", so the old value is kept.
pub fn unmask(new: &str, old: Option<&str>) -> String {
    if new == MASK {
        old.unwrap_or_default().to_string()
    } else {
        new.to_string()
    }
}

/// One extra request header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HeaderRule {
    /// Which requests (see [`Pattern`]).
    pub pattern: String,
    /// The header name, e.g. `X-Api-Key`.
    pub name: String,
    /// The value (literal, `${ENV}`, `config:` or `keychain:`).
    pub value: String,
}

/// Headers the web view or HTTP itself owns: never set by a rule.
const RESERVED: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "keep-alive",
    "proxy-authorization",
    "proxy-connection",
    "cookie",
    "origin",
];

impl HeaderRule {
    /// Validate, and compile the pattern.
    pub fn check(&self, base: &str) -> Result<Pattern, String> {
        let n = self.name.trim();
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) {
            return Err("not a header name".into());
        }
        if RESERVED.contains(&n.to_ascii_lowercase().as_str()) || n.to_ascii_lowercase().starts_with("sec-") {
            return Err(format!("{n} is set by the web view, not by a rule"));
        }
        if self.value.contains(['\r', '\n', '\0']) || self.value.len() > 8192 {
            return Err("the value has a line break or is too long".into());
        }
        check_value(&self.value)?;
        Pattern::parse(&self.pattern, base)
    }

    /// This rule as Preferences shows it.
    pub fn masked(&self) -> Self {
        Self { value: mask(&self.value), ..self.clone() }
    }
}

/// Form fields to fill on matching pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AutofillRule {
    /// Which pages (see [`Pattern`]); only pages on the wrapped site are filled.
    pub pattern: String,
    /// CSS selector → value (literal, `${ENV}`, `config:` or `keychain:`).
    pub fields: BTreeMap<String, String>,
}

impl AutofillRule {
    /// Validate, and compile the pattern.
    pub fn check(&self, base: &str) -> Result<Pattern, String> {
        if self.fields.is_empty() {
            return Err("no fields".into());
        }
        for (sel, v) in &self.fields {
            if sel.trim().is_empty() || sel.len() > 256 || sel.contains(['\n', '\r', '\0']) {
                return Err(format!("{sel:?} is not a CSS selector"));
            }
            check_value(v).map_err(|e| format!("{sel}: {e}"))?;
        }
        Pattern::parse(&self.pattern, base)
    }

    /// This rule as Preferences shows it.
    pub fn masked(&self) -> Self {
        Self { pattern: self.pattern.clone(), fields: self.fields.iter().map(|(k, v)| (k.clone(), mask(v))).collect() }
    }
}

/// The script that fills `fields` (selector → resolved value) in the page: it sets each
/// input's value the way a person typing would (so frameworks see an `input` event).
/// Values travel as JSON, never spliced into code.
pub fn fill_script(fields: &[(String, String)]) -> String {
    let data = serde_json::to_string(fields).unwrap_or_else(|_| "[]".into()).replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    FILL_JS.replace("__DATA__", &data)
}

/// The fill script before its data is put in (no `format!`: its braces would clash with the
/// template engine the app was generated with).
const FILL_JS: &str = r#"(function (f) {
  f.forEach(function (p) {
    var el;
    try { el = document.querySelector(p[0]); } catch (e) { return; }
    if (!el) return;
    var d = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), "value");
    if (d && d.set) { d.set.call(el, p[1]); } else { el.value = p[1]; }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  });
})(__DATA__);"#;

/// The user's environment as a login shell sees it, read once (macOS; elsewhere the
/// process environment, which already has it). Waits at most three seconds.
pub fn login_env() -> &'static BTreeMap<String, String> {
    static ENV: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| {
        let mut env: BTreeMap<String, String> = std::env::vars().collect();
        if cfg!(target_os = "macos") {
            if let Some(extra) = shell_env(std::time::Duration::from_secs(3)) {
                for (k, v) in extra {
                    env.entry(k).or_insert(v);
                }
            }
        }
        env
    })
}

/// `$SHELL -l -c 'env -0'`, with a timeout; `None` when it fails or takes too long.
fn shell_env(timeout: std::time::Duration) -> Option<Vec<(String, String)>> {
    let shell = std::env::var("SHELL").ok().filter(|s| s.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
    let mut child = std::process::Command::new(shell)
        .args(["-l", "-c", "/usr/bin/env -0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        let _ = tx.send(buf);
    });
    let buf = rx.recv_timeout(timeout).ok();
    let _ = child.kill();
    let _ = child.wait();
    Some(parse_env0(&buf?))
}

/// `KEY=value\0` records.
fn parse_env0(buf: &[u8]) -> Vec<(String, String)> {
    buf.split(|b| *b == 0)
        .filter_map(|rec| {
            let s = String::from_utf8_lossy(rec);
            let (k, v) = s.split_once('=')?;
            valid_var(k).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_and_regexes() {
        let base = "https://h.example/kvm/";
        let rel = Pattern::parse("/api/*", base).unwrap();
        assert!(rel.matches("https://h.example/kvm/api/"));
        assert!(rel.matches("https://h.example/kvm/api/hid?x=1"));
        assert!(!rel.matches("https://h.example/api/hid"));
        assert!(!rel.matches("https://h.example.evil/kvm/api/"));
        let abs = Pattern::parse("https://*.example.com/*.js", base).unwrap();
        assert!(abs.matches("https://cdn.example.com/a/b.js"));
        assert!(!abs.matches("https://cdn.example.com/a/b.css"));
        let q = Pattern::parse("https://h.example/?", base).unwrap();
        assert!(q.matches("https://h.example/a") && !q.matches("https://h.example/ab"));
        assert!(Pattern::parse("*", base).unwrap().matches("anything"));
        let re = Pattern::parse(r"re:^https://h\.example/(api|ws)/", base).unwrap();
        assert!(re.matches("https://h.example/ws/x") && !re.matches("https://h.example/login"));
        assert!(Pattern::parse("re:(", base).is_err());
        assert!(Pattern::parse("", base).is_err());
        // No exponential blow-up on many stars.
        assert!(!glob(&b"*a".repeat(30), &b"a".repeat(200)[..].iter().chain(b"b").copied().collect::<Vec<_>>()));
    }

    #[test]
    fn env_interpolation() {
        let env = |k: &str| match k {
            "A" => Some("1".to_string()),
            "B_2" => Some("two".to_string()),
            _ => None,
        };
        assert_eq!(interpolate("x${A}y${B_2}", &env).unwrap(), "x1ytwo");
        assert_eq!(interpolate("cost $$5 and $x", &env).unwrap(), "cost $5 and $x");
        assert_eq!(interpolate("plain", &env).unwrap(), "plain");
        assert_eq!(interpolate("${NOPE}", &env).unwrap_err(), "environment variable NOPE is not set");
        assert!(interpolate("${A", &env).is_err());
        assert!(interpolate("${1A}", &env).is_err());
        assert_eq!(parse_env0(b"A=1\0B=x=y\0bad line\0=z\0"), [("A".into(), "1".into()), ("B".into(), "x=y".into())]);
    }

    #[test]
    fn sources_and_masking() {
        assert_eq!(source("config:wrapper.token"), Source::Config("wrapper.token"));
        assert_eq!(source("keychain:kvm-password"), Source::Keychain("kvm-password"));
        assert_eq!(source("abc"), Source::Text("abc"));
        assert_eq!(mask("secret"), MASK);
        assert_eq!(mask("Bearer ${T}"), "Bearer ${T}");
        assert_eq!(mask("keychain:pw"), "keychain:pw");
        assert_eq!(unmask(MASK, Some("old")), "old");
        assert_eq!(unmask("new", Some("old")), "new");
        let h = HeaderRule { pattern: "*".into(), name: "X-Key".into(), value: "s3cret".into() };
        assert_eq!(h.masked().value, MASK);
        assert!(!format!("{:?}", h.masked()).contains("s3cret"));
        assert!(check_value("keychain:").is_err());
        assert!(check_value("${OK}").is_ok());
    }

    #[test]
    fn header_rules_refuse_what_the_browser_owns() {
        let rule = |name: &str, value: &str| HeaderRule { pattern: "*".into(), name: name.into(), value: value.into() };
        assert!(rule("X-Api-Key", "v").check("").is_ok());
        for bad in ["Host", "content-length", "Cookie", "Sec-Fetch-Mode", "Bad Name", ""] {
            assert!(rule(bad, "v").check("").is_err(), "{bad}");
        }
        assert!(rule("X-A", "a\r\nInjected: 1").check("").is_err());
    }

    #[test]
    fn autofill_rules_and_script() {
        let r = AutofillRule {
            pattern: "/login".into(),
            fields: [("#user".to_string(), "admin".to_string()), ("#pass".into(), "keychain:kvm".into())].into(),
        };
        assert!(r.check("https://h").unwrap().matches("https://h/login"));
        assert_eq!(r.masked().fields["#user"], MASK);
        assert_eq!(r.masked().fields["#pass"], "keychain:kvm");
        assert!(AutofillRule { pattern: "*".into(), fields: BTreeMap::new() }.check("").is_err());
        let js = fill_script(&[("#user".into(), "a\"b</script>".into())]);
        assert!(js.contains(r##"[["#user","a\"b</script>"]]"##), "{js}");
    }
}
