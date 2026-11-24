//! Commands on an app: create one, list its features, turn them on and off, change an
//! answer, and upgrade to a newer template. All of them return a [`Report`] and print
//! nothing.
//!
//! ```no_run
//! use kitchen::{project::{self, Project}, Options};
//! use std::path::Path;
//!
//! // kitchen new notes --enable email
//! let report = project::create(&project::NewApp { name: "notes".into(), enable: vec!["email".into()], ..Default::default() })?;
//! println!("{report}");
//!
//! // kitchen enable tts --dry-run
//! let app = Project::find(Path::new("notes"))?;
//! println!("{}", app.set_features(&["tts"], &[], &Options { dry_run: true, ..Options::default() })?);
//! # Ok::<(), kitchen::Error>(())
//! ```

use crate::compat::{self, Level};
use crate::manifest::{Manifest, FILE};
use crate::merge::{self, Action};
use crate::template::{Feature, Template};
use crate::{Error, Options, Report, Result};
use std::path::{Path, PathBuf};

/// A generated app on disk.
#[derive(Debug, Clone)]
pub struct Project {
    /// The folder with `kitchen.toml`.
    pub root: PathBuf,
    /// Its contents.
    pub manifest: Manifest,
}

/// A feature and its value in this app.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FeatureState {
    /// The question.
    #[serde(flatten)]
    pub feature: Feature,
    /// The app's answer (the default when it has none).
    pub value: toml::Value,
}

impl Project {
    /// The app at or above `start`.
    pub fn find(start: &Path) -> Result<Self> {
        let root = Manifest::find_root(start)
            .ok_or_else(|| Error::Usage(format!("no {FILE} at or above {}: run this inside a Kitchen app, or create one with `kitchen new`", start.display())))?;
        Ok(Self {
            manifest: Manifest::load(&root)?,
            root,
        })
    }

    /// The template this app was generated from.
    pub fn template(&self) -> Result<Template> {
        Template::open(
            &self.manifest.template.source,
            Some(&self.manifest.template.rev),
        )
    }

    /// Every question of the template, with this app's answer.
    pub fn features(&self) -> Result<Vec<FeatureState>> {
        Ok(self
            .template()?
            .features()?
            .into_iter()
            .map(|f| FeatureState {
                value: self
                    .manifest
                    .answers
                    .get(&f.name)
                    .cloned()
                    .unwrap_or_else(|| f.default.clone()),
                feature: f,
            })
            .collect())
    }

    /// The features as a report: `on`/`off` (or the value) with each question.
    pub fn features_report(&self) -> Result<Report> {
        let list = self.features()?;
        let mut r = Report::new(format!(
            "{} features in {}",
            list.len(),
            self.root.display()
        ));
        for s in &list {
            let value = match &s.value {
                toml::Value::Boolean(true) => "on".to_string(),
                toml::Value::Boolean(false) => "off".to_string(),
                toml::Value::String(v) => v.clone(),
                v => v.to_string(),
            };
            let mut needs = compat::needs_of(&s.feature.name);
            if let Some(p) = s.feature.requires.as_deref() {
                // A compound condition (`models != "download" && stt`) is said as is,
                // unless the rule table already names what it needs.
                if !needs.contains(&p) && (needs.is_empty() || !p.contains(' ')) {
                    needs.push(p);
                }
            }
            let needs = if needs.is_empty() {
                String::new()
            } else {
                format!(" (needs {})", needs.join(", "))
            };
            r.lines.push(format!(
                "{:<18} {:<8} {}{needs}",
                s.feature.name, value, s.feature.prompt
            ));
        }
        r.next.push(
            "kitchen enable <feature> / kitchen disable <feature> / kitchen set <name>=<value>"
                .into(),
        );
        r.next
            .push("kitchen features --check   (do the answers work together?)".into());
        Ok(r)
    }

