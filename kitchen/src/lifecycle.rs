//! The app lifecycle: `kitchen start`, `test`, `package`, `publish` and `eject`.
//!
//! One path from a new app to a release, the way web frameworks have one:
//!
//! ```text
//! kitchen start                     cargo tauri dev, KITCHEN_ENV=development (Vite HMR + Rust rebuilds)
//! kitchen test [--unit|--feature]   cargo test (or nextest) + the real app over WebDriver, KITCHEN_ENV=test
//! kitchen package [linux] --release cargo tauri build --bundles ..., then collected into ./packages
//! kitchen publish [--yes]           a GitHub release with ./packages (dry run by default)
//! kitchen eject vite|tauri|cargo    where each tool's own config is, and what Kitchen adds
//! ```
//!
//! Kitchen hides none of the tools: every step is a plain command ([`Step`]) that is
//! printed before it runs, so you can run it yourself.
//!
//! Each command first builds a [`Plan`] (pure, and tested), then [`execute`]s it, either
//! on the terminal ([`Io::Inherit`]) or capturing the output for a report ([`Io::Capture`],
//! for `--json` and MCP).
//!
//! ```
//! use kitchen::lifecycle::{package_plan, Platform};
//! use std::path::Path;
//!
//! let plan = package_plan(Path::new("/app"), Some(Platform::Linux), Platform::Linux, true).unwrap();
//! assert_eq!(plan.steps[0].command_line(), "cargo tauri build --bundles appimage,deb");
//! assert!(package_plan(Path::new("/app"), Some(Platform::Windows), Platform::Linux, true).is_err());
//! ```

use crate::{io, Error, Report, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The desktop OS a package is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS: `.app` and `.dmg`.
    Mac,
    /// Windows: NSIS `.exe` and `.msi`.
    Windows,
    /// Linux: `.AppImage` and `.deb`.
    Linux,
}

impl Platform {
    /// The OS this binary runs on, if it is one of the three.
    pub fn host() -> Option<Platform> {
        match std::env::consts::OS {
            "macos" => Some(Platform::Mac),
            "windows" => Some(Platform::Windows),
            "linux" => Some(Platform::Linux),
            _ => None,
        }
    }

    /// `mac`, `mac os`, `macos`, `darwin`, `windows`, `win`, `linux`.
    pub fn parse(s: &str) -> Result<Platform> {
        match s.to_ascii_lowercase().as_str() {
            "mac" | "macos" | "darwin" | "osx" => Ok(Platform::Mac),
            "windows" | "win" => Ok(Platform::Windows),
            "linux" => Ok(Platform::Linux),
            other => Err(Error::Usage(format!(
                "unknown platform {other:?}: mac, windows or linux"
            ))),
        }
    }

    /// The name used in commands and `config/platforms/<name>.json`.
    pub fn name(self) -> &'static str {
        match self {
            Platform::Mac => "mac",
            Platform::Windows => "windows",
            Platform::Linux => "linux",
        }
    }

    /// The Tauri bundle formats built for it.
    pub fn bundles(self) -> &'static str {
        match self {
            Platform::Mac => "app,dmg",
            Platform::Windows => "nsis,msi",
            Platform::Linux => "appimage,deb",
        }
    }
}

/// One command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What it is for, e.g. `unit tests`.
    pub label: String,
    /// The program (`cargo`, `npm`, ...).
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// Where it runs.
    pub dir: PathBuf,
}

impl Step {
    /// A step running `program args...` in `dir`.
    pub fn new(label: &str, dir: &Path, program: &str, args: &[&str]) -> Step {
        Step {
            label: label.into(),
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
            dir: dir.to_path_buf(),
        }
    }

