//! Command-line arguments: one small parser and one dispatcher, used by the `kitchen`
//! binary and by the MCP server alike.
//!
//! ```
//! use kitchen::cli::Args;
//!
//! let a = Args::parse(["generate", "setting", "invoices.due_days", "int", "30", "--label", "Days", "--dry-run"]);
//! assert_eq!(a.words, ["generate", "setting", "invoices.due_days", "int", "30"]);
//! assert_eq!(a.value("label"), Some("Days"));
//! assert!(a.flag("dry-run"));
//! ```

use crate::lifecycle::{self, Io, Platform, TestKinds};
use crate::project::{self, NewApp, Project};
use crate::{doctor, generate, service, Error, Options, Report, Result};
use std::path::{Path, PathBuf};

/// The help text.
pub const HELP: &str = "kitchen: create and grow cross-platform desktop apps

Usage:
  kitchen new <name> [--enable a,b] [--disable c] [--set key=value] [--dir <parent>]
                     [--source <git url or folder>] [--rev <commit|tag|branch>] [--no-git]
  kitchen features                     list the features and which are on
  kitchen enable <feature>...          turn features on (keeps your edits; 3-way merge)
  kitchen disable <feature>...         turn features off
  kitchen set <name>=<value>           change an answer, e.g. frontend=vanilla
  kitchen upgrade [--rev <rev>]        move to a newer template, keeping your edits
  kitchen generate module <name> [--config]
  kitchen generate setting <module>.<key> <bool|int|float|text|list> [default] [--label <text>]
  kitchen generate command <name>
  kitchen generate page <Name>
  kitchen generate job <name>          a background job type (needs background_jobs)
  kitchen generate menu <id> [--label <text>] [--shortcut CmdOrCtrl+Shift+K]   (tray_menu)
  kitchen generate shortcut <action> <keys>                                   (tray_menu)
  kitchen generate route <name>        POST /v1/<name> on the HTTP API (needs http_api)
  kitchen doctor [--json]              check this machine and this app

  kitchen start [--install]            run the app with hot reload (cargo tauri dev, KITCHEN_ENV=development)
  kitchen test [--unit|--feature]      unit tests (cargo test / nextest) and the real app over WebDriver
  kitchen package [mac|windows|linux] [--release]
                                       installers for this OS (cargo tauri build), copied to ./packages
  kitchen publish [--tag v1.2.3] [--yes]
                                       GitHub release with ./packages via gh (dry run without --yes)
  kitchen eject vite|tauri|cargo       where that tool's own config lives, and what Kitchen adds
  kitchen service <install|uninstall|start|stop|restart|status|logs> [--system] [--lines N] [--follow]
                                       the app's daemon as an OS service (feature daemon)
  kitchen build                        cargo tauri build, as is
  kitchen mcp                          serve these commands to AI agents (MCP over stdio)

Options:
  --dry-run   show what would change, change nothing
  --json      print the result as JSON
  -h, --help  this text;  -V, --version  the version

`kitchen g` is short for `kitchen generate`; `kitchen dev` is `kitchen start`.
";

/// Parsed arguments: words in order, and `--flag` / `--key value` / `--key=value` options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args {
    /// Positional words (`generate`, `module`, `invoices` ...).
    pub words: Vec<String>,
    /// Options in order, `(name, value)`; a bare flag has value `None`.
    pub options: Vec<(String, Option<String>)>,
}

/// Options that take a value (all others are flags).
const VALUED: &[&str] = &[
    "enable", "disable", "set", "dir", "source", "rev", "label", "shortcut", "lines", "tag",
];

impl Args {
    /// Parse `argv` (without the program name).
    pub fn parse<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut a = Args::default();
        let mut it = argv.into_iter().map(Into::into);
        while let Some(w) = it.next() {
            if let Some(opt) = w.strip_prefix("--") {
                match opt.split_once('=') {
                    Some((k, v)) => a.options.push((k.to_string(), Some(v.to_string()))),
                    None if VALUED.contains(&opt) => a.options.push((opt.to_string(), it.next())),
                    None => a.options.push((opt.to_string(), None)),
                }
            } else if w == "-h" {
                a.options.push(("help".into(), None));
            } else if w == "-V" {
                a.options.push(("version".into(), None));
            } else {
                a.words.push(w);
            }
        }
        a
    }

    /// Whether `--name` was given.
    pub fn flag(&self, name: &str) -> bool {
        self.options.iter().any(|(k, _)| k == name)
    }

    /// The last value of `--name`.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Every value of `--name`, comma lists split.
    pub fn values(&self, name: &str) -> Vec<String> {
        self.options
            .iter()
            .filter(|(k, _)| k == name)
            .filter_map(|(_, v)| v.as_deref())
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    fn word(&self, i: usize, what: &str) -> Result<&str> {
        self.words
            .get(i)
            .map(String::as_str)
            .ok_or_else(|| Error::Usage(format!("missing {what}; see `kitchen --help`")))
    }
}

