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
    if let Some(dir) = project {
        out.extend(project_checks(dir));
    }
    out
}

/// Checks on an app: its `kitchen.toml`, and the markers the generators need.
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
    let markers = [
        ("desktop-core/src/lib.rs", "kitchen:modules"),
        ("desktop-core/src/config.rs", "kitchen:config"),
        ("src-tauri/src/lib.rs", "kitchen:mods"),
        ("src-tauri/src/lib.rs", "kitchen:handlers"),
        ("src-tauri/src/service.rs", "kitchen:methods"),
    ];
    let missing: Vec<String> = markers
        .iter()
        .filter(|(f, m)| {
            std::fs::read_to_string(root.join(f))
                .map(|s| !s.contains(m))
                .unwrap_or(true)
        })
        .map(|(f, m)| format!("{f} ({m})"))
        .collect();
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
    out
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
    fn report_counts_problems() {
        let r = report(&[
            check("a", Status::Ok, "", ""),
            check("b", Status::Fail, "x", "do y"),
        ]);
        assert_eq!(r.summary, "1 problem(s) to fix before building.");
        assert_eq!(r.next, ["b: do y"]);
    }
}
