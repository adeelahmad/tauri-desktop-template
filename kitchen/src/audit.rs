//! `kitchen audit`: the app's supply-chain checks in one command, like `make audit`.
//!
//! - `cargo deny check` (licences, advisories, bans and sources, as set in `deny.toml`);
//! - `cargo audit` (RustSec advisories for `Cargo.lock`);
//! - `npm audit --omit=dev` for `ui-react/` when the app has it;
//! - `pip-audit -r python/requirements.txt` for the Python example when it has one.
//!
//! A tool that is not installed is skipped with the command that installs it. Every check
//! that can run does run, and the command fails at the end if any of them failed.
//!
//! ```
//! use kitchen::audit::{plan, Tools};
//!
//! let dir = std::env::temp_dir().join("kitchen-audit-doc");
//! std::fs::create_dir_all(&dir).unwrap();
//! let p = plan(&dir, Tools { cargo_deny: true, cargo_audit: false, npm: true, pip_audit: true });
//! assert_eq!(p.steps[0].command_line(), "cargo deny check");
//! assert!(p.notes.iter().any(|n| n.contains("cargo install --locked cargo-audit")));
//! ```

use crate::lifecycle::{execute, Io, Plan, Step};
use crate::{Error, Report, Result};
use std::path::Path;
use std::process::{Command, Stdio};

/// Which audit tools this machine has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tools {
    /// `cargo deny` runs.
    pub cargo_deny: bool,
    /// `cargo audit` runs.
    pub cargo_audit: bool,
    /// `npm` runs.
    pub npm: bool,
    /// `pip-audit` runs.
    pub pip_audit: bool,
}

impl Tools {
    /// Look for each tool on the PATH.
    pub fn detect() -> Tools {
        let runs = |program: &str, args: &[&str]| {
            Command::new(program)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        Tools {
            cargo_deny: runs("cargo", &["deny", "--version"]),
            cargo_audit: runs("cargo", &["audit", "--version"]),
            npm: runs("npm", &["--version"]),
            pip_audit: runs("pip-audit", &["--version"]),
        }
    }
}

/// The checks for the app in `root`, given the tools there are. Missing tools become notes
/// with the install command; parts the app does not have are left out.
pub fn plan(root: &Path, tools: Tools) -> Plan {
    let mut plan = Plan::default();
    let mut check = |have: bool, step: Step, hint: &str| {
        if have {
            plan.steps.push(step);
        } else {
            plan.notes
                .push(format!("skipped {}: install it with `{hint}`", step.label));
        }
    };
    check(
        tools.cargo_deny,
        Step::new("cargo-deny", root, "cargo", &["deny", "check"]),
        "cargo install --locked cargo-deny",
    );
    check(
        tools.cargo_audit,
        Step::new("cargo-audit", root, "cargo", &["audit"]),
        "cargo install --locked cargo-audit",
    );
    if root.join("ui-react/package.json").is_file() {
        check(
            tools.npm,
            Step::new(
                "npm audit",
                root,
                "npm",
                &["--prefix", "ui-react", "audit", "--omit=dev"],
            ),
            "Node.js from https://nodejs.org",
        );
    }
    if root.join("python/requirements.txt").is_file() {
        check(
            tools.pip_audit,
            Step::new(
                "pip-audit",
                root,
                "pip-audit",
                &["-r", "python/requirements.txt", "--progress-spinner", "off"],
            ),
            "pipx install pip-audit",
        );
    }
    plan
}

/// Run every check in `plan`, carrying on past failures; fail at the end if any failed.
pub fn run(plan: &Plan, io_mode: Io) -> Result<Report> {
    let mut report = Report::new("");
    for note in &plan.notes {
        if io_mode == Io::Inherit {
            eprintln!("kitchen: {note}");
        }
        report.lines.push(format!("note: {note}"));
    }
    let mut failed = Vec::new();
    for step in &plan.steps {
        let one = Plan {
            steps: vec![step.clone()],
            notes: vec![],
        };
        match execute(&one, io_mode, "") {
            Ok(r) => report.lines.extend(r.lines),
            Err(e) => {
                report.lines.push(format!("FAIL {e}"));
                failed.push(step.label.clone());
            }
        }
    }
    if !failed.is_empty() {
        let mut msg = format!("audit found problems in: {}", failed.join(", "));
        for line in &report.lines {
            msg.push('\n');
            msg.push_str(line);
        }
        return Err(Error::Tool(msg));
    }
    report.summary = if plan.steps.is_empty() {
        "No audit tool is installed; nothing was checked.".into()
    } else {
        format!("Audit passed ({} checks).", plan.steps.len())
    };
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_only_what_the_app_has() {
        let dir = crate::TempDir::new("audit").unwrap();
        let all = Tools {
            cargo_deny: true,
            cargo_audit: true,
            npm: true,
            pip_audit: true,
        };
        let labels = |p: &Plan| p.steps.iter().map(|s| s.label.clone()).collect::<Vec<_>>();
        assert_eq!(
            labels(&plan(dir.path(), all)),
            ["cargo-deny", "cargo-audit"]
        );

        std::fs::create_dir_all(dir.path().join("ui-react")).unwrap();
        std::fs::write(dir.path().join("ui-react/package.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("python")).unwrap();
        std::fs::write(dir.path().join("python/requirements.txt"), "").unwrap();
        let p = plan(dir.path(), all);
        assert_eq!(
            labels(&p),
            ["cargo-deny", "cargo-audit", "npm audit", "pip-audit"]
        );
        assert!(p.notes.is_empty());

        let p = plan(dir.path(), Tools::default());
        assert!(p.steps.is_empty());
        assert_eq!(p.notes.len(), 4);
        assert!(
            p.notes[3].contains("pipx install pip-audit"),
            "{:?}",
            p.notes
        );
    }

    #[test]
    fn keeps_going_and_fails_at_the_end() {
        let dir = crate::TempDir::new("audit-run").unwrap();
        let plan = Plan {
            steps: vec![
                Step::new("first", dir.path(), "cargo", &["--no-such-flag"]),
                Step::new("second", dir.path(), "cargo", &["--version"]),
            ],
            notes: vec!["skipped x".into()],
        };
        let e = run(&plan, Io::Capture).unwrap_err().to_string();
        assert!(e.contains("problems in: first"), "{e}");
        assert!(e.contains("ok   second"), "{e}");
        let ok = run(
            &Plan {
                steps: vec![plan.steps[1].clone()],
                notes: vec![],
            },
            Io::Capture,
        )
        .unwrap();
        assert!(ok.summary.contains("1 checks"), "{ok}");
    }
}