/// Run one command that produces a report, as if started in `cwd`. Tools that `start`,
/// `test` and `package` run have their output captured into the report.
pub fn run(args: &Args, cwd: &Path) -> Result<Report> {
    run_with(args, cwd, Io::Capture)
}

/// [`run`], choosing where the output of the tools goes ([`Io::Inherit`]: the terminal).
pub fn run_with(args: &Args, cwd: &Path, io_mode: Io) -> Result<Report> {
    let opts = Options {
        dry_run: args.flag("dry-run"),
    };
    let project = || Project::find(cwd);
    let cmd = args.words.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "new" => {
            let set = args
                .values("set")
                .into_iter()
                .map(|kv| {
                    kv.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .ok_or_else(|| Error::Usage(format!("--set {kv}: use key=value")))
                })
                .collect::<Result<_>>()?;
            project::create(&NewApp {
                name: args.word(1, "the app name")?.to_string(),
                dir: Some(
                    args.value("dir")
                        .map(|d| cwd.join(d))
                        .unwrap_or_else(|| cwd.to_path_buf()),
                ),
                source: args
                    .value("source")
                    .map(|s| {
                        if Path::new(s).is_relative() && cwd.join(s).is_dir() {
                            cwd.join(s).display().to_string()
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_default(),
                rev: args.value("rev").map(String::from),
                enable: args.values("enable"),
                disable: args.values("disable"),
                set,
                no_git: args.flag("no-git"),
            })
        }
        "features" | "list" => project()?.features_report(),
        "enable" | "disable" => {
            let names: Vec<&str> = args.words[1..]
                .iter()
                .flat_map(|w| w.split(','))
                .filter(|s| !s.is_empty())
                .collect();
            if names.is_empty() {
                return Err(Error::Usage(format!(
                    "name a feature: kitchen {cmd} <feature>; `kitchen features` lists them"
                )));
            }
            let p = project()?;
            if cmd == "enable" {
                p.set_features(&names, &[], &opts)
            } else {
                p.set_features(&[], &names, &opts)
            }
        }
        "set" => {
            let kv = args.word(1, "name=value")?;
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| Error::Usage(format!("{kv}: use name=value")))?;
            project()?.set_answer(k, v, &opts)
        }
        "upgrade" => project()?.upgrade(args.value("rev"), &opts),
        "generate" | "g" => {
            let root = project()?.root;
            let kind = args.word(
                1,
                "what to generate (module, setting, command, page, menu, shortcut, job, route)",
            )?;
            let name = args.word(2, "a name")?;
            match kind {
                "module" | "mod" => generate::module(&root, name, args.flag("config"), &opts),
                "setting" => generate::setting(
                    &root,
                    name,
                    args.word(3, "the type (bool, int, float, text, list)")?,
                    args.words.get(4).map(String::as_str),
                    args.value("label"),
                    &opts,
                ),
                "command" | "cmd" => generate::command(&root, name, &opts),
                "page" => generate::page(&root, name, &opts),
                "menu" => generate::menu(
                    &root,
                    name,
                    args.value("label"),
                    args.value("shortcut"),
                    &opts,
                ),
                "shortcut" => generate::shortcut(
                    &root,
                    name,
                    args.word(3, "the keys, e.g. CmdOrCtrl+Shift+K")?,
                    &opts,
                ),
                "job" => generate::job(&root, name, &opts),
                "route" => generate::route(&root, name, &opts),
                other => Err(Error::Usage(format!(
                    "cannot generate {other:?}: module, setting, command, page, menu, shortcut, job or route"
                ))),
            }
        }
        "doctor" => Ok(doctor::report(&doctor::run(Some(cwd)))),
        "start" | "dev" => {
            let root = project()?.root;
            if io_mode == Io::Capture {
                let mut r = Report::new(
                    "`kitchen start` keeps running (it serves the UI and rebuilds on every edit), so run it in a terminal.",
                );
                r.next = vec![
                    format!("cd {} && kitchen start", root.display()),
                    "it runs: KITCHEN_ENV=development cargo tauri dev".into(),
                ];
                return Ok(r);
            }
            let plan = lifecycle::start_plan(&root, &args.words[1..], args.flag("install"))?;
            lifecycle::execute(&plan, io_mode, "The app stopped.")
        }
        "test" => test(args, &project()?.root, io_mode),
        "package" => {
            let root = project()?.root;
            let host = host()?;
            let platform = args.words.get(1).map(|w| Platform::parse(w)).transpose()?;
            let release = args.flag("release");
            let mut plan = lifecycle::package_plan(&root, platform, host, release)?;
            lifecycle::prerequisites(&root, args.flag("install"), false, &mut plan)?;
            let mut r = lifecycle::execute(&plan, io_mode, "")?;
            let files = lifecycle::collect_packages(&root, release)?;
            r.summary = format!(
                "{} package(s) in {}",
                files.len(),
                root.join("packages").display()
            );
            r.lines
                .extend(files.iter().map(|f| format!("package {}", f.display())));
            r.next.push(
                "kitchen publish   (shows the GitHub release it would make; --yes makes it)".into(),
            );
            Ok(r)
        }
        "publish" => {
            let p = project()?;
            let repo = p
                .manifest
                .answers
                .get("github_repo")
                .and_then(|v| v.as_str())
                .map(String::from);
            let title = p
                .manifest
                .answers
                .get("app_name")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| p.manifest.app.name.clone());
            let plan =
                lifecycle::publish_plan(&p.root, args.value("tag"), repo.as_deref(), &title)?;
            let has_gh = std::process::Command::new("gh")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .status()
                .is_ok();
            let manual = vec![
                "install the GitHub CLI (https://cli.github.com) and run `gh auth login`, then `kitchen publish --yes`".to_string(),
                "or by hand: push the tag, open https://github.com/<owner>/<repo>/releases/new, pick the tag and attach the files in ./packages".to_string(),
            ];
            if !args.flag("yes") {
                let mut r =
                    Report::new("Dry run: this would create the release (add --yes to do it).");
                r.lines = plan.steps.iter().map(|s| s.display()).collect();
                if !has_gh {
                    r.next = manual;
                }
                return Ok(r);
            }
            if !has_gh {
                return Err(Error::Tool(format!(
                    "gh is not installed:
  {}",
                    manual.join(
                        "
  "
                    )
                )));
            }
            lifecycle::execute(&plan, io_mode, "Published the release.")
        }
        "eject" => lifecycle::eject(
            &project()?.root,
            args.word(1, "a tool: vite, tauri or cargo")?,
        ),
        "service" => {
            let lines = args
                .value("lines")
                .map(|n| {
                    n.parse()
                        .map_err(|_| Error::Usage(format!("--lines {n}: give a number")))
                })
                .transpose()?;
            service::run(
                &project()?,
                args.words.get(1).map(String::as_str).unwrap_or("status"),
                args.flag("system"),
                lines,
                &opts,
            )
        }
        "help" => Ok(Report::new(HELP.trim_end())),
        other => Err(Error::Usage(format!(
            "unknown command {other:?}; see `kitchen --help`"
        ))),
    }
}