    /// `kitchen features --check`: check `kitchen.toml` against the rule table
    /// ([`compat`]). The summary says how many problems there are ("no problems" when
    /// none); every error comes with the commands that fix it.
    pub fn check_features(&self) -> Report {
        let mut found = compat::check(|n| self.manifest.is_on(n));
        found.extend(compat::check_models(|k| self.manifest.text(k)));
        let errors = found.iter().filter(|f| f.level == Level::Error).count();
        let mut r = Report::new(if errors == 0 {
            format!("{FILE}: no problems; these answers work together")
        } else {
            format!("{FILE}: {errors} problem(s); these answers do not work together")
        });
        for f in &found {
            r.lines.push(f.to_string());
            if let (Level::Error, Some(needs)) = (f.level, f.other) {
                r.next.push(format!(
                    "kitchen enable {} --with-deps   (turns on {needs}), or kitchen disable {}",
                    f.feature, f.feature
                ));
            }
        }
        r
    }

    /// Turn features on and off. Turning one on also turns on what it needs; turning one
    /// off also turns off what needs it.
    pub fn set_features(
        &self,
        enable: &[&str],
        disable: &[&str],
        opts: &Options,
    ) -> Result<Report> {
        let features = self.template()?.features()?;
        let mut next = self.manifest.clone();
        let deps = if opts.with_deps || opts.dry_run {
            Deps::Follow
        } else {
            Deps::Refuse
        };
        let mut notes = switch(&features, &mut next, enable, disable, deps, deps)?;
        if !opts.with_deps && !notes.is_empty() {
            notes.push(
                "(a dry run; add --with-deps to switch the dependencies listed above too, without it this stops)"
                    .into(),
            );
        }
        gate(&self.manifest, &next, &mut notes)?;
        self.reconfigure(next, None, opts, notes)
    }

    /// Change a text answer, e.g. `frontend=vanilla` or `embedding_model=bge-small-en-v1.5`.
    pub fn set_answer(&self, key: &str, value: &str, opts: &Options) -> Result<Report> {
        let features = self.template()?.features()?;
        let f = features.iter().find(|f| f.name == key).ok_or_else(|| {
            Error::Usage(format!(
                "no question {key:?}; `kitchen features` lists them"
            ))
        })?;
        if f.is_switch() {
            let on: bool = value
                .parse()
                .map_err(|_| Error::Usage(format!("{key} takes true or false")))?;
            let (enable, disable): (&[&str], &[&str]) =
                if on { (&[key], &[]) } else { (&[], &[key]) };
            return self.set_features(enable, disable, opts);
        }
        if let Some(parent) = f.requires.as_deref().filter(|p| !self.manifest.holds(p)) {
            return Err(Error::Usage(format!(
                "{key} applies only with {parent} on: `kitchen enable {parent}` first"
            )));
        }
        let v = {
            if !f.choices.is_empty() && !f.choices.iter().any(|c| c == value) {
                return Err(Error::Usage(format!(
                    "{key} is one of: {}",
                    f.choices.join(", ")
                )));
            }
            toml::Value::String(value.to_string())
        };
        let mut next = self.manifest.clone();
        next.set(key, v);
        fill_defaults(&features, &mut next);
        let mut notes = Vec::new();
        gate(&self.manifest, &next, &mut notes)?;
        self.reconfigure(next, None, opts, notes)
    }

    /// Move to a newer template (`rev`: a commit, tag or branch; `None` for the newest),
    /// keeping the app's own edits.
    pub fn upgrade(&self, rev: Option<&str>, opts: &Options) -> Result<Report> {
        let newer = Template::open(&self.manifest.template.source, Some(rev.unwrap_or("HEAD")))?;
        self.reconfigure(self.manifest.clone(), Some(newer), opts, Vec::new())
    }

