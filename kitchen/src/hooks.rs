//! `kitchen hooks`: the app's own git hooks in `.githooks/`, turned on with
//! `git config core.hooksPath .githooks` (no Python, no hook manager).
//!
//! The template ships `.githooks/pre-commit`: `cargo fmt --check` and `cargo clippy -D
//! warnings` when Rust files are staged, and prettier + eslint on staged `ui-react/` files
//! when the app has the React frontend. `kitchen doctor` warns when the hooks exist but are
//! not turned on.
//!
//! ```text
//! kitchen hooks install     # git config core.hooksPath .githooks (and make them executable)
//! kitchen hooks status      # on or off
//! kitchen hooks uninstall   # git config --unset core.hooksPath
//! ```
//!
//! The state is plain data, so it is easy to check without git:
//!
//! ```
//! use kitchen::hooks::{state, HookState, HOOKS_DIR};
//!
//! assert_eq!(state(true, Some(HOOKS_DIR)), HookState::On);
//! assert_eq!(state(true, Some("./.githooks/")), HookState::On);
//! assert_eq!(state(true, None), HookState::Off);
//! assert_eq!(state(true, Some(".husky")), HookState::Elsewhere(".husky".into()));
//! assert_eq!(state(false, None), HookState::Missing);
//! ```

use crate::{io, Error, Report, Result};
use std::path::Path;
use std::process::Command;

/// The folder the hooks live in, relative to the app's root.
pub const HOOKS_DIR: &str = ".githooks";

/// Whether the app's hooks are turned on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookState {
    /// `core.hooksPath` points at `.githooks`.
    On,
    /// `.githooks` exists but git does not use it.
    Off,
    /// git uses another hooks folder (named).
    Elsewhere(String),
    /// The app has no `.githooks` folder.
    Missing,
}

/// The state from what is on disk (`has_dir`) and git's `core.hooksPath` value.
pub fn state(has_dir: bool, hooks_path: Option<&str>) -> HookState {
    if !has_dir {
        return HookState::Missing;
    }
    match hooks_path.map(|p| {
        p.trim()
            .trim_start_matches("./")
            .trim_end_matches(['/', '\\'])
    }) {
        None | Some("") => HookState::Off,
        Some(HOOKS_DIR) => HookState::On,
        Some(other) => HookState::Elsewhere(other.to_string()),
    }
}

/// `git config core.hooksPath` in `root`, or `None` when unset (or not a repository).
pub fn hooks_path(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The state of the app at `root`.
pub fn current(root: &Path) -> HookState {
    state(root.join(HOOKS_DIR).is_dir(), hooks_path(root).as_deref())
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let out = io(
        "run git (is it installed?)",
        Command::new("git").arg("-C").arg(root).args(args).output(),
    )?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Tool(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// `kitchen hooks <install|uninstall|status>` for the app at `root`.
pub fn run(root: &Path, action: &str, dry_run: bool) -> Result<Report> {
    let dir = root.join(HOOKS_DIR);
    match action {
        "status" => Ok(match current(root) {
            HookState::On => Report::new("Git hooks are on (core.hooksPath = .githooks)."),
            HookState::Off => {
                let mut r = Report::new("Git hooks are off.");
                r.next.push("kitchen hooks install".into());
                r
            }
            HookState::Elsewhere(p) => {
                let mut r = Report::new(format!("git uses another hooks folder: {p}"));
                r.next
                    .push("kitchen hooks install (replaces it with .githooks)".into());
                r
            }
            HookState::Missing => Report::new("This app has no .githooks folder."),
        }),
        "install" => {
            if !dir.is_dir() {
                return Err(Error::Usage(format!(
                    "{} has no {HOOKS_DIR} folder; `kitchen upgrade` brings the template's",
                    root.display()
                )));
            }
            if !root.join(".git").exists() {
                return Err(Error::Usage(format!(
                    "{} is not a git repository; run `git init` first",
                    root.display()
                )));
            }
            let mut r = Report::new(if dry_run {
                "Would turn the git hooks on."
            } else {
                "Git hooks are on: commits now run .githooks/pre-commit."
            });
            r.lines
                .push(format!("git config core.hooksPath {HOOKS_DIR}"));
            if !dry_run {
                make_executable(&dir)?;
                git(root, &["config", "core.hooksPath", HOOKS_DIR])?;
            }
            r.next.push(
                "skip once with `git commit --no-verify`; `kitchen hooks uninstall` turns them off"
                    .into(),
            );
            Ok(r)
        }
        "uninstall" => {
            let mut r = Report::new(if dry_run {
                "Would turn the git hooks off."
            } else {
                "Git hooks are off."
            });
            r.lines.push("git config --unset core.hooksPath".into());
            if !dry_run && hooks_path(root).is_some() {
                git(root, &["config", "--unset", "core.hooksPath"])?;
            }
            Ok(r)
        }
        other => Err(Error::Usage(format!(
            "kitchen hooks {other}: use install, uninstall or status"
        ))),
    }
}

/// Unix: git runs only executable hooks, and a template copy may have lost the bit.
fn make_executable(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in io(format!("read {}", dir.display()), std::fs::read_dir(dir))? {
            let path = io("read .githooks", entry)?.path();
            if path.is_file() {
                let mut perms =
                    io(format!("read {}", path.display()), std::fs::metadata(&path))?.permissions();
                perms.set_mode(perms.mode() | 0o755);
                io(
                    format!("make {} executable", path.display()),
                    std::fs::set_permissions(&path, perms),
                )?;
            }
        }
    }
    let _ = dir;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_ok() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    #[test]
    fn install_status_uninstall() {
        if !git_ok() {
            return;
        }
        let tmp = crate::TempDir::new("hooks").unwrap();
        let root = tmp.path();
        assert!(run(root, "install", false).is_err(), "no .githooks yet");
        std::fs::create_dir_all(root.join(HOOKS_DIR)).unwrap();
        std::fs::write(root.join(HOOKS_DIR).join("pre-commit"), "#!/bin/sh\n").unwrap();
        assert!(run(root, "install", false).is_err(), "not a repository yet");
        git(root, &["init", "-q"]).unwrap();
        assert_eq!(current(root), HookState::Off);
        run(root, "install", true).unwrap();
        assert_eq!(current(root), HookState::Off, "dry run changes nothing");
        run(root, "install", false).unwrap();
        assert_eq!(current(root), HookState::On);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(root.join(HOOKS_DIR).join("pre-commit"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert!(run(root, "status", false).unwrap().summary.contains("on"));
        run(root, "uninstall", false).unwrap();
        assert_eq!(current(root), HookState::Off);
        run(root, "uninstall", false).unwrap(); // twice is fine
        assert!(run(root, "frobnicate", false).is_err());
    }
}
