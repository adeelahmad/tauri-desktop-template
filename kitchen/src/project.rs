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
            r.next
                .push("cargo build (or `kitchen dev`) to check the result.".into());
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
    r.lines.push(format!("features on: {}", on.join(", ")));
    if !req.no_git && crate::template::git(Some(&out), &["init", "--quiet"]).is_ok() {
        r.lines.push("git repository initialised".into());
    }
    r.next = vec![
        format!("cd {name}"),
        "kitchen doctor     # check this machine".into(),
        "kitchen dev        # run it".into(),
    ];
    Ok(r)
}