    /// Render the template before and after, and merge the difference into the app.
    /// `notes` (dependencies switched, warnings, licence notices) come first in the report.
    fn reconfigure(
        &self,
        mut next: Manifest,
        newer: Option<Template>,
        opts: &Options,
        notes: Vec<String>,
    ) -> Result<Report> {
        let before = self.template()?;
        let after = newer.unwrap_or_else(|| before.clone());
        if next == self.manifest && after.rev == before.rev {
            return Ok(Report::new("Nothing to change."));
        }
        let tmp = crate::TempDir::new("render")?;
        let (b, n) = (tmp.path().join("base"), tmp.path().join("next"));
        std::fs::create_dir_all(&b)
            .and_then(|_| std::fs::create_dir_all(&n))
            .map_err(|e| Error::Io("create a scratch folder".into(), e))?;
        let base = before.render(&self.manifest, &b)?;
        let nxt = after.render(&next, &n)?;
        let plan = merge::plan(&base, &nxt, &self.root, &[FILE])?;
        let verb = if opts.dry_run {
            "Would change"
        } else {
            "Changed"
        };
        let mut r = Report::new(format!(
            "{verb} {} files{}",
            plan.changes.len(),
            if plan.conflicts() > 0 {
                format!(", {} to resolve by hand", plan.conflicts())
            } else {
                String::new()
            }
        ));
        if self.manifest.template.rev.is_empty() {
            r.next.push("This app does not record its template commit, so more files may show as merged; check them.".into());
        }
        r.lines.extend(notes);
        for c in &plan.changes {
            let a = serde_json::to_value(c.action)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            let note = if c.note.is_empty() {
                String::new()
            } else {
                format!("  ({})", c.note)
            };
            r.lines.push(format!("{a:<8} {}{note}", c.path.display()));
        }
        if !opts.dry_run {
            merge::apply(&plan, &base, &nxt, &self.root)?;
            next.template.rev = after.rev.clone();
            next.save(&self.root)?;
            if plan.changes.iter().any(|c| c.action == Action::Conflict) {
                r.next.push(
                    "Resolve the files marked conflict (<<<<<<< markers or .kitchen-new files)."
                        .into(),
                );
            }
            if self.root.join(".git").exists() {
                r.next.push("git diff        # review what changed".into());
            }
            r.next.push(format!(
                "kitchen start   # try it (a plain `cargo build` builds only desktop-core; the app is `cargo build -p {}`)",
                self.manifest.app.name
            ));
        } else if !plan.changes.is_empty() {
            r.next.push(
                "Nothing was changed. Run the same command without --dry-run to apply it.".into(),
            );
        }
        Ok(r)
    }
}

/// What to do when a switch needs another one changed ([`compat::REQUIRES`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deps {
    /// Change them too, and list them.
    Follow,
    /// Refuse, saying what it would take (`--with-deps`).
    Refuse,
}