fn host() -> Result<Platform> {
    Platform::host().ok_or_else(|| {
        Error::Usage(format!(
            "{} is not a desktop OS Kitchen packages for",
            std::env::consts::OS
        ))
    })
}

/// `kitchen test`: `--unit`, `--feature`, or both. With neither flag, feature tests that
/// this machine cannot run are skipped with a note instead of failing.
fn test(args: &Args, root: &Path, io_mode: Io) -> Result<Report> {
    let (unit, feature) = (args.flag("unit"), args.flag("feature"));
    let kinds = if unit || feature {
        TestKinds { unit, feature }
    } else {
        TestKinds {
            unit: true,
            feature: true,
        }
    };
    let host = host()?;
    let nextest = std::process::Command::new("cargo")
        .args(["nextest", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let drivers = lifecycle::Drivers::detect();
    let mut plan = match lifecycle::test_plan(root, kinds, host, nextest, drivers) {
        Err(e) if !feature && kinds.feature => {
            let mut p = lifecycle::test_plan(
                root,
                TestKinds {
                    unit: true,
                    feature: false,
                },
                host,
                nextest,
                drivers,
            )?;
            p.notes.push(format!("feature tests skipped: {e}"));
            p
        }
        other => other?,
    };
    if kinds.feature
        && plan
            .steps
            .iter()
            .any(|s| s.args.first().is_some_and(|a| a == "tauri"))
    {
        lifecycle::prerequisites(root, args.flag("install"), false, &mut plan)?;
    }
    lifecycle::execute(&plan, io_mode, "Tests passed.")
}

/// The folder a command runs in: `--project <dir>` style callers pass it, else the
/// current folder.
pub fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| Error::Io("find the current folder".into(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_options() {
        let a = Args::parse([
            "new",
            "x",
            "--enable",
            "a,b",
            "--enable=c",
            "--no-git",
            "--set",
            "k=v",
        ]);
        assert_eq!(a.words, ["new", "x"]);
        assert_eq!(a.values("enable"), ["a", "b", "c"]);
        assert!(a.flag("no-git"));
        assert_eq!(a.value("set"), Some("k=v"));
    }

    #[test]
    fn errors_are_helpful() {
        let dir = crate::TempDir::new("cli").unwrap();
        let e = run(&Args::parse(["features"]), dir.path())
            .unwrap_err()
            .to_string();
        assert!(e.contains("kitchen new"), "{e}");
        let e = run(&Args::parse(["frobnicate"]), dir.path())
            .unwrap_err()
            .to_string();
        assert!(e.contains("--help"), "{e}");
        assert!(run(&Args::parse(["help"]), dir.path())
            .unwrap()
            .summary
            .contains("kitchen new"));
    }
}
