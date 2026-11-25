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
use crate::{
    audit, doctor, eval, generate, hooks, i18n, models, service, web, Error, Options, Report,
    Result,
};
use std::path::{Path, PathBuf};

/// The help text.
pub const HELP: &str = "kitchen: create and grow cross-platform desktop apps

Usage:
  kitchen new <name> [--enable a,b] [--disable c] [--set key=value] [--dir <parent>]
                     [--source <git url or folder>] [--rev <commit|tag|branch>] [--no-git]
                     [--with-deps] [--engine system|chromium]
                                       --engine chromium: Chromium (CEF) instead of WKWebView
                                       on macOS (Tauri 3 alpha; about 150 MB more)
  kitchen new <name> --web <https url> [--insecure-tls] [--capture-keys]
                     [--release-shortcut Ctrl+Alt+Shift+K] [--mic] [--muted] [--camera]
                     [--screen] [--scheme <myapp>] [--no-fetch]
                                       a web app: one site as a desktop app (its title, icon
                                       and certificate are read from the site). On a terminal,
                                       `kitchen new <name>` asks Desktop or Web first
  kitchen icon fetch <url> [--insecure-tls] [--out icon.png]
                                       the site's best icon as a 1024px PNG; in an app, its
                                       icon set
  kitchen enable deep_link [--scheme <myapp>]
                                       the deep-link plugin, and the scheme it registers
  kitchen features [--check]           list the features and which are on; --check: do the
                                       answers in kitchen.toml work together?
  kitchen enable <feature>... [--with-deps]
                                       turn features on (keeps your edits; 3-way merge);
                                       --with-deps also turns on what they need
  kitchen disable <feature>... [--with-deps]
                                       turn features off; --with-deps also turns off what needs them
  kitchen set <name>=<value>           change an answer, e.g. frontend=vanilla (same checks)
  kitchen upgrade [--rev <rev>]        move to a newer template, keeping your edits
  kitchen generate module <name> [--config]
  kitchen generate setting <module>.<key> <bool|int|float|text|list> [default] [--label <text>]
  kitchen generate command <name> [--no-bindings]   (also updates the UI's typed IPC bindings)
  kitchen generate page <Name>
  kitchen generate job <name>          a background job type (needs background_jobs)
  kitchen generate menu <id> [--label <text>] [--shortcut CmdOrCtrl+Shift+K]   (tray_menu)
  kitchen generate shortcut <action> <keys>                                   (tray_menu)
  kitchen generate route <name>        POST /v1/<name> on the HTTP API (needs http_api)
  kitchen generate locale <code> [--pseudo]
                                       locales/<code>/app.ftl from English, each message marked
                                       untranslated (or pseudo-translated) (needs i18n)
  kitchen i18n check                   every locale has exactly English's messages, valid Fluent,
                                       and the UI only asks for ids English has (needs i18n)
  kitchen hooks <install|uninstall|status>
                                       git hooks in .githooks/ (fmt, clippy, prettier, eslint)
  kitchen doctor [--json]              check this machine and this app

  kitchen start [--install]            run the app with hot reload (cargo tauri dev, KITCHEN_ENV=development);
                                       installs the UI's npm packages on the first run, and
                                       with --install also a missing Tauri CLI
  kitchen test [--unit|--feature]      unit tests (cargo test / nextest) and the real app over WebDriver
  kitchen package [mac|windows|linux] [--release]
                                       installers for this OS (cargo tauri build), copied to ./packages
  kitchen publish [--tag v1.2.3] [--yes]
                                       GitHub release with ./packages via gh (dry run without --yes)
  kitchen audit                        cargo-deny, cargo-audit, npm audit (ui-react) and pip-audit
                                       (python example); missing tools are skipped with a hint
  kitchen eject vite|tauri|cargo       where that tool's own config lives, and what Kitchen adds
  kitchen service <install|uninstall|start|stop|restart|status|logs> [--system] [--lines N] [--follow]
                                       the app's daemon as an OS service (feature daemon)
  kitchen eval [dataset] [--baseline <file>] [--threshold 0.02] [--k 5] [--answers]
                                       the app's search evals (<app> --eval, feature evals);
                                       fails on a regression against the baseline
  kitchen models list|add <name>|remove <name>|fetch|verify
                                       models built into the installer (models.lock):
                                       fetch downloads them and checks their SHA-256s
  kitchen enable bundled_models        same as kitchen set models=bundle_with_fallback
  kitchen build                        cargo tauri build, as is
  kitchen mcp                          serve these commands to AI agents (MCP over stdio)

Options:
  --dry-run   show what would change (and which dependencies), change nothing
  --with-deps switch the features a change needs too (see \"Feature dependencies\")
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
    "enable",
    "disable",
    "set",
    "dir",
    "source",
    "rev",
    "label",
    "shortcut",
    "lines",
    "tag",
    "web",
    "release-shortcut",
    "scheme",
    "engine",
    "out",
];

/// `kitchen eval`'s options that take a value.
const EVAL_VALUED: &[&str] = &["baseline", "threshold", "k"];

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
                    None if VALUED.contains(&opt) || EVAL_VALUED.contains(&opt) => {
                        a.options.push((opt.to_string(), it.next()))
                    }
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
        with_deps: args.flag("with-deps"),
    };
    let project = || Project::find(cwd);
    let cmd = args.words.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "new" => {
            let mut set: Vec<(String, String)> = args
                .values("set")
                .into_iter()
                .map(|kv| {
                    kv.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .ok_or_else(|| Error::Usage(format!("--set {kv}: use key=value")))
                })
                .collect::<Result<_>>()?;
            let name = args.word(1, "the app name")?.to_string();
            if let Some(e) = args.value("engine") {
                set.push(engine_answer(e)?);
            }
            let mut enable = args.values("enable");
            let mut lines = Vec::new();
            let mut icon_png = None;
            if let Some(url) = args.value("web") {
                let web = web_new(args, url, &name, &set)?;
                enable.extend(web.enable);
                set.extend(web.set);
                lines = web.lines;
                icon_png = web.icon;
            }
            let mut report = project::create(&NewApp {
                name,
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
                enable,
                disable: args.values("disable"),
                set,
                no_git: args.flag("no-git"),
                with_deps: opts.with_deps,
                icon_png,
            })?;
            report.lines.splice(0..0, lines);
            Ok(report)
        }
        "icon" => {
            match args.words.get(1).map(String::as_str) {
                Some("fetch") => {}
                _ => {
                    return Err(Error::Usage(
                        "use: kitchen icon fetch <url> [--insecure-tls] [--out icon.png]".into(),
                    ))
                }
            }
            let url = args.word(2, "the site's URL")?;
            let app = Project::find(cwd).ok().map(|p| p.root);
            web::icon_fetch(
                url,
                args.flag("insecure-tls"),
                app.as_deref(),
                args.value("out").map(|o| cwd.join(o)),
            )
        }
        "features" | "list" if args.flag("check") => Ok(project()?.check_features()),
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
            // `deep_link` is the deep-link plugin plus, with --scheme, the scheme it registers.
            if names == ["deep_link"] && cmd == "enable" {
                let mut r = p.set_features(&["tauri_plugins", "tauri_deep_link"], &[], &opts)?;
                if let Some(scheme) = args.value("scheme") {
                    let scheme = web::check_scheme(scheme)?;
                    let p = project()?;
                    let r2 = p.set_answer("deep_link_scheme", &scheme, &opts)?;
                    r.lines.extend(r2.lines);
                    r.next.extend(r2.next);
                }
                return Ok(r);
            }
            // `bundled_models` is not a question: it is the `models` answer.
            if names == ["bundled_models"] {
                let mode = if cmd == "enable" {
                    "bundle_with_fallback"
                } else {
                    "download"
                };
                return p.set_answer("models", mode, &opts);
            }
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
                "what to generate (module, setting, command, page, menu, shortcut, job, route, locale)",
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
                "command" | "cmd" => {
                    let mut r = generate::command(&root, name, &opts)?;
                    // Typed IPC: the UI's TypeScript types follow the new command.
                    if !opts.dry_run && !args.flag("no-bindings") {
                        let done = lifecycle::bindings_plan(&root)
                            .and_then(|p| lifecycle::execute(&p, io_mode, ""));
                        match done {
                            Ok(_) => r.lines.push("update   typed IPC bindings (cargo test export_bindings)".into()),
                            Err(e) => r.next.push(format!(
                                "update the typed IPC bindings once it builds: KITCHEN_BINDINGS=write cargo test -p <app> --lib export_bindings ({e})"
                            )),
                        }
                    }
                    Ok(r)
                }
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
                "locale" => i18n::generate_locale(&root, name, args.flag("pseudo"), &opts),
                other => Err(Error::Usage(format!(
                    "cannot generate {other:?}: module, setting, command, page, menu, shortcut, job, route or locale"
                ))),
            }
        }
        "models" => {
            let p = project()?;
            let action = args.words.get(1).map(String::as_str).unwrap_or("list");
            match action {
                "list" | "ls" => models::list(&p.root),
                "add" => models::add(
                    &p.root,
                    p.manifest.text("models").as_deref(),
                    args.word(2, "a model name (`kitchen models list`)")?,
                    &opts,
                ),
                "remove" | "rm" => models::remove(
                    &p.root,
                    args.word(2, "a model name (`kitchen models list`)")?,
                    &opts,
                ),
                other => {
                    let plan = models::plan(&p.root, other)?;
                    if opts.dry_run {
                        let mut r = Report::new("Dry run: this would run:");
                        r.lines = plan.steps.iter().map(|s| s.display()).collect();
                        r.lines.extend(plan.notes);
                        return Ok(r);
                    }
                    lifecycle::execute(
                        &plan,
                        io_mode,
                        "The bundled models are in place and their checksums match.",
                    )
                }
            }
        }
        "hooks" => hooks::run(
            &project()?.root,
            args.words.get(1).map(String::as_str).unwrap_or("status"),
            opts.dry_run,
        ),
        "doctor" => Ok(doctor::report(&doctor::run(Some(cwd)))),
        "i18n" => match args.words.get(1).map(String::as_str).unwrap_or("check") {
            "check" => i18n::check(&project()?.root),
            other => Err(Error::Usage(format!(
                "kitchen i18n {other}: the only subcommand is check"
            ))),
        },
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
        "eval" => {
            let p = project()?;
            let abs = |s: Option<&str>| s.map(|s| eval::absolute(cwd, s));
            let dataset = abs(args.words.get(1).map(String::as_str));
            let baseline = abs(args.value("baseline"));
            let a = eval::eval_args(
                dataset.as_deref(),
                baseline.as_deref(),
                args.value("threshold"),
                args.value("k"),
                args.flag("answers"),
            );
            eval::run(&p, &a, &opts)
        }
        "package" => {
            let root = project()?.root;
            let host = host()?;
            let platform = args.words.get(1).map(|w| Platform::parse(w)).transpose()?;
            let release = args.flag("release");
            let mut plan = lifecycle::package_plan(&root, platform, host, release)?;
            if is_chromium(&root) {
                plan.notes
                    .extend(lifecycle::chromium_notes(platform.unwrap_or(host)));
            }
            let key = std::env::var_os("TAURI_SIGNING_PRIVATE_KEY").is_some_and(|k| !k.is_empty());
            lifecycle::skip_updater_artifacts(&mut plan, key);
            lifecycle::prerequisites(&root, args.flag("install"), false, &mut plan)?;
            let started = std::time::SystemTime::now();
            let mut r = lifecycle::execute(&plan, io_mode, "")?;
            let files = lifecycle::collect_packages(&root, release, Some(started))?;
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
        "audit" => {
            let root = project()?.root;
            audit::run(&audit::plan(&root, audit::Tools::detect()), io_mode)
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
    // Translations (feature i18n) are checked first: no build needed.
    let translations = if root.join("locales").is_dir() {
        Some(i18n::check(root)?)
    } else {
        None
    };
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
    let mut r = lifecycle::execute(&plan, io_mode, "Tests passed.")?;
    if let Some(t) = translations {
        r.lines.insert(0, t.summary);
    }
    Ok(r)
}

/// Whether the app at `root` was generated with `web_engine = "chromium"`.
fn is_chromium(root: &Path) -> bool {
    crate::manifest::Manifest::load(root)
        .ok()
        .and_then(|m| m.text("web_engine"))
        .is_some_and(|e| e == "chromium")
}

/// `--engine system|chromium` for `kitchen new`: the `web_engine` answer.
///
/// ```
/// use kitchen::cli::engine_answer;
///
/// assert_eq!(engine_answer("chromium").unwrap(), ("web_engine".to_string(), "chromium".to_string()));
/// assert_eq!(engine_answer("CEF").unwrap().1, "chromium");
/// assert!(engine_answer("gecko").is_err());
/// ```
pub fn engine_answer(engine: &str) -> Result<(String, String)> {
    let e = match engine.trim().to_ascii_lowercase().as_str() {
        "system" | "webkit" | "native" => "system",
        "chromium" | "cef" | "chrome" => "chromium",
        other => {
            return Err(Error::Usage(format!(
                "--engine {other}: use system (the OS web view, the default) or chromium (CEF on macOS)"
            )))
        }
    };
    Ok(("web_engine".into(), e.into()))
}

/// The folder a command runs in: `--project <dir>` style callers pass it, else the
/// current folder.
pub fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| Error::Io("find the current folder".into(), e))
}

