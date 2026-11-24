//! A small WebDriver client for tauri-driver: just what feature tests need (start a
//! session on the app, find elements, type, click, read text), as plain HTTP and JSON.
//!
//! Each [`App`] gets its own tauri-driver on free ports and its own scratch home (so its
//! own settings, data folder and store, file or embedded Postgres), so tests run in
//! parallel and never touch your real data.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Errors are messages: a failing test should say what it was doing.
pub type Result<T> = std::result::Result<T, String>;

/// How long `find` waits for an element to appear.
pub const WAIT: Duration = Duration::from_secs(60);

/// The app under test, in a WebDriver session. Dropping it closes the app and the driver.
pub struct App {
    base: String,
    session: String,
    driver: Child,
    agent: ureq::Agent,
    home: PathBuf,
}

/// One element on the page.
pub struct Element<'a> {
    app: &'a App,
    id: String,
}

/// The built app: `APP_BINARY`, else `<target>/debug/<name>`.
pub fn app_binary(name: &str) -> PathBuf {
    if let Some(p) = std::env::var_os("APP_BINARY") {
        return PathBuf::from(p);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    target.join("debug").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Why this machine cannot run a feature test, if it cannot.
pub fn skip_reason(binary: &std::path::Path) -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("macOS has no WebDriver for WKWebView; `kitchen test --feature` runs `--self-test` there".into());
    }
    if Command::new("tauri-driver").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        return Some("tauri-driver is not installed: cargo install tauri-driver --locked".into());
    }
    if cfg!(target_os = "linux") {
        if Command::new("WebKitWebDriver").arg("--version").stdout(Stdio::null()).status().is_err() {
            return Some("WebKitWebDriver is missing: sudo apt install webkit2gtk-driver (or your distro's webkit2gtk driver)".into());
        }
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Some("no display: run under `xvfb-run -a`".into());
        }
    }
    if !binary.exists() {
        return Some(format!("{} is not built: KITCHEN_ENV=test cargo tauri build --debug --no-bundle", binary.display()));
    }
    None
}

/// A port nobody is listening on right now.
fn free_port() -> Result<u16> {
    TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| format!("find a free port: {e}"))
}

impl App {
    /// Start tauri-driver and open `binary` in a fresh session, with its config and data
    /// folders in a scratch home (XDG on Linux) so the test never sees your own data.
    pub fn launch(binary: &std::path::Path) -> Result<App> {
        let (port, native) = (free_port()?, free_port()?);
        let home = std::env::temp_dir().join(format!("e2e-home-{}-{port}", std::process::id()));
        std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        let driver = Command::new("tauri-driver")
            .args(["--port", &port.to_string(), "--native-port", &native.to_string()])
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("APPDATA", home.join("roaming"))
            .env("LOCALAPPDATA", home.join("local"))
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| format!("start tauri-driver: {e}"))?;
        let start = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if start.elapsed() > Duration::from_secs(20) {
                return Err(format!("tauri-driver did not listen on 127.0.0.1:{port}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let mut app = App { base: format!("http://127.0.0.1:{port}"), session: String::new(), driver, agent, home };
        let caps = json!({"capabilities": {"alwaysMatch": {"browserName": "wry", "tauri:options": {"application": binary}}}});
        let v = app.send("POST", "/session", Some(caps))?;
        app.session = v["sessionId"].as_str().ok_or_else(|| format!("no session in {v}"))?.to_string();
        Ok(app)
    }

    fn send(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let res = match (method, body) {
            ("GET", _) => self.agent.get(&url).call(),
            ("DELETE", _) => self.agent.delete(&url).call(),
            (_, b) => self.agent.post(&url).send_json(b.unwrap_or_else(|| json!({}))),
        };
        let mut res = res.map_err(|e| format!("{method} {path}: {e}"))?;
        let v: Value = res.body_mut().read_json().map_err(|e| format!("{method} {path}: {e}"))?;
        let value = v.get("value").cloned().unwrap_or(Value::Null);
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            return Err(format!("{method} {path}: {err}: {}", value["message"].as_str().unwrap_or_default()));
        }
        Ok(value)
    }

    fn find_by(&self, using: &str, value: &str) -> Result<Element<'_>> {
        let start = Instant::now();
        loop {
            match self.send("POST", &format!("/session/{}/element", self.session), Some(json!({"using": using, "value": value}))) {
                Ok(v) => {
                    let id =
                        v.as_object().and_then(|o| o.values().next()).and_then(Value::as_str).ok_or_else(|| format!("bad element {v}"))?;
                    return Ok(Element { app: self, id: id.to_string() });
                }
                Err(e) if start.elapsed() > WAIT => return Err(format!("{value} did not appear in {WAIT:?}: {e}")),
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    /// The first element matching a CSS selector, waiting up to [`WAIT`] for it.
    pub fn find(&self, css: &str) -> Result<Element<'_>> {
        self.find_by("css selector", css)
    }

    /// The first element matching an XPath, waiting up to [`WAIT`] for it.
    pub fn find_xpath(&self, xpath: &str) -> Result<Element<'_>> {
        self.find_by("xpath", xpath)
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if !self.session.is_empty() {
            let _ = self.send("DELETE", &format!("/session/{}", self.session), None);
        }
        // SIGTERM first: tauri-driver then stops the native driver it started (a SIGKILL
        // would leave WebKitWebDriver running).
        #[cfg(unix)]
        let _ = Command::new("kill").args(["-TERM", &self.driver.id().to_string()]).status();
        let start = Instant::now();
        while matches!(self.driver.try_wait(), Ok(None)) && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.driver.kill();
        let _ = self.driver.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Element<'_> {
    fn path(&self, what: &str) -> String {
        format!("/session/{}/element/{}/{what}", self.app.session, self.id)
    }

    /// Type into it.
    pub fn type_text(&self, text: &str) -> Result<()> {
        self.app.send("POST", &self.path("value"), Some(json!({"text": text}))).map(drop)
    }

    /// Click it.
    pub fn click(&self) -> Result<()> {
        self.app.send("POST", &self.path("click"), None).map(drop)
    }

    /// Its visible text.
    pub fn text(&self) -> Result<String> {
        Ok(self.app.send("GET", &self.path("text"), None)?.as_str().unwrap_or_default().to_string())
    }
}