    /// The same step with `key=value` in its environment.
    pub fn with_env(mut self, key: &str, value: impl Into<String>) -> Step {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The command as you would type it (without the environment).
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(|w| {
                if w.contains(' ') {
                    format!("'{w}'")
                } else {
                    w.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The command with its environment, e.g. `KITCHEN_ENV=test cargo test --workspace`.
    pub fn display(&self) -> String {
        let env: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if env.is_empty() {
            self.command_line()
        } else {
            format!("{} {}", env.join(" "), self.command_line())
        }
    }
}

/// What a command will do: steps to run, and notes for the report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Run in order; the first failure stops the plan.
    pub steps: Vec<Step>,
    /// Said in the report (why something is skipped, what to do next).
    pub notes: Vec<String>,
}

/// Where a step's output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Io {
    /// To this terminal (the CLI).
    Inherit,
    /// Into the report (`--json`, MCP).
    Capture,
}

/// Whether `program args` runs and exits with success.
fn runs(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether `program` can be started at all (any exit code).
fn exists(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// The tools `kitchen start` / `test` / `package` need that are missing, as `(what, fix
/// step)`: the Tauri CLI, and with `deps` the UI's npm packages (a build installs those
/// itself with `npm ci`, so only `start` asks for them).
pub fn missing_tools(root: &Path, deps: bool) -> Vec<(String, Step)> {
    let mut out = Vec::new();
    if !crate::doctor::tauri_cli_ok() {
        out.push((
            format!("the Tauri {} CLI", crate::doctor::TAURI_CLI),
            Step::new(
                "install the Tauri CLI",
                root,
                "cargo",
                &crate::doctor::tauri_cli_install_args(),
            ),
        ));
    }
    let ui = root.join("ui-react");
    if deps && ui.join("package.json").is_file() && !ui.join("node_modules").is_dir() {
        out.push((
            "the UI's npm packages".to_string(),
            Step::new(
                "install the UI's npm packages",
                root,
                "npm",
                &["--prefix", "ui-react", "install", "--include=dev"],
            ),
        ));
    }
    out
}

/// Put a missing `npm install` of the UI's packages first, then refuse over a missing
/// machine-wide tool with the exact fix commands, or (with `install`) put those fixes first.
pub fn prerequisites(root: &Path, install: bool, deps: bool, plan: &mut Plan) -> Result<()> {
    let (local, missing): (Vec<_>, Vec<_>) = missing_tools(root, deps)
        .into_iter()
        .partition(|(_, step)| step.program == "npm");
    // The UI's own packages live in the app (like the crates cargo fetches), so they are
    // installed without asking; only machine-wide tools wait for `--install`.
    if !local.is_empty() {
        plan.notes
            .push("First run: installing the UI's npm packages into ui-react/node_modules.".into());
        plan.steps
            .splice(0..0, local.into_iter().map(|(_, step)| step));
    }
    if missing.is_empty() {
        return Ok(());
    }
    if install {
        plan.steps
            .splice(0..0, missing.into_iter().map(|(_, step)| step));
        return Ok(());
    }
    let mut msg = String::from("missing ");
    msg.push_str(
        &missing
            .iter()
            .map(|(w, _)| w.as_str())
            .collect::<Vec<_>>()
            .join(" and "),
    );
    msg.push_str(". Run:\n");
    for (_, step) in &missing {
        msg.push_str(&format!("  {}\n", step.command_line()));
    }
    msg.push_str("or add --install to let Kitchen run them.");
    Err(Error::Tool(msg))
}

/// `kitchen start`: `cargo tauri dev` with `KITCHEN_ENV=development`, plus `extra` args.
pub fn start_plan(root: &Path, extra: &[String], install: bool) -> Result<Plan> {
    let mut step = Step::new(
        "start the app (hot reload)",
        root,
        "cargo",
        &["tauri", "dev"],
    )
    .with_env("KITCHEN_ENV", "development");
    step.args.extend(extra.iter().cloned());
    let mut plan = Plan {
        steps: vec![step],
        notes: vec![
            "UI edits reload in place (Vite HMR); Rust edits rebuild and restart the app.".into(),
        ],
    };
    prerequisites(root, install, true, &mut plan)?;
    Ok(plan)
}

/// Which tests `kitchen test` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestKinds {
    /// `cargo test --workspace` (or nextest) and the UI's `npm test`, if it has one.
    pub unit: bool,
    /// The real app, driven through WebDriver (`e2e/`), or `--self-test` on macOS.
    pub feature: bool,
}

/// The app's binary name: `[package] name` in `src-tauri/Cargo.toml`.
pub fn binary_name(root: &Path) -> Result<String> {
    let path = root.join("src-tauri/Cargo.toml");
    let text = io(
        format!("read {}", path.display()),
        std::fs::read_to_string(&path),
    )?;
    let v: toml::Value =
        toml::from_str(&text).map_err(|e| Error::Usage(format!("{}: {e}", path.display())))?;
    v.get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(String::from)
        .ok_or_else(|| Error::Usage(format!("{}: no [package] name", path.display())))
}

/// The cargo target folder: `CARGO_TARGET_DIR`, else `<app>/target`.
pub fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_relative() { root.join(p) } else { p })
        .unwrap_or_else(|| root.join("target"))
}

/// What the machine has for feature tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drivers {
    /// `tauri-driver` is on PATH.
    pub tauri_driver: bool,
    /// The OS's WebDriver: WebKitWebDriver (Linux) or msedgedriver (Windows).
    pub native: bool,
    /// A display (or `xvfb-run` to make one) on Linux; always true elsewhere.
    pub display: bool,
    /// `xvfb-run` is needed and present.
    pub xvfb: bool,
    /// Node is installed: the UI's accessibility check (`npm run a11y`) can run.
    pub node: bool,
}