/// What `kitchen new --web` adds to the new app's answers.
struct WebNew {
    enable: Vec<String>,
    set: Vec<(String, String)>,
    lines: Vec<String>,
    icon: Option<Vec<u8>>,
}

/// `kitchen new <name> --web <url>`: check the URL, look at the site (title, certificate,
/// icon) unless `--no-fetch`, and turn the flags into answers. The name and identifier
/// come from the page title and host unless `--set` gives them.
fn web_new(args: &Args, url: &str, name: &str, given: &[(String, String)]) -> Result<WebNew> {
    let u = web::check_url(url)?;
    let insecure = args.flag("insecure-tls");
    let mut a = web::WebAnswers {
        url: url.to_string(),
        insecure_tls: insecure,
        capture_keys: args.flag("capture-keys"),
        release_shortcut: args
            .value("release-shortcut")
            .map(web::check_shortcut)
            .transpose()?
            .unwrap_or_default(),
        microphone: args.flag("mic") || args.flag("microphone"),
        muted: args.flag("muted"),
        camera: args.flag("camera") || args.flag("webcam"),
        screen: args.flag("screen"),
        scheme: args
            .value("scheme")
            .map(web::check_scheme)
            .transpose()?
            .unwrap_or_default(),
        ..web::WebAnswers::default()
    };
    let mut lines = Vec::new();
    let mut title = None;
    let mut icon = None;
    if !args.flag("no-fetch") {
        let p = web::probe(&u, insecure)?;
        title = p.title.clone();
        if let Some(fp) = &p.sha256 {
            lines.push(format!(
                "site certificate SHA-256 {}{}",
                web::display_fingerprint(fp),
                if p.trusted { " (trusted)" } else { " (self-signed or untrusted: pinned in the app; compare it with the device's own)" }
            ));
            if insecure && !p.trusted {
                a.cert_sha256 = fp.clone();
            }
        }
        match &p.icon_from {
            Some((from, side)) => lines.push(format!("site icon {from} ({side}px)")),
            None => lines.push("site icon: none found, the app keeps the template's".into()),
        }
        lines.extend(p.notes.iter().map(|n| format!("skipped  {n}")));
        icon = p.icon;
    }
    let (enable, mut set) = a.answers();
    let (app_name, identifier) = web::names(title.as_deref(), &u, name);
    for (k, v) in [("app_name", app_name), ("identifier", identifier)] {
        if !given.iter().any(|(g, _)| g == k) {
            set.push((k.into(), v));
        }
    }
    Ok(WebNew {
        enable,
        set,
        lines,
        icon,
    })
}

