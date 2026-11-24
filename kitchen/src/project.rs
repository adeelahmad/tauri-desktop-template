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
//! println!("{}", app.set_features(&["tts"], &[], &Options { dry_run: true })?);
//! # Ok::<(), kitchen::Error>(())
//! ```

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
            let needs = s
                .feature
                .requires
                .as_ref()
                .map(|p| format!(" (needs {p})"))
                .unwrap_or_default();
            r.lines.push(format!(
                "{:<18} {:<8} {}{needs}",
                s.feature.name, value, s.feature.prompt
            ));
        }
        r.next.push(
            "kitchen enable <feature> / kitchen disable <feature> / kitchen set <name>=<value>"
                .into(),
        );
        Ok(r)
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
        switch(&features, &mut next, enable, disable)?;
        self.reconfigure(next, None, opts)
    }

    /// Change a text answer, e.g. `frontend=vanilla` or `embedding_model=bge-small-en-v1.5`.
    pub fn set_answer(&self, key: &str, value: &str, opts: &Options) -> Result<Report> {
        let features = self.template()?.features()?;
        let f = features.iter().find(|f| f.name == key).ok_or_else(|| {
            Error::Usage(format!(
                "no question {key:?}; `kitchen features` lists them"
            ))
        })?;
        let v = if f.is_switch() {
            toml::Value::Boolean(
                value
                    .parse()
                    .map_err(|_| Error::Usage(format!("{key} takes true or false")))?,
            )
        } else {
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
        self.reconfigure(next, None, opts)
    }

    /// Move to a newer template (`rev`: a commit, tag or branch; `None` for the newest),
    /// keeping the app's own edits.
    pub fn upgrade(&self, rev: Option<&str>, opts: &Options) -> Result<Report> {
        let newer = Template::open(&self.manifest.template.source, Some(rev.unwrap_or("HEAD")))?;
        self.reconfigure(self.manifest.clone(), Some(newer), opts)
    }

    /// Render the template before and after, and merge the difference into the app.
    fn reconfigure(
        &self,
        mut next: Manifest,
        newer: Option<Template>,
        opts: &Options,
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

/// Turn features on (with what they need) and off (with what needs them) in `m`.
fn switch<S: AsRef<str>>(
    features: &[Feature],
    m: &mut Manifest,
    enable: &[S],
    disable: &[S],
) -> Result<()> {
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
    for name in enable.iter().map(AsRef::as_ref) {
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            m.set(&n, toml::Value::Boolean(true));
            // Questions that only apply under this one start at their defaults.
            for child in features
                .iter()
                .filter(|f| f.requires.as_deref() == Some(&n))
            {
                if !m.answers.contains_key(&child.name) {
                    m.set(&child.name, child.default.clone());
                }
            }
            cur = features
                .iter()
                .find(|f| f.name == n)
                .and_then(|f| f.requires.clone());
        }
    }
    for name in disable.iter().map(AsRef::as_ref) {
        let mut off = vec![name.to_string()];
        while let Some(n) = off.pop() {
            m.set(&n, toml::Value::Boolean(false));
            off.extend(
                features
                    .iter()
                    .filter(|f| f.requires.as_deref() == Some(&n) && f.is_switch())
                    .map(|f| f.name.clone()),
            );
        }
    }
    Ok(())
}

/// Forget the answers to questions whose feature is off (`background_agent` without
/// `ipc_worker`): they do nothing, so a new app does not record them as on. Turning the
/// feature on later fills them in with their defaults again ([`switch`]).
fn drop_unused_answers(features: &[Feature], m: &mut Manifest) {
    let unused: Vec<String> = features
        .iter()
        .filter(|f| f.requires.as_ref().is_some_and(|r| !m.is_on(r)))
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
    switch(&features, &mut m, &req.enable, &req.disable)?;
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
        switch(&features, &mut m, &["ipc_worker"], &[] as &[&str]).unwrap();
        assert!(m.is_on("ipc_worker") && m.is_on("background_agent"));
        // An explicit answer is kept.
        m.set("background_agent", toml::Value::Boolean(false));
        switch(&features, &mut m, &["ipc_worker"], &[] as &[&str]).unwrap();
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
}