/// Turn features on and off in `m`. What the enabled ones need (`on`) and what needs the
/// disabled ones (`off`) is switched too or refused, per [`Deps`]. Returns one line per
/// dependency switched.
fn switch<S: AsRef<str>>(
    features: &[Feature],
    m: &mut Manifest,
    enable: &[S],
    disable: &[S],
    on: Deps,
    off: Deps,
) -> Result<Vec<String>> {
    for name in enable.iter().chain(disable).map(AsRef::as_ref) {
        match features.iter().find(|f| f.name == name) {
            Some(f) if f.is_switch() => {}
            Some(_) => {
                return Err(Error::Usage(format!(
                    "{name} is not an on/off feature: use `kitchen set {name}=<value>`"
                )))
            }
            None => {
                return Err(Error::Usage(format!(
                    "no feature {name:?}; `kitchen features` lists them"
                )))
            }
        }
    }
    let enable: Vec<&str> = enable.iter().map(AsRef::as_ref).collect();
    let disable: Vec<&str> = disable.iter().map(AsRef::as_ref).collect();
    let needed = compat::missing_needs(&enable, |n| m.is_on(n));
    let dependents = compat::dependents(&disable, |n| m.is_on(n) && !enable.contains(&n));
    let why = |a: &str, b: &str| {
        compat::REQUIRES
            .iter()
            .find(|r| r.feature == a && r.needs == b)
            .map(|r| r.why)
            .unwrap_or_default()
    };
    let mut problems = Vec::new();
    if on == Deps::Refuse {
        for (need, by) in &needed {
            problems.push(format!(
                "{by} needs {need} ({}), which is off",
                why(by, need)
            ));
        }
    }
    if off == Deps::Refuse {
        for (dep, of) in &dependents {
            problems.push(format!(
                "{dep} needs {of} ({}), and {dep} is on",
                why(dep, of)
            ));
        }
    }
    if !problems.is_empty() {
        let mut fix = Vec::new();
        if on == Deps::Refuse && !needed.is_empty() {
            fix.push(format!(
                "kitchen enable {} --with-deps   (also turns on {})",
                enable.join(" "),
                needed
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if off == Deps::Refuse && !dependents.is_empty() {
            fix.push(format!(
                "kitchen disable {} --with-deps   (also turns off {})",
                disable.join(" "),
                dependents
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        return Err(Error::Usage(format!(
            "these features need others:\n  {}\nTo switch those too:\n  {}\n(add --dry-run to see the files first)",
            problems.join("\n  "),
            fix.join("\n  ")
        )));
    }
    let mut notes = Vec::new();
    for name in &enable {
        m.set(name, toml::Value::Boolean(true));
    }
    for (need, by) in &needed {
        m.set(need, toml::Value::Boolean(true));
        notes.push(format!(
            "also on   {need}  ({by} needs it: {})",
            why(by, need)
        ));
    }
    fill_defaults(features, m);
    for name in &disable {
        m.set(name, toml::Value::Boolean(false));
    }
    for (dep, of) in &dependents {
        m.set(dep, toml::Value::Boolean(false));
        notes.push(format!(
            "also off  {dep}  (it needs {of}: {})",
            why(dep, of)
        ));
    }
    Ok(notes)
}

/// Questions that only apply under a feature (or answer) that now holds start at their
/// defaults (`kitchen new` leaves them out while it does not). Repeated, since one filled
/// answer can open further questions (`models` opens `bundle_stt_model`).
fn fill_defaults(features: &[Feature], m: &mut Manifest) {
    loop {
        let fill: Vec<&Feature> = features
            .iter()
            .filter(|f| f.requires.as_ref().is_some_and(|r| m.holds(r)))
            .filter(|f| !m.answers.contains_key(&f.name))
            .collect();
        if fill.is_empty() {
            return;
        }
        for child in fill {
            m.set(&child.name, child.default.clone());
        }
    }
}

/// Check `after` against the rule table before anything is rendered: an error that
/// `before` did not have is refused; new warnings and licence notices go into `notes`.
fn gate(before: &Manifest, after: &Manifest, notes: &mut Vec<String>) -> Result<()> {
    let mut old = compat::check(|n| before.is_on(n));
    old.extend(compat::check_models(|k| before.text(k)));
    let mut new = compat::check(|n| after.is_on(n));
    new.extend(compat::check_models(|k| after.text(k)));
    let fresh: Vec<_> = new.iter().filter(|f| !old.contains(f)).collect();
    let errors: Vec<String> = fresh
        .iter()
        .filter(|f| f.level == Level::Error)
        .map(|f| format!("  {}", f.message))
        .collect();
    if !errors.is_empty() {
        return Err(Error::Usage(format!(
            "these answers do not work together:\n{}\nAdd --with-deps to switch what they need, or see `kitchen features --check`.",
            errors.join("\n")
        )));
    }
    notes.extend(fresh.iter().map(|f| f.to_string()));
    notes.extend(
        new.iter()
            .filter(|f| f.level == Level::Error)
            .map(|f| format!("{f} (already in {FILE}; `kitchen features --check`)")),
    );
    Ok(())
}

/// Forget the answers to questions whose feature is off (`background_agent` without
/// `ipc_worker`): they do nothing, so a new app does not record them as on. Turning the
/// feature on later fills them in with their defaults again ([`switch`]).
fn drop_unused_answers(features: &[Feature], m: &mut Manifest) {
    let unused: Vec<String> = features
        .iter()
        .filter(|f| f.requires.as_ref().is_some_and(|r| !m.holds(r)))
        .map(|f| f.name.clone())
        .collect();
    m.answers.retain(|k, _| !unused.contains(k));
}

/// What `kitchen new` needs.
#[derive(Debug, Clone, Default)]
pub struct NewApp {
    /// Folder and crate name, e.g. `notes`.
    pub name: String,
    /// Parent folder (default: the current one).
    pub dir: Option<PathBuf>,
    /// Template source (default: [`crate::template::DEFAULT_SOURCE`]).
    pub source: String,
    /// Template commit, tag or branch (default: newest).
    pub rev: Option<String>,
    /// Features to turn on.
    pub enable: Vec<String>,
    /// Features to turn off.
    pub disable: Vec<String>,
    /// Other answers, `key=value`.
    pub set: Vec<(String, String)>,
    /// Skip `git init`.
    pub no_git: bool,
    /// Also turn on what the enabled features need (`--with-deps`). Features only on by
    /// default that need a disabled one are turned off either way, as cargo-generate
    /// would not ask them.
    pub with_deps: bool,
}

/// Create an app from the template (`kitchen new`).
pub fn create(req: &NewApp) -> Result<Report> {
    let name = req.name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || name.starts_with(['-', '_'])
    {
        return Err(Error::Usage(format!(
            "{name:?} is not a usable app name: letters, digits, - and _"
        )));
    }
    let parent = match &req.dir {
        Some(d) => d.clone(),
        None => {
            std::env::current_dir().map_err(|e| Error::Io("find the current folder".into(), e))?
        }
    };
    let dest = parent.join(name);
    if dest.exists() {
        return Err(Error::Usage(format!("{} already exists", dest.display())));
    }
    let source = match Path::new(&req.source) {
        p if !req.source.is_empty() && p.is_dir() => p
            .canonicalize()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| req.source.clone()),
        _ => req.source.clone(),
    };
    let template = Template::open(&source, req.rev.as_deref())?;
    let features = template.features()?;
    let mut m = Manifest::default();
    m.app.name = name.to_string();
    m.template.source = source;
    m.template.rev = template.rev.clone();
    for f in &features {
        m.set(&f.name, f.default.clone());
    }
    let title: String = name
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect::<Vec<_>>()
        .join(" ");
    let ident: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase();
    m.set("app_name", toml::Value::String(title));
    m.set(
        "identifier",
        toml::Value::String(format!(
            "com.example.{}",
            if ident.is_empty() {
                "app".into()
            } else {
                ident
            }
        )),
    );
    m.set("github_repo", toml::Value::String(format!("owner/{name}")));
    let on = if req.with_deps {
        Deps::Follow
    } else {
        Deps::Refuse
    };
    let mut notes = switch(
        &features,
        &mut m,
        &req.enable,
        &req.disable,
        on,
        Deps::Follow,
    )?;
    for (k, v) in &req.set {
        let f = features
            .iter()
            .find(|f| &f.name == k)
            .ok_or_else(|| Error::Usage(format!("no question {k:?}")))?;
        let value = if f.is_switch() {
            toml::Value::Boolean(v == "true")
        } else {
            toml::Value::String(v.clone())
        };
        m.set(k, value);
    }
    drop_unused_answers(&features, &mut m);
    gate(&Manifest::default(), &m, &mut notes)?;
    std::fs::create_dir_all(&parent)
        .map_err(|e| Error::Io(format!("create {}", parent.display()), e))?;
    let out = template.render(&m, &parent)?;
    m.save(&out)?;
    let mut r = Report::new(format!("Created {}", out.display()));
    let on: Vec<&str> = features
        .iter()
        .filter(|f| f.is_switch() && m.is_on(&f.name))
        .map(|f| f.name.as_str())
        .collect();
    r.lines.push(if on.is_empty() {
        "features on: none yet (every feature starts off; `kitchen features` lists them)".into()
    } else {
        format!("features on: {}", on.join(", "))
    });
    r.lines.extend(notes);
    let agent: Vec<&str> = crate::doctor::AGENT_FILES
        .iter()
        .copied()
        .filter(|f| out.join(f).exists())
        .collect();
    if !agent.is_empty() {
        r.lines.push(format!("for AI agents: {}", agent.join(", ")));
    }
    if !req.no_git && crate::template::git(Some(&out), &["init", "--quiet"]).is_ok() {
        r.lines.push(first_commit(&out));
    }
    r.next = next_steps(name, out.join("AGENTS.md").is_file());
    Ok(r)
}

/// Commit the new app, so `git diff` shows exactly what a later `kitchen enable` or
/// `kitchen generate` changed. Without a git identity it stays uncommitted, and says so.
fn first_commit(dir: &Path) -> String {
    let git = |args: &[&str]| crate::template::git(Some(dir), args);
    match git(&["add", "--all"])
        .and_then(|_| git(&["commit", "--quiet", "--no-verify", "-m", "New app from Kitchen"]))
    {
        Ok(_) => "git repository initialised, with a first commit".into(),
        Err(_) => "git repository initialised (not committed: set git user.name and user.email, then commit)".into(),
    }
}

/// What to run after `kitchen new`, in order, for a web developer seeing it the first time.
fn next_steps(name: &str, agents_md: bool) -> Vec<String> {
    let mut next = vec![
        format!("cd {name}"),
        "kitchen start      # installs the UI's npm packages, builds (a few minutes the first time) and opens the window".into(),
        "kitchen features   # what else you can turn on: kitchen enable <feature>".into(),
        "kitchen doctor     # if something is missing on this machine, it says how to fix it".into(),
    ];
    if agents_md {
        next.push("cat AGENTS.md      # what agents (and people) read first".into());
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_steps_start_the_app_first() {
        let n = next_steps("notes", true);
        assert_eq!(n[0], "cd notes");
        assert!(n[1].starts_with("kitchen start "), "{n:?}");
        assert!(n.iter().all(|l| !l.starts_with("kitchen dev")), "{n:?}");
        assert!(n.last().unwrap().starts_with("cat AGENTS.md"));
        assert!(!next_steps("notes", false)
            .iter()
            .any(|l| l.contains("AGENTS")));
    }

    fn feature(name: &str, requires: Option<&str>) -> Feature {
        Feature {
            name: name.into(),
            prompt: String::new(),
            default: toml::Value::Boolean(requires.is_some()),
            choices: Vec::new(),
            requires: requires.map(String::from),
        }
    }

    #[test]
    fn answers_under_an_off_feature_are_dropped_and_come_back_on_enable() {
        let features = [
            feature("ipc_worker", None),
            feature("background_agent", Some("ipc_worker")),
        ];
        let mut m = Manifest::default();
        for f in &features {
            m.set(&f.name, f.default.clone());
        }
        drop_unused_answers(&features, &mut m);
        assert!(!m.answers.contains_key("background_agent"));
        let refuse = Deps::Refuse;
        switch(
            &features,
            &mut m,
            &["ipc_worker"],
            &[] as &[&str],
            refuse,
            refuse,
        )
        .unwrap();
        assert!(m.is_on("ipc_worker") && m.is_on("background_agent"));
        // An explicit answer is kept.
        m.set("background_agent", toml::Value::Boolean(false));
        switch(
            &features,
            &mut m,
            &["ipc_worker"],
            &[] as &[&str],
            refuse,
            refuse,
        )
        .unwrap();
        assert!(!m.is_on("background_agent"));
    }

    #[test]
    fn first_commit_makes_a_commit_or_says_why_not() {
        let d = crate::TempDir::new("first-commit").unwrap();
        std::fs::write(d.path().join("a.txt"), "x").unwrap();
        crate::template::git(Some(d.path()), &["init", "--quiet"]).unwrap();
        // A fixed identity for this repository only, so the test does not depend on the machine.
        for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com")] {
            crate::template::git(Some(d.path()), &["config", k, v]).unwrap();
        }
        assert_eq!(
            first_commit(d.path()),
            "git repository initialised, with a first commit"
        );
        let log = crate::template::git(Some(d.path()), &["log", "--format=%s"]).unwrap();
        assert_eq!(log, "New app from Kitchen");
    }

    fn features() -> Vec<Feature> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../template/cargo-generate.toml");
        crate::template::parse_features(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn manifest(on: &[&str]) -> Manifest {
        let mut m = Manifest::default();
        for f in features().iter().filter(|f| f.is_switch()) {
            m.set(&f.name, toml::Value::Boolean(on.contains(&f.name.as_str())));
        }
        m
    }

    #[test]
    fn enable_refuses_missing_needs_unless_with_deps() {
        let f = features();
        let mut m = manifest(&[]);
        let e = switch(&f, &mut m, &["daemon"], &[], Deps::Refuse, Deps::Refuse)
            .unwrap_err()
            .to_string();
        assert!(e.contains("daemon needs ipc_worker"), "{e}");
        assert!(e.contains("kitchen enable daemon --with-deps"), "{e}");
        assert!(!m.is_on("daemon"), "nothing changes when refused");

        let notes = switch(&f, &mut m, &["daemon"], &[], Deps::Follow, Deps::Refuse).unwrap();
        assert!(m.is_on("daemon") && m.is_on("ipc_worker"));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("also on   ipc_worker"), "{notes:?}");

        // Already there: nothing to add.
        let mut m = manifest(&["postgres"]);
        assert!(
            switch(&f, &mut m, &["bm25"], &[], Deps::Refuse, Deps::Refuse)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn disable_refuses_dependents_unless_with_deps() {
        let f = features();
        let mut m = manifest(&["ipc_worker", "daemon", "background_agent"]);
        let e = switch(&f, &mut m, &[], &["ipc_worker"], Deps::Refuse, Deps::Refuse)
            .unwrap_err()
            .to_string();
        assert!(e.contains("daemon needs ipc_worker"), "{e}");
        assert!(e.contains("--with-deps"), "{e}");
        let notes = switch(&f, &mut m, &[], &["ipc_worker"], Deps::Refuse, Deps::Follow).unwrap();
        assert!(!m.is_on("daemon") && !m.is_on("background_agent") && !m.is_on("ipc_worker"));
        assert_eq!(notes.len(), 2, "{notes:?}");
        // Turning one on and its need off at once is refused, not silently undone.
        let mut m = manifest(&["ipc_worker"]);
        switch(
            &f,
            &mut m,
            &["daemon"],
            &["ipc_worker"],
            Deps::Follow,
            Deps::Follow,
        )
        .unwrap();
        assert!(gate(&manifest(&["ipc_worker"]), &m, &mut Vec::new()).is_err());
    }

    #[test]
    fn gate_refuses_new_errors_and_reports_warnings_and_licences() {
        let ok = manifest(&["postgres"]);
        let mut notes = Vec::new();
        let e = gate(&ok, &manifest(&["postgres", "daemon"]), &mut notes)
            .unwrap_err()
            .to_string();
        assert!(e.contains("daemon needs ipc_worker"), "{e}");

        let mut notes = Vec::new();
        gate(&ok, &manifest(&["postgres", "bm25", "mobile"]), &mut notes).unwrap();
        assert!(
            notes.iter().any(|n| n.starts_with("licence: bm25")),
            "{notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with("warning: postgres is desktop only")),
            "{notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.contains("licence: postgres")),
            "already on: {notes:?}"
        );

        // A problem the app already had does not block an unrelated change.
        let bad = manifest(&["daemon"]);
        let mut notes = Vec::new();
        gate(&bad, &manifest(&["daemon", "email"]), &mut notes).unwrap();
        assert!(
            notes.iter().any(|n| n.contains("already in kitchen.toml")),
            "{notes:?}"
        );
    }
}