/// `kitchen new <name>` on a terminal, with no other choices given: ask Desktop or Web
/// first, and for a web app the site's questions in order (URL, TLS, key capture, release
/// shortcut, microphone, muted, webcam, screen sharing). Returns the arguments to run.
/// `input` and `output` are the terminal (or anything else, for tests).
///
/// ```
/// use kitchen::cli::{ask_new, Args};
///
/// let args = Args::parse(["new", "kvm"]);
/// let mut answers = "2\nhttps://pikvm.local\ny\ny\n\nn\ny\n\n\n\n".as_bytes();
/// let out = ask_new(&args, &mut answers, &mut Vec::new()).unwrap();
/// assert_eq!(out.value("web"), Some("https://pikvm.local"));
/// assert!(out.flag("insecure-tls") && out.flag("capture-keys") && out.flag("muted"));
/// assert!(!out.flag("mic") && !out.flag("camera") && out.value("release-shortcut").is_none());
/// assert!(out.value("engine").is_none());
///
/// // A desktop app on Chromium.
/// let out = ask_new(&args, &mut "1\n2\n".as_bytes(), &mut Vec::new()).unwrap();
/// assert_eq!(out.value("engine"), Some("chromium"));
/// ```
pub fn ask_new(
    args: &Args,
    input: &mut impl std::io::BufRead,
    output: &mut impl std::io::Write,
) -> Result<Args> {
    let mut t = Terminal { input, output };
    let mut out = args.clone();
    loop {
        match t.ask("What are you making?\n  1) Desktop app (boilerplate)\n  2) Web app (wrap a URL)\nChoose 1 or 2 [1]:")?.as_str() {
            "" | "1" | "d" | "desktop" => {
                ask_engine(&mut t, &mut out)?;
                return Ok(out);
            }
            "2" | "w" | "web" => break,
            _ => {}
        }
    }
    let url = loop {
        let u = t.ask("Site URL (https://...):")?;
        match web::check_url(&u) {
            Ok(_) => break u,
            Err(e) => t.say(&e.to_string()),
        }
    };
    out.options.push(("web".into(), Some(url)));
    let mut flag = |t: &mut Terminal<_, _>, q: &str, name: &str| -> Result<()> {
        if t.yes_no(q)? {
            out.options.push((name.into(), None));
        }
        Ok(())
    };
    flag(&mut t, "Ignore TLS certificate errors for this site? (self-signed, e.g. PiKVM; the certificate is pinned)", "insecure-tls")?;
    flag(
        &mut t,
        "Capture all keys (Cmd/Win, Cmd+Tab, Alt+Tab, media keys) and send them to the page?",
        "capture-keys",
    )?;
    let shortcut = loop {
        let s = t.ask(
            "Shortcut that releases the keys (changeable in Preferences) [Ctrl+Alt+Shift+K]:",
        )?;
        if s.is_empty() {
            break None;
        }
        match web::check_shortcut(&s) {
            Ok(s) => break Some(s),
            Err(e) => t.say(&e.to_string()),
        }
    };
    flag(&mut t, "Allow microphone?", "mic")?;
    flag(&mut t, "Start muted?", "muted")?;
    flag(&mut t, "Allow webcam?", "camera")?;
    flag(&mut t, "Allow screen sharing?", "screen")?;
    if let Some(s) = shortcut {
        out.options.push(("release-shortcut".into(), Some(s)));
    }
    ask_engine(&mut t, &mut out)?;
    Ok(out)
}

