//! `kitchen doctor`: what this machine (and this app) needs, and the command that fixes
//! each missing piece.
//!
//! ```no_run
//! for c in kitchen::doctor::run(None) {
//!     println!("{:?} {}: {}", c.status, c.name, c.detail);
//! }
//! ```

use crate::manifest::Manifest;
use crate::Report;
use std::path::Path;
use std::process::Command;

/// How a check went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Fine.
    Ok,
    /// Works, but something is off (an optional tool is missing, an old version).
    Warn,
    /// Builds will fail until it is fixed.
    Fail,
}

/// One check.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Check {
    /// What was checked.
    pub name: String,
    /// The result.
    pub status: Status,
    /// What was found.
    pub detail: String,
    /// A command or step that fixes it (empty when fine).
    pub fix: String,
}

fn check(name: &str, status: Status, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status,
        detail: detail.into(),
        fix: fix.into(),
    }
}

/// The first line a program prints for `args`, or `None` when it cannot run.
fn version(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    let text = if out.stdout.is_empty() {
        out.stderr
    } else {
        out.stdout
    };
    out.status.success().then(|| {
        String::from_utf8_lossy(&text)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

/// `(major, minor)` from the first `x.y` in a version line.
pub fn parse_version(line: &str) -> Option<(u32, u32)> {
    line.split(|c: char| !c.is_ascii_digit() && c != '.')
        .find_map(|w| {
            let mut it = w.split('.');
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
}

/// Run every check. With `project`, also check that app.
pub fn run(project: Option<&Path>) -> Vec<Check> {
    let mut out = Vec::new();
    let tool = |name: &str,
                program: &str,
                args: &[&str],
                min: Option<(u32, u32)>,
                required: bool,
                fix: &str| match version(program, args) {
        Some(v) => match (min, parse_version(&v)) {
            (Some(m), Some(have)) if have < m => check(
                name,
                Status::Fail,
                format!("{v} (needs {}.{}+)", m.0, m.1),
                fix,
            ),
            _ => check(name, Status::Ok, v, ""),
        },
        None => check(
            name,
            if required { Status::Fail } else { Status::Warn },
            "not found",
            fix,
        ),
    };
    out.push(tool(
        "Rust",
        "rustc",
        &["--version"],
        Some((1, 89)),
        true,
        "rustup update stable   (or install from https://rustup.rs)",
    ));
    out.push(tool(
        "cargo-generate",
        "cargo",
        &["generate", "--version"],
        None,
        true,
        "cargo install cargo-generate",
    ));
    out.push(tool(
        "Tauri CLI",
        "cargo",
        &["tauri", "--version"],
        Some((2, 0)),
        false,
        "cargo install tauri-cli --version '^2' --locked",
    ));
    out.push(tool(
        "git",
        "git",
        &["--version"],
        None,
        true,
        "install git (https://git-scm.com)",
    ));
    out.push(tool(
        "Node.js",
        "node",
        &["--version"],
        Some((20, 0)),
        false,
        "install Node.js 20+ (https://nodejs.org) for the React frontend",
    ));
    out.push(tool(
        "npm",
        "npm",
        &["--version"],
        None,
        false,
        "comes with Node.js",
    ));
    if cfg!(target_os = "linux") {
        out.push(match Command::new("pkg-config").args(["--exists", "webkit2gtk-4.1"]).status() {
            Ok(s) if s.success() => check("WebKitGTK 4.1", Status::Ok, "found", ""),
            _ => check(
                "WebKitGTK 4.1",
                Status::Fail,
                "missing (Tauri needs it on Linux)",
                "sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev",
            ),
        });
    }
    if cfg!(target_os = "macos") {
        out.push(match version("xcode-select", &["-p"]) {
            Some(p) => check("Xcode command line tools", Status::Ok, p, ""),
            None => check(
                "Xcode command line tools",
                Status::Fail,
                "missing",
                "xcode-select --install",
            ),
        });
    }
    out.extend(feature_test_checks());
    if let Some(dir) = project {
        out.extend(project_checks(dir));
    }
    out
}

/// Whether `program` starts at all (some drivers exit non-zero on `--version`).
fn starts(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// `kitchen test --feature` needs tauri-driver and the OS's WebDriver (optional: warnings).
fn feature_test_checks() -> Vec<Check> {
    if cfg!(target_os = "macos") {
        return vec![check(
            "Feature tests",
            Status::Ok,
            "macOS has no WKWebView WebDriver; `kitchen test --feature` runs --self-test",
            "",
        )];
    }
    let mut out = vec![if starts("tauri-driver", "--help") {
        check("tauri-driver", Status::Ok, "found", "")
    } else {
        check(
            "tauri-driver",
            Status::Warn,
            "not found (needed by `kitchen test --feature`)",
            "cargo install tauri-driver --locked",
        )
    }];
    if cfg!(target_os = "linux") {
        out.push(if starts("WebKitWebDriver", "--version") {
            check("WebKitWebDriver", Status::Ok, "found", "")
        } else {
            check(
                "WebKitWebDriver",
                Status::Warn,
                "not found (needed by `kitchen test --feature`)",
                "sudo apt install webkit2gtk-driver   (Fedora: webkit2gtk4.1-devel; Arch: webkit2gtk-4.1)",
            )
        });
    }
    if cfg!(target_os = "windows") {
        out.push(if starts("msedgedriver", "--version") {
            check("msedgedriver", Status::Ok, "found", "")
        } else {
            check(
                "msedgedriver",
                Status::Warn,
                "not found (needed by `kitchen test --feature`)",
                "cargo install msedgedriver-tool && msedgedriver-tool   (a driver matching your Edge, on PATH)",
            )
        });
    }
    out
}

/// Checks on an app: its `kitchen.toml`, the markers the generators need, and the agent
/// docs (`AGENTS.md` and friends).
fn project_checks(dir: &Path) -> Vec<Check> {
    let Some(root) = Manifest::find_root(dir) else {
        return vec![check(
            "Kitchen app",
            Status::Warn,
            "not inside one (no kitchen.toml)",
            "cd into an app, or `kitchen new <name>`",
        )];
    };
    let mut out = Vec::new();
    let mut tray_menu = false;
    match Manifest::load(&root) {
        Ok(m) => {
            let rev = if m.template.rev.is_empty() {
                "template commit unknown".to_string()
            } else {
                format!(
                    "template {}",
                    &m.template.rev[..m.template.rev.len().min(12)]
                )
            };
            tray_menu = m.is_on("tray_menu");
            out.push(check(
                "Kitchen app",
                Status::Ok,
                format!("{} ({rev})", root.display()),
                "",
            ));
            for (child, parent) in [
                ("bm25", "postgres"),
                ("local_llm", "ai"),
                ("background_agent", "ipc_worker"),
                ("daemon", "ipc_worker"),
            ] {
                if m.is_on(child) && !m.is_on(parent) {
                    out.push(check(
                        "Answers",
                        Status::Warn,
                        format!("{child} is on but {parent} is off, so it does nothing"),
                        format!("kitchen enable {parent}"),
                    ));
                }
            }
        }
        Err(e) => out.push(check(
            "Kitchen app",
            Status::Fail,
            e.to_string(),
            "fix kitchen.toml, or restore it from git",
        )),
    }
    let mut markers = vec![
        ("desktop-core/src/lib.rs", "kitchen:modules"),
        ("desktop-core/src/config.rs", "kitchen:config"),
        ("src-tauri/src/lib.rs", "kitchen:mods"),
        ("src-tauri/src/lib.rs", "kitchen:handlers"),
        ("src-tauri/src/service.rs", "kitchen:methods"),
        // Only in apps with background jobs.
        ("desktop-core/src/job_types/mod.rs", "kitchen:job-mods"),
        ("desktop-core/src/job_types/mod.rs", "kitchen:jobs"),
    ];
    if tray_menu {
        markers.extend([
            ("src-tauri/src/tray.rs", "kitchen:actions"),
            ("src-tauri/src/service.rs", "kitchen:action-methods"),
        ]);
    }
    let missing: Vec<String> = markers
        .iter()
        .filter(|(f, _)| !f.contains("job_types") || root.join(f).is_file())
        .filter(|(f, m)| {
            std::fs::read_to_string(root.join(f))
                .map(|s| !s.contains(m))
                .unwrap_or(true)
        })
        .map(|(f, m)| format!("{f} ({m})"))
        .collect();
    out.push(hooks_check(&root));
    out.push(if missing.is_empty() {
        check("Generator markers", Status::Ok, "all present", "")
    } else {
        check(
            "Generator markers",
            Status::Warn,
            format!("missing: {}", missing.join(", ")),
            "kitchen upgrade (or add the comment back by hand)",
        )
    });
    out.push(agent_files(&root));
    out
}

/// Whether the app's `.githooks` are turned on (`kitchen hooks install`).
fn hooks_check(root: &Path) -> Check {
    use crate::hooks::HookState;
    match crate::hooks::current(root) {
        HookState::On => check("Git hooks", Status::Ok, "on (.githooks)", ""),
        HookState::Off => check(
            "Git hooks",
            Status::Warn,
            "off: commits skip fmt and clippy",
            "kitchen hooks install",
        ),
        HookState::Elsewhere(p) => check(
            "Git hooks",
            Status::Warn,
            format!("git uses {p}, not .githooks"),
            "kitchen hooks install",
        ),
        HookState::Missing => check("Git hooks", Status::Ok, "none in this app", ""),
    }
}

/// The files that tell AI agents how to work in an app (`kitchen new` writes them).
pub const AGENT_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "TESTING.md",
    "BRAND.md",
    ".mcp.json",
    ".claude/skills",
];

/// Warn when the agent docs are missing; `AGENTS.md` is named first, since agents read it
/// before anything else.
fn agent_files(root: &Path) -> Check {
    let missing: Vec<&str> = AGENT_FILES
        .iter()
        .copied()
        .filter(|f| !root.join(f).exists())
        .collect();
    if missing.is_empty() {
        return check(
            "Agent docs",
            Status::Ok,
            "AGENTS.md and the rest present",
            "",
        );
    }
    let detail = if missing.contains(&"AGENTS.md") {
        format!(
            "AGENTS.md is missing, so agents get no instructions (missing: {})",
            missing.join(", ")
        )
    } else {
        format!("missing: {}", missing.join(", "))
    };
    check(
        "Agent docs",
        Status::Warn,
        detail,
        "kitchen upgrade adds them to an app made before they existed; otherwise copy them from the template",
    )
}

/// The checks as a report.
pub fn report(checks: &[Check]) -> Report {
    let fails = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn).count();
    let mut r = Report::new(match (fails, warns) {
        (0, 0) => "Everything is ready.".to_string(),
        (0, w) => format!("Ready, with {w} warning(s)."),
        (f, _) => format!("{f} problem(s) to fix before building."),
    });
    for c in checks {
        let mark = match c.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        r.lines.push(format!("{mark} {:<26} {}", c.name, c.detail));
        if !c.fix.is_empty() {
            r.next.push(format!("{}: {}", c.name, c.fix));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("rustc 1.89.0 (abc 2025-08-01)"),
            Some((1, 89))
        );
        assert_eq!(parse_version("v20.11.1"), Some((20, 11)));
        assert_eq!(parse_version("tauri-cli 2.1.0"), Some((2, 1)));
        assert_eq!(parse_version("none"), None);
    }

    #[test]
    fn warns_without_agents_md() {
        let dir = crate::TempDir::new("doctor").unwrap();
        let c = agent_files(dir.path());
        assert_eq!(c.status, Status::Warn);
        assert!(c.detail.starts_with("AGENTS.md is missing"), "{}", c.detail);
        for f in AGENT_FILES {
            let p = dir.path().join(f);
            if f.ends_with("skills") {
                std::fs::create_dir_all(&p).unwrap();
            } else {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, "x").unwrap();
            }
        }
        assert_eq!(agent_files(dir.path()).status, Status::Ok);
        std::fs::remove_file(dir.path().join("BRAND.md")).unwrap();
        let c = agent_files(dir.path());
        assert_eq!(
            (c.status, c.detail.as_str()),
            (Status::Warn, "missing: BRAND.md")
        );
    }

    #[test]
    fn report_counts_problems() {
        let r = report(&[
            check("a", Status::Ok, "", ""),
            check("b", Status::Fail, "x", "do y"),
        ]);
        assert_eq!(r.summary, "1 problem(s) to fix before building.");
        assert_eq!(r.next, ["b: do y"]);
    }
}