impl Drivers {
    /// Look at this machine.
    pub fn detect() -> Drivers {
        let linux = cfg!(target_os = "linux");
        let has_display =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        let xvfb = linux && !has_display && exists("xvfb-run", "--help");
        Drivers {
            tauri_driver: exists("tauri-driver", "--help"),
            native: if linux {
                exists("WebKitWebDriver", "--version")
            } else {
                exists("msedgedriver", "--version")
            },
            display: !linux || has_display || xvfb,
            xvfb,
            node: runs("node", &["--version"]),
        }
    }
}

/// The steps that install the feature-test drivers on `host`.
pub fn driver_fixes(host: Platform) -> Vec<String> {
    let mut v = vec!["cargo install tauri-driver --locked".to_string()];
    match host {
        Platform::Linux => {
            v.push("sudo apt install webkit2gtk-driver xvfb   (Fedora: webkit2gtk4.1-devel; Arch: webkit2gtk-4.1)".into())
        }
        Platform::Windows => v.push(
            "install msedgedriver matching your Edge version (cargo install msedgedriver-tool && msedgedriver-tool) and put it on PATH".into(),
        ),
        Platform::Mac => {}
    }
    v
}

/// `kitchen test`: the steps for `kinds` on `host`, given what the machine has.
pub fn test_plan(
    root: &Path,
    kinds: TestKinds,
    host: Platform,
    nextest: bool,
    drivers: Drivers,
) -> Result<Plan> {
    let mut plan = Plan::default();
    if kinds.unit {
        if nextest {
            plan.steps.push(
                Step::new(
                    "unit tests (nextest)",
                    root,
                    "cargo",
                    &["nextest", "run", "--workspace"],
                )
                .with_env("KITCHEN_ENV", "test"),
            );
            plan.steps.push(
                Step::new(
                    "doc tests",
                    root,
                    "cargo",
                    &["test", "--workspace", "--doc"],
                )
                .with_env("KITCHEN_ENV", "test"),
            );
        } else {
            plan.steps.push(
                Step::new("unit tests", root, "cargo", &["test", "--workspace"])
                    .with_env("KITCHEN_ENV", "test"),
            );
        }
        if has_npm_test(root) {
            plan.steps.push(
                Step::new("UI tests", root, "npm", &["--prefix", "ui-react", "test"])
                    .with_env("KITCHEN_ENV", "test"),
            );
        }
        // axe-core on every screen and Settings section, plus the pseudo-locale layout
        // check (ui-react/scripts/a11y.mjs); it skips itself when Playwright is missing.
        if has_npm_script(root, "a11y") {
            if drivers.node {
                plan.steps.push(
                    Step::new(
                        "accessibility (axe-core)",
                        root,
                        "npm",
                        &["--prefix", "ui-react", "run", "a11y"],
                    )
                    .with_env("KITCHEN_ENV", "test"),
                );
            } else {
                plan.notes
                    .push("accessibility check skipped: Node is not installed".into());
            }
        }
    }
    if kinds.feature {
        feature_steps(root, host, drivers, &mut plan)?;
    }
    Ok(plan)
}

fn has_npm_test(root: &Path) -> bool {
    has_npm_script(root, "test")
}