/// The last question of [`ask_new`]: the web engine on macOS. Only a Chromium answer adds
/// `--engine chromium`; the system web view is the default.
fn ask_engine<R: std::io::BufRead, W: std::io::Write>(
    t: &mut Terminal<'_, R, W>,
    out: &mut Args,
) -> Result<()> {
    loop {
        match t
            .ask("Web engine on macOS?\n  1) System web view (WKWebView, small)\n  2) Chromium (CEF: Tauri 3 alpha, about 150 MB more, Chromium security updates are yours to ship)\nChoose 1 or 2 [1]:")?
            .as_str()
        {
            "" | "1" | "s" | "system" => return Ok(()),
            "2" | "c" | "chromium" => {
                out.options.push(("engine".into(), Some("chromium".into())));
                return Ok(());
            }
            _ => {}
        }
    }
}

/// Questions and answers on a terminal.
struct Terminal<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
}

impl<R: std::io::BufRead, W: std::io::Write> Terminal<'_, R, W> {
    fn ask(&mut self, q: &str) -> Result<String> {
        let _ = write!(self.output, "{q} ");
        let _ = self.output.flush();
        let mut line = String::new();
        crate::io("read the answer", self.input.read_line(&mut line))?;
        if line.is_empty() {
            return Err(Error::Usage("no answer (end of input)".into()));
        }
        Ok(line.trim().to_string())
    }

    fn say(&mut self, s: &str) {
        let _ = writeln!(self.output, "  {s}");
    }

    fn yes_no(&mut self, q: &str) -> Result<bool> {
        loop {
            match self
                .ask(&format!("{q} [y/N]:"))?
                .to_ascii_lowercase()
                .as_str()
            {
                "y" | "yes" => return Ok(true),
                "" | "n" | "no" => return Ok(false),
                _ => {}
            }
        }
    }
}

/// Whether `kitchen new` should ask [`ask_new`]'s questions: nothing chosen on the command
/// line yet (no `--web`, `--enable`, `--set`, `--engine`, `--json` or `--yes`).
pub fn should_ask_new(args: &Args) -> bool {
    args.words.first().is_some_and(|w| w == "new")
        && !["web", "enable", "disable", "set", "json", "yes", "engine"]
            .iter()
            .any(|f| args.flag(f))
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
