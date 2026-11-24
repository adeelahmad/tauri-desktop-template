//! `kitchen service`: install and control an app's daemon (template feature `daemon`) from
//! its folder.
//!
//! It finds the app's built binary (`target/debug/<name>` or `target/release/<name>`,
//! whichever is newer; `CARGO_TARGET_DIR` is honoured), builds it with
//! `cargo build -p <name>` when there is none, and runs `<binary> --service <action>`.
//! With `--dry-run` it only says what it would run (and builds nothing); the MCP tool
//! `service` uses that for every action but `status` and `logs` unless `allow_changes`
//! is true.
//!
//! ```
//! use kitchen::service::{binary_candidates, service_args};
//! use std::path::Path;
//!
//! assert_eq!(service_args("install", true, None).unwrap(), ["--service", "install", "--system"]);
//! assert_eq!(service_args("logs", false, Some(20)).unwrap(), ["--service", "logs", "-n", "20"]);
//! assert!(service_args("reboot", false, None).is_err());
//!
//! let c = binary_candidates(Path::new("/apps/notes"), "notes", None);
//! assert!(c[0].ends_with("target/debug/notes") || c[0].ends_with("target/debug/notes.exe"));
//! ```

use crate::project::Project;
use crate::{Error, Options, Report, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The actions `<app> --service` takes.
pub const ACTIONS: &[&str] = &[
    "install",
    "uninstall",
    "start",
    "stop",
    "restart",
    "status",
    "logs",
];

/// Actions that change nothing on the machine.
pub fn read_only(action: &str) -> bool {
    matches!(action, "status" | "logs")
}

/// The arguments for the app binary.
pub fn service_args(action: &str, system: bool, lines: Option<usize>) -> Result<Vec<String>> {
    if !ACTIONS.contains(&action) {
        return Err(Error::Usage(format!(
            "unknown service action {action:?}: use one of {}",
            ACTIONS.join(", ")
        )));
    }
    let mut v = vec!["--service".to_string(), action.to_string()];
    if system {
        v.push("--system".into());
    }
    if let (Some(n), "logs") = (lines, action) {
        v.extend(["-n".into(), n.to_string()]);
    }
    Ok(v)
}

/// Where the built binary may be, in the order checked: debug, then release.
pub fn binary_candidates(root: &Path, name: &str, target_dir: Option<&Path>) -> Vec<PathBuf> {
    let target = target_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join("target"));
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    ["debug", "release"]
        .iter()
        .map(|p| target.join(p).join(&file))
        .collect()
}

fn target_dir(root: &Path) -> Option<PathBuf> {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|t| if t.is_relative() { root.join(t) } else { t })
}

/// The newest existing binary, if any.
pub fn find_binary(root: &Path, name: &str) -> Option<PathBuf> {
    binary_candidates(root, name, target_dir(root).as_deref())
        .into_iter()
        .filter_map(|p| {
            std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| (t, p))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// Run `action` on the app at `project`.
pub fn run(
    project: &Project,
    action: &str,
    system: bool,
    lines: Option<usize>,
    opts: &Options,
) -> Result<Report> {
    let args = service_args(action, system, lines)?;
    if !project.manifest.is_on("daemon") {
        return Err(Error::Usage(
            "this app has no daemon: turn it on with `kitchen enable daemon` (it needs ipc_worker)"
                .into(),
        ));
    }
    let name = &project.manifest.app.name;
    let root = &project.root;
    let existing = find_binary(root, name);
    if opts.dry_run {
        let bin = existing
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| format!("(after `cargo build -p {name}`) target/debug/{name}"));
        let mut r = Report::new(format!("would run: {bin} {}", args.join(" ")));
        if !read_only(action) {
            r.next
                .push("run it for real with allow_changes (MCP) or without --dry-run".into());
        }
        return Ok(r);
    }
    let bin = match existing {
        Some(b) => b,
        None => {
            let status = Command::new("cargo")
                .args(["build", "-p", name])
                .current_dir(root)
                .status()
                .map_err(|e| Error::Tool(format!("cargo build: {e}")))?;
            if !status.success() {
                return Err(Error::Tool(format!("cargo build -p {name} failed")));
            }
            find_binary(root, name).ok_or_else(|| {
                Error::Tool(format!(
                    "built, but found no binary for {name} under target/"
                ))
            })?
        }
    };
    let out = Command::new(&bin)
        .args(&args)
        .current_dir(root)
        .output()
        .map_err(|e| Error::Io(format!("run {}", bin.display()), e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    if !out.status.success() {
        let msg = [stdout.as_str(), stderr.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Tool(if msg.is_empty() {
            format!("{} {} failed", bin.display(), args.join(" "))
        } else {
            msg
        }));
    }
    let mut lines_out = stdout.lines();
    let mut r = Report::new(lines_out.next().unwrap_or("done").to_string());
    r.lines = lines_out.map(|l| l.trim().to_string()).collect();
    if !stderr.is_empty() {
        r.lines.extend(stderr.lines().map(String::from));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_status_and_logs_are_read_only() {
        assert!(read_only("status") && read_only("logs"));
        assert!(ACTIONS.iter().filter(|a| !read_only(a)).count() == 5);
    }

    #[test]
    fn finds_the_newest_binary() {
        let dir = crate::TempDir::new("svc").unwrap();
        let root = dir.path();
        assert_eq!(find_binary(root, "app"), None);
        let [debug, release] = [0, 1].map(|i| binary_candidates(root, "app", None)[i].clone());
        std::fs::create_dir_all(debug.parent().unwrap()).unwrap();
        std::fs::write(&debug, "x").unwrap();
        // Only when CARGO_TARGET_DIR is not set for this test run.
        if std::env::var_os("CARGO_TARGET_DIR").is_none() {
            assert_eq!(find_binary(root, "app"), Some(debug.clone()));
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::create_dir_all(release.parent().unwrap()).unwrap();
            std::fs::write(&release, "x").unwrap();
            assert_eq!(find_binary(root, "app"), Some(release));
        }
    }

    #[test]
    fn needs_the_daemon_feature() {
        let dir = crate::TempDir::new("svc2").unwrap();
        std::fs::write(
            dir.path().join("kitchen.toml"),
            "[app]\nname = \"x\"\n[answers]\ndaemon = false\n",
        )
        .unwrap();
        let p = Project::find(dir.path()).unwrap();
        let e = run(&p, "status", false, None, &Options::default())
            .unwrap_err()
            .to_string();
        assert!(e.contains("kitchen enable daemon"), "{e}");
        std::fs::write(
            dir.path().join("kitchen.toml"),
            "[app]\nname = \"x\"\n[answers]\ndaemon = true\n",
        )
        .unwrap();
        let p = Project::find(dir.path()).unwrap();
        let r = run(&p, "install", true, None, &Options { dry_run: true }).unwrap();
        assert!(
            r.summary.contains("--service install --system"),
            "{}",
            r.summary
        );
    }
}