fn has_npm_script(root: &Path, name: &str) -> bool {
    std::fs::read_to_string(root.join("ui-react/package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["scripts"][name].is_string())
}

/// `kitchen generate command` (or by hand): rewrite the UI's typed IPC bindings from the
/// Rust commands, `cargo test -p <app> --lib export_bindings` with `KITCHEN_BINDINGS=write`
/// (see the app's docs/GUIDE.md, "Typed IPC").
pub fn bindings_plan(root: &Path) -> Result<Plan> {
    let name = binary_name(root)?;
    let mut plan = Plan::default();
    plan.steps.push(
        Step::new(
            "typed IPC bindings",
            root,
            "cargo",
            &["test", "-p", &name, "--lib", "export_bindings"],
        )
        .with_env("KITCHEN_BINDINGS", "write"),
    );
    Ok(plan)
}

fn feature_steps(root: &Path, host: Platform, drivers: Drivers, plan: &mut Plan) -> Result<()> {
    let name = binary_name(root)?;
    let binary = target_dir(root)
        .join("debug")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let build = Step::new(
        "build the app with its UI (debug)",
        root,
        "cargo",
        &["tauri", "build", "--debug", "--no-bundle"],
    )
    .with_env("KITCHEN_ENV", "test");
    if host == Platform::Mac {
        plan.notes.push(
            "macOS has no WebDriver for WKWebView, so feature tests cannot drive the window here; running the app's --self-test instead (it indexes and searches a sample through the real backend). Run the WebDriver tests on Linux or Windows (CI).".into(),
        );
        plan.steps.push(build);
        let mut st = Step::new(
            "self-test of the built app",
            root,
            &binary.display().to_string(),
            &["--self-test"],
        );
        st.env.push(("KITCHEN_ENV".into(), "test".into()));
        plan.steps.push(st);
        return Ok(());
    }
    if !root.join("e2e/Cargo.toml").is_file() {
        return Err(Error::Usage(
            "this app has no e2e/ crate; `kitchen upgrade` adds it".into(),
        ));
    }
    let mut missing = Vec::new();
    if !drivers.tauri_driver {
        missing.push("tauri-driver");
    }
    if !drivers.native {
        missing.push(if host == Platform::Linux {
            "WebKitWebDriver"
        } else {
            "msedgedriver"
        });
    }
    if !drivers.display {
        missing.push("a display (or xvfb-run)");
    }
    if !missing.is_empty() {
        return Err(Error::Tool(format!(
            "feature tests need {}. Install with:\n  {}\nSee e2e/src/lib.rs for the manual steps.",
            missing.join(", "),
            driver_fixes(host).join("\n  ")
        )));
    }
    plan.steps.push(build);
    let mut args = vec!["test", "-p", "e2e", "--", "--ignored"];
    let program = if drivers.xvfb {
        args.insert(0, "cargo");
        args.insert(0, "-a");
        "xvfb-run"
    } else {
        "cargo"
    };
    plan.steps.push(
        Step::new("feature tests (tauri-driver)", root, program, &args)
            .with_env("KITCHEN_ENV", "test")
            .with_env("APP_BINARY", binary.display().to_string()),
    );
    Ok(())
}

/// `kitchen package`: build the installers for `platform` (default: this OS). Building for
/// another OS is refused: Tauri bundles for the OS it runs on.
pub fn package_plan(
    root: &Path,
    platform: Option<Platform>,
    host: Platform,
    release: bool,
) -> Result<Plan> {
    let p = platform.unwrap_or(host);
    if p != host {
        return Err(Error::Usage(format!(
            "cannot package for {} on {}: Tauri builds installers for the OS it runs on (WebView and signing tools are native). Run `kitchen package {}` on a {} machine, or build it in CI (the release workflow, when the app has installers on).",
            p.name(),
            host.name(),
            p.name(),
            p.name()
        )));
    }
    let mut args = vec!["tauri", "build", "--bundles", p.bundles()];
    if !release {
        args.push("--debug");
    }
    let env = if release { "production" } else { "development" };
    Ok(Plan {
        steps: vec![Step::new(
            &format!("build {} packages", p.name()),
            root,
            "cargo",
            &args,
        )
        .with_env("KITCHEN_ENV", env)],
        notes: vec![],
    })
}

/// What `kitchen package` says about an app on the Chromium engine (`web_engine =
/// "chromium"`): on macOS the Tauri 3 CLI bundles and signs CEF itself; elsewhere the app
/// builds with the system web view.
///
/// ```
/// use kitchen::lifecycle::{chromium_notes, Platform};
///
/// assert!(chromium_notes(Platform::Mac).iter().any(|n| n.contains("Helper (Renderer)")));
/// assert!(chromium_notes(Platform::Windows)[0].contains("WebView2"));
/// ```
pub fn chromium_notes(platform: Platform) -> Vec<String> {
    match platform {
        Platform::Mac => vec![
            format!(
                "Chromium engine: cargo tauri build ({}) puts the CEF framework and five helper apps (… Helper, Helper (GPU), Helper (Renderer), Helper (Plugin), Helper (Alerts)) in Contents/Frameworks and signs them before the app; expect a .app of about 340 MB and a .dmg of about 150 MB",
                crate::doctor::TAURI_CLI
            ),
            "Chromium engine: the first build downloads CEF (about 130 MB) into CEF_PATH (default ~/Library/Caches/tauri-cef) and builds its wrapper with cmake and ninja (`kitchen doctor` checks them)".into(),
            "Chromium engine: signing and notarizing use the same APPLE_* variables as before; the CLI adds the entitlements CEF needs (allow-jit, allow-unsigned-executable-memory, disable-library-validation) to src-tauri/Entitlements.plist's, for the app and every helper".into(),
        ],
        Platform::Windows => vec!["web_engine = chromium changes macOS only: Windows builds with WebView2 (Chromium already)".into()],
        Platform::Linux => vec!["web_engine = chromium changes macOS only: Linux builds with WebKitGTK".into()],
    }
}

/// Skip the signed updater archives (`.tar.gz` + `.sig`) when no signing key is set.
///
/// With the `updater` feature, `tauri.conf.json` asks for updater artifacts, and Tauri then
/// stops the build if `TAURI_SIGNING_PRIVATE_KEY` is missing. Local and CI test builds rarely
/// have the key, so `kitchen package` turns the artifacts off for that build and says so. The
/// release workflow does the same, and signs when the key is a CI secret.
///
/// ```
/// use kitchen::lifecycle::{package_plan, skip_updater_artifacts, Platform};
/// use std::path::Path;
///
/// let mut plan = package_plan(Path::new("/app"), None, Platform::Mac, false).unwrap();
/// skip_updater_artifacts(&mut plan, false);
/// assert!(plan.steps[0].args.iter().any(|a| a.contains("createUpdaterArtifacts")));
/// assert_eq!(plan.notes.len(), 1);
/// ```
pub fn skip_updater_artifacts(plan: &mut Plan, has_signing_key: bool) {
    if has_signing_key {
        return;
    }
    for step in plan
        .steps
        .iter_mut()
        .filter(|s| s.args.iter().any(|a| a == "build"))
    {
        step.args.push("--config".into());
        step.args
            .push(r#"{"bundle":{"createUpdaterArtifacts":false}}"#.into());
    }
    plan.notes.push(
        "no TAURI_SIGNING_PRIVATE_KEY: built without the signed updater archives (set the key to include them)"
            .into(),
    );
}

/// Whether a file in `bundle/` is an installer worth collecting.
pub fn is_package(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    [".dmg", ".deb", ".rpm", ".appimage", ".msi", ".exe", ".app"]
        .iter()
        .any(|ext| name.ends_with(ext))
        && !name.ends_with(".sig")
}

/// Copy the installers from `<target>/<debug|release>/bundle` into `<app>/packages`.
///
/// With `since`, only installers written at or after that time are copied: a target
/// folder shared by several apps (`CARGO_TARGET_DIR`) also holds the other apps' bundles.
pub fn collect_packages(
    root: &Path,
    release: bool,
    since: Option<std::time::SystemTime>,
) -> Result<Vec<PathBuf>> {
    let bundle = target_dir(root)
        .join(if release { "release" } else { "debug" })
        .join("bundle");
    collect_into(&bundle, &root.join("packages"), since)
}

/// Whether `path` was written at or after `since` (a little early counts too, for
/// file systems with coarse timestamps).
fn fresh(path: &Path, since: Option<std::time::SystemTime>) -> bool {
    let Some(since) = since else { return true };
    let since = since
        .checked_sub(std::time::Duration::from_secs(2))
        .unwrap_or(since);
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t >= since)
}

/// Copy the installers in `bundle/<kind>/` into `out` (only those written since `since`).
pub fn collect_into(
    bundle: &Path,
    out: &Path,
    since: Option<std::time::SystemTime>,
) -> Result<Vec<PathBuf>> {
    io(
        format!("create {}", out.display()),
        std::fs::create_dir_all(out),
    )?;
    let mut copied = Vec::new();
    let Ok(kinds) = std::fs::read_dir(bundle) else {
        return Ok(copied);
    };
    for kind in kinds.flatten() {
        let Ok(files) = std::fs::read_dir(kind.path()) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            if !is_package(&path) || !fresh(&path, since) {
                continue;
            }
            let dest = out.join(f.file_name());
            copy_all(&path, &dest)?;
            copied.push(dest);
        }
    }
    copied.sort();
    Ok(copied)
}

fn copy_all(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        io(
            format!("create {}", to.display()),
            std::fs::create_dir_all(to),
        )?;
        for e in io(format!("read {}", from.display()), std::fs::read_dir(from))?.flatten() {
            copy_all(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        io(
            format!("copy {} to {}", from.display(), to.display()),
            std::fs::copy(from, to).map(drop),
        )
    }
}

/// The app's version, from `src-tauri/tauri.conf.json`.
pub fn app_version(root: &Path) -> Result<String> {
    let path = root.join("src-tauri/tauri.conf.json");
    let text = io(
        format!("read {}", path.display()),
        std::fs::read_to_string(&path),
    )?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Usage(format!("{}: {e}", path.display())))?;
    v["version"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::Usage(format!("{}: no version", path.display())))
}

/// `kitchen publish`: a GitHub release `v<version>` (or `tag`) with every file in
/// `./packages`, through `gh`. `repo` is `owner/name` (from kitchen.toml when known).
pub fn publish_plan(
    root: &Path,
    tag: Option<&str>,
    repo: Option<&str>,
    title: &str,
) -> Result<Plan> {
    let dir = root.join("packages");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .map(|p| {
                    format!(
                        "packages/{}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        return Err(Error::Usage(
            "./packages is empty: run `kitchen package --release` first".into(),
        ));
    }
    let tag = match tag {
        Some(t) => t.to_string(),
        None => format!("v{}", app_version(root)?),
    };
    let mut args: Vec<String> = vec!["release".into(), "create".into(), tag.clone()];
    args.extend(files);
    args.extend([
        "--title".into(),
        format!("{title} {tag}"),
        "--generate-notes".into(),
    ]);
    if let Some(r) = repo {
        args.extend(["--repo".into(), r.to_string()]);
    }
    let mut step = Step::new("create the GitHub release", root, "gh", &[]);
    step.args = args;
    Ok(Plan {
        steps: vec![step],
        notes: vec![],
    })
}

/// Run a plan. With [`Io::Inherit`] each step prints its command, then its own output;
/// with [`Io::Capture`] the report holds the commands and the tail of the output of a
/// step that failed. The first failure stops the plan and is an [`Error::Tool`].
pub fn execute(plan: &Plan, io_mode: Io, done: &str) -> Result<Report> {
    let mut report = Report::new(done);
    for note in &plan.notes {
        if io_mode == Io::Inherit {
            eprintln!("kitchen: {note}");
        }
        report.lines.push(format!("note: {note}"));
    }
    for step in &plan.steps {
        if io_mode == Io::Inherit {
            eprintln!("kitchen: {}: {}", step.label, step.display());
        }
        let mut cmd = Command::new(&step.program);
        cmd.args(&step.args).current_dir(&step.dir);
        for (k, v) in &step.env {
            cmd.env(k, v);
        }
        let (ok, tail) = match io_mode {
            Io::Inherit => {
                let status = cmd
                    .status()
                    .map_err(|e| Error::Tool(format!("{}: {e}", step.program)))?;
                (status.success(), String::new())
            }
            Io::Capture => {
                let out = cmd
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| Error::Tool(format!("{}: {e}", step.program)))?;
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                (out.status.success(), tail_lines(&text, 60))
            }
        };
        if !ok {
            let mut msg = format!("{} failed: {}", step.label, step.display());
            if !tail.is_empty() {
                msg.push_str("\n--- last lines ---\n");
                msg.push_str(&tail);
            }
            return Err(Error::Tool(msg));
        }
        let mut line = format!("ok   {}: {}", step.label, step.display());
        if io_mode == Io::Capture && step.label.contains("test") {
            let results: Vec<&str> = tail
                .lines()
                .filter(|l| l.starts_with("test result:") || l.contains("Summary"))
                .filter(|l| !l.starts_with("test result: ok. 0 passed; 0 failed; 0 ignored"))
                .collect();
            if !results.is_empty() {
                line.push_str(&format!(" ({})", results.join("; ")));
            }
        }
        report.lines.push(line);
    }
    Ok(report)
}

fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// `kitchen eject <tool>`: where the tool's own config is, and what Kitchen sets on top.
/// Nothing is hidden, so nothing is moved: edit those files directly.
pub fn eject(root: &Path, tool: &str) -> Result<Report> {
    let (summary, files, kitchen): (&str, &[&str], &[&str]) = match tool {
        "vite" => (
            "Vite builds the React UI. Edit its config directly.",
            &["ui-react/vite.config.js", "ui-react/package.json", "ui-react/eslint.config.js"],
            &[
                "nothing: `kitchen start` runs `cargo tauri dev`, which runs the beforeDevCommand in src-tauri/tauri.conf.json (npm run dev, port 5173)",
                "`kitchen package` runs the beforeBuildCommand (npm ci && npm run build into ui-react/dist)",
                "TAURI_DEV_HOST is set by `cargo tauri android|ios dev` only",
            ],
        ),
        "tauri" => (
            "Tauri is the window, the bundler and the updater. Edit its config directly.",
            &[
                "src-tauri/tauri.conf.json",
                "src-tauri/tauri.release.conf.json (if present)",
                "src-tauri/capabilities/*.json (what the UI may call)",
                "src-tauri/build.rs",
            ],
            &[
                "KITCHEN_ENV: development (start), test (test), production (package --release); build.rs merges config/ for it",
                "`kitchen package` adds --bundles (mac: app,dmg; windows: nsis,msi; linux: appimage,deb) and --debug unless --release",
                "`kitchen test --feature` runs `cargo tauri build --debug --no-bundle`",
            ],
        ),
        "cargo" => (
            "Cargo builds the Rust workspace. Edit its config directly.",
            &[
                "Cargo.toml (workspace, lints, release profile)",
                "src-tauri/Cargo.toml (app features)",
                "desktop-core/Cargo.toml (engine features)",
                ".cargo/config.toml (build-time env: APP_REPOSITORY, APP_IDENTIFIER, ...)",
            ],
            &[
                "`kitchen test` runs `cargo nextest run --workspace` (+ doc tests) when nextest is installed, else `cargo test --workspace`",
                "`kitchen enable|disable` change the features in the Cargo.toml files with a three-way merge",
            ],
        ),
        other => {
            return Err(Error::Usage(format!(
                "eject what? {other:?}: vite, tauri or cargo"
            )))
        }
    };
    let mut r = Report::new(summary);
    for f in files {
        let path = f.split(' ').next().unwrap_or(f);
        let mark = if path.contains('*') || root.join(path).exists() {
            "    "
        } else {
            "(no)"
        };
        r.lines.push(format!("{mark} {f}"));
    }
    r.next = kitchen
        .iter()
        .map(|k| format!("Kitchen adds: {k}"))
        .collect();
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Drivers = Drivers {
        tauri_driver: true,
        native: true,
        display: true,
        xvfb: false,
        node: true,
    };

    fn app() -> crate::TempDir {
        let d = crate::TempDir::new("lifecycle").unwrap();
        std::fs::create_dir_all(d.path().join("src-tauri")).unwrap();
        std::fs::create_dir_all(d.path().join("e2e")).unwrap();
        std::fs::write(
            d.path().join("src-tauri/Cargo.toml"),
            "[package]\nname = \"notes\"\n",
        )
        .unwrap();
        std::fs::write(d.path().join("e2e/Cargo.toml"), "").unwrap();
        std::fs::write(
            d.path().join("src-tauri/tauri.conf.json"),
            r#"{"version": "1.2.3"}"#,
        )
        .unwrap();
        d
    }

    #[test]
    fn platforms() {
        assert_eq!(Platform::parse("macOS").unwrap(), Platform::Mac);
        assert_eq!(Platform::parse("win").unwrap(), Platform::Windows);
        assert!(Platform::parse("beos").is_err());
        assert_eq!(Platform::Windows.bundles(), "nsis,msi");
    }

    #[test]
    fn package_refuses_other_os() {
        let root = Path::new("/a");
        let e = package_plan(root, Some(Platform::Mac), Platform::Linux, true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("cannot package for mac on linux"), "{e}");
        let p = package_plan(root, None, Platform::Windows, false).unwrap();
        assert_eq!(
            p.steps[0].display(),
            "KITCHEN_ENV=development cargo tauri build --bundles nsis,msi --debug"
        );
        let mut q = p.clone();
        skip_updater_artifacts(&mut q, true);
        assert_eq!(q.steps, p.steps);
        skip_updater_artifacts(&mut q, false);
        assert!(
            q.steps[0]
                .display()
                .ends_with(r#"--config {"bundle":{"createUpdaterArtifacts":false}}"#),
            "{}",
            q.steps[0].display()
        );
    }

    #[test]
    fn unit_tests_prefer_nextest() {
        let d = app();
        let kinds = TestKinds {
            unit: true,
            feature: false,
        };
        let p = test_plan(d.path(), kinds, Platform::Linux, false, ALL).unwrap();
        assert_eq!(p.steps.len(), 1);
        assert_eq!(
            p.steps[0].display(),
            "KITCHEN_ENV=test cargo test --workspace"
        );
        let p = test_plan(d.path(), kinds, Platform::Linux, true, ALL).unwrap();
        assert_eq!(p.steps[0].command_line(), "cargo nextest run --workspace");
        assert_eq!(p.steps[1].command_line(), "cargo test --workspace --doc");
        std::fs::create_dir_all(d.path().join("ui-react")).unwrap();
        std::fs::write(
            d.path().join("ui-react/package.json"),
            r#"{"scripts": {"test": "vitest run"}}"#,
        )
        .unwrap();
        let p = test_plan(d.path(), kinds, Platform::Linux, false, ALL).unwrap();
        assert_eq!(p.steps[1].command_line(), "npm --prefix ui-react test");
        std::fs::write(
            d.path().join("ui-react/package.json"),
            r#"{"scripts": {"a11y": "node scripts/a11y.mjs"}}"#,
        )
        .unwrap();
        let p = test_plan(d.path(), kinds, Platform::Linux, false, ALL).unwrap();
        assert_eq!(p.steps[1].command_line(), "npm --prefix ui-react run a11y");
        let no_node = Drivers { node: false, ..ALL };
        let p = test_plan(d.path(), kinds, Platform::Linux, false, no_node).unwrap();
        assert_eq!(p.steps.len(), 1);
        assert!(p.notes[0].contains("Node is not installed"));
    }

    #[test]
    fn bindings_run_the_export_test() {
        let d = app();
        let p = bindings_plan(d.path()).unwrap();
        assert_eq!(
            p.steps[0].display(),
            "KITCHEN_BINDINGS=write cargo test -p notes --lib export_bindings"
        );
    }

    #[test]
    fn feature_tests_by_platform() {
        let d = app();
        let kinds = TestKinds {
            unit: false,
            feature: true,
        };
        let p = test_plan(d.path(), kinds, Platform::Linux, false, ALL).unwrap();
        assert_eq!(
            p.steps[0].command_line(),
            "cargo tauri build --debug --no-bundle"
        );
        assert_eq!(p.steps[1].command_line(), "cargo test -p e2e -- --ignored");
        assert!(p.steps[1]
            .env
            .iter()
            .any(|(k, v)| k == "APP_BINARY" && v.contains("notes")));
        let xvfb = Drivers { xvfb: true, ..ALL };
        let p = test_plan(d.path(), kinds, Platform::Linux, false, xvfb).unwrap();
        assert!(p.steps[1]
            .command_line()
            .starts_with("xvfb-run -a cargo test"));
        let none = Drivers {
            tauri_driver: false,
            native: false,
            ..ALL
        };
        let e = test_plan(d.path(), kinds, Platform::Linux, false, none)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("tauri-driver, WebKitWebDriver") && e.contains("cargo install tauri-driver"),
            "{e}"
        );
        let p = test_plan(d.path(), kinds, Platform::Mac, false, none).unwrap();
        assert!(p.steps[1].command_line().ends_with(" --self-test"));
        assert!(p.notes[0].contains("WKWebView"));
    }

    #[test]
    fn publish_needs_packages() {
        let d = app();
        assert!(publish_plan(d.path(), None, None, "Notes")
            .unwrap_err()
            .to_string()
            .contains("kitchen package"));
        std::fs::create_dir_all(d.path().join("packages")).unwrap();
        std::fs::write(d.path().join("packages/notes_1.2.3_amd64.deb"), "").unwrap();
        let p = publish_plan(d.path(), None, Some("me/notes"), "Notes").unwrap();
        assert_eq!(
            p.steps[0].command_line(),
            "gh release create v1.2.3 packages/notes_1.2.3_amd64.deb --title 'Notes v1.2.3' --generate-notes --repo me/notes"
        );
    }

    #[test]
    fn collects_installers() {
        let d = app();
        let bundle = d.path().join("target/release/bundle");
        std::fs::create_dir_all(bundle.join("deb")).unwrap();
        std::fs::create_dir_all(bundle.join("macos/Notes.app/Contents")).unwrap();
        std::fs::write(bundle.join("deb/notes.deb"), "x").unwrap();
        std::fs::write(bundle.join("deb/control.tar"), "x").unwrap();
        std::fs::write(bundle.join("macos/Notes.app/Contents/Info.plist"), "x").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let stale = collect_into(&bundle, &d.path().join("old"), Some(later)).unwrap();
        assert!(
            stale.is_empty(),
            "bundles from before the build are left out: {stale:?}"
        );
        let got = collect_into(&bundle, &d.path().join("packages"), None).unwrap();
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Notes.app", "notes.deb"]);
        assert!(d
            .path()
            .join("packages/Notes.app/Contents/Info.plist")
            .exists());
    }

    #[test]
    fn eject_lists_the_files() {
        let d = app();
        let r = eject(d.path(), "tauri").unwrap();
        assert!(r.lines[0].contains("src-tauri/tauri.conf.json"));
        assert!(r.next.iter().any(|n| n.contains("KITCHEN_ENV")));
        assert!(eject(d.path(), "webpack").is_err());
    }

    #[test]
    fn start_needs_the_tauri_cli_or_says_how() {
        let d = app();
        match start_plan(d.path(), &["--release".into()], false) {
            Ok(p) => assert_eq!(
                p.steps[0].display(),
                "KITCHEN_ENV=development cargo tauri dev --release"
            ),
            Err(e) => assert!(e.to_string().contains("cargo install tauri-cli"), "{e}"),
        }
        let p = start_plan(d.path(), &[], true).unwrap();
        assert_eq!(p.steps.last().unwrap().command_line(), "cargo tauri dev");
    }

    #[test]
    fn start_installs_the_ui_packages_without_asking() {
        let d = app();
        std::fs::create_dir_all(d.path().join("ui-react")).unwrap();
        std::fs::write(d.path().join("ui-react/package.json"), "{}").unwrap();
        // With --install, a missing Tauri CLI is installed too, so the plan always builds.
        let p = start_plan(d.path(), &[], true).unwrap();
        assert!(
            p.steps
                .iter()
                .any(|s| s.command_line() == "npm --prefix ui-react install --include=dev"),
            "{:?}",
            p.steps
        );
        assert!(p.notes.iter().any(|n| n.starts_with("First run")));
        std::fs::create_dir_all(d.path().join("ui-react/node_modules")).unwrap();
        let p = start_plan(d.path(), &[], true).unwrap();
        assert!(!p.steps.iter().any(|s| s.program == "npm"));
    }
}
