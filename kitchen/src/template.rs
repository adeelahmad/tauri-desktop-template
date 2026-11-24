//! The template: where it comes from, which features it offers, and rendering it.
//!
//! A template source is a git URL (cloned once into the cache, then fetched) or a local
//! folder. A local folder without a commit is used as it is, which is how you work on the
//! template itself. With a commit, that exact commit is checked out into the cache, so an
//! app can always be compared with the template it was made from.
//!
//! Rendering runs `cargo generate` (install it with `cargo install cargo-generate`).
//!
//! ```no_run
//! use kitchen::template::Template;
//!
//! let t = Template::open("../tauri-desktop-template", None)?;
//! for f in t.features()? {
//!     println!("{:<16} {}", f.name, f.prompt);
//! }
//! # Ok::<(), kitchen::Error>(())
//! ```

use crate::manifest::Manifest;
use crate::{io, Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The template's home.
pub const DEFAULT_SOURCE: &str = "https://github.com/adeelahmad/tauri-desktop-template";

/// One question the template asks (a cargo-generate placeholder).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Feature {
    /// Placeholder name, e.g. `email`.
    pub name: String,
    /// The question, as `cargo generate` asks it.
    pub prompt: String,
    /// `true`/`false` for yes/no questions, else the default text.
    pub default: toml::Value,
    /// Allowed values of a text question.
    pub choices: Vec<String>,
    /// A yes/no question that only applies when this one is on (`bm25` needs `postgres`).
    pub requires: Option<String>,
}

impl Feature {
    /// Whether this is a yes/no question (a feature you can enable or disable).
    pub fn is_switch(&self) -> bool {
        self.default.is_bool()
    }
}

/// A template checked out on disk.
#[derive(Debug, Clone)]
pub struct Template {
    /// The folder holding `cargo-generate.toml`.
    pub root: PathBuf,
    /// Where it came from (as given).
    pub source: String,
    /// The commit, or empty for a folder that is not a git checkout.
    pub rev: String,
}

impl Template {
    /// Open `source` (git URL or folder) at `rev` (a commit, tag or branch; `None` for the
    /// newest, or for a local folder as it is on disk).
    pub fn open(source: &str, rev: Option<&str>) -> Result<Self> {
        let source = if source.is_empty() { DEFAULT_SOURCE } else { source };
        let local = Path::new(source);
        let rev = rev.filter(|r| !r.is_empty());
        if local.is_dir() && rev.is_none() {
            let head = git(Some(local), &["rev-parse", "HEAD"]).unwrap_or_default();
            return Ok(Self { root: template_root(local)?, source: source.to_string(), rev: head });
        }
        let mirror = cache_dir()?.join("repos").join(slug(source));
        if !mirror.join("HEAD").is_file() {
            io("create the cache", std::fs::create_dir_all(mirror.parent().unwrap_or(&mirror)))?;
            git(None, &["clone", "--bare", "--quiet", source, &mirror.to_string_lossy()])?;
        }
        let want = rev.unwrap_or("HEAD");
        let resolve = |m: &Path| git(Some(m), &["rev-parse", "--verify", "--quiet", &format!("{want}^{{commit}}")]);
        let sha = match (rev, resolve(&mirror)) {
            (Some(_), Ok(sha)) => sha, // a known commit never changes
            _ => {
                git(Some(&mirror), &["fetch", "--quiet", "--prune", "origin", "+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"])?;
                resolve(&mirror).map_err(|_| Error::Usage(format!("{want} is not in {source}")))?
            }
        };
        let tree = cache_dir()?.join("trees").join(&sha);
        if !tree.is_dir() {
            let tmp = tree.with_extension(format!("tmp{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&tmp);
            io("create the cache", std::fs::create_dir_all(&tmp))?;
            let index = tmp.with_extension("index");
            let out = Command::new("git")
                .arg(format!("--git-dir={}", mirror.display()))
                .arg(format!("--work-tree={}", tmp.display()))
                .args(["checkout", &sha, "--", "."])
                .env("GIT_INDEX_FILE", &index)
                .output();
            let _ = std::fs::remove_file(&index);
            check("git checkout", out)?;
            io("fill the cache", std::fs::rename(&tmp, &tree))?;
        }
        Ok(Self { root: template_root(&tree)?, source: source.to_string(), rev: sha })
    }

    /// Every question in `cargo-generate.toml`, in file order.
    pub fn features(&self) -> Result<Vec<Feature>> {
        let path = self.root.join("cargo-generate.toml");
        let text = io(format!("read {}", path.display()), std::fs::read_to_string(&path))?;
        parse_features(&text)
    }

    /// Render the app for `manifest`'s answers into `<parent>/<name>`, and return that
    /// folder. Answers the template does not ask (or that depend on a feature that is
    /// off) are left out.
    pub fn render(&self, manifest: &Manifest, parent: &Path) -> Result<PathBuf> {
        let features = self.features()?;
        let mut m = manifest.clone();
        m.answers.retain(|k, _| features.iter().any(|f| &f.name == k));
        let off: Vec<String> =
            features.iter().filter(|f| f.requires.as_ref().is_some_and(|r| !m.is_on(r))).map(|f| f.name.clone()).collect();
        m.answers.retain(|k, _| !off.contains(k));
        let name = if manifest.app.name.is_empty() { "app" } else { manifest.app.name.as_str() };
        let out = Command::new("cargo")
            .args(["generate", "--path"])
            .arg(&self.root)
            .args(["--name", name, "--destination"])
            .arg(parent)
            .args(["--silent", "--vcs", "none", "--force"])
            .args(m.cargo_generate_args())
            // Not inside the template: its `.cargo/config.toml` is Liquid, not TOML.
            .current_dir(parent)
            .output();
        check("cargo generate (install it with `cargo install cargo-generate`)", out)?;
        let dir = parent.join(name);
        if dir.is_dir() {
            Ok(dir)
        } else {
            Err(Error::Tool(format!("cargo generate did not create {}", dir.display())))
        }
    }
}

/// Parse the questions out of a `cargo-generate.toml`.
pub fn parse_features(text: &str) -> Result<Vec<Feature>> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| Error::Usage(format!("cargo-generate.toml: {e}")))?;
    let mut out = Vec::new();
    let mut add = |name: &str, spec: &toml::Value, requires: Option<String>| {
        let s = |k: &str| spec.get(k).and_then(toml::Value::as_str).unwrap_or_default().to_string();
        let default = spec.get("default").cloned().unwrap_or_else(|| toml::Value::String(String::new()));
        let choices = spec.get("choices").and_then(toml::Value::as_array).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        out.push(Feature { name: name.to_string(), prompt: s("prompt"), default, choices, requires });
    };
    if let Some(p) = doc.get("placeholders").and_then(toml::Value::as_table) {
        for (name, spec) in p {
            add(name, spec, None);
        }
    }
    if let Some(c) = doc.get("conditional").and_then(toml::Value::as_table) {
        for (cond, body) in c {
            // Only simple `feature` conditions carry questions in this template.
            let simple = cond.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if let (true, Some(p)) = (simple, body.get("placeholders").and_then(toml::Value::as_table)) {
                for (name, spec) in p {
                    add(name, spec, Some(cond.clone()));
                }
            }
        }
    }
    Ok(out)
}

/// The folder with `cargo-generate.toml`: `dir` itself or its `template/`.
fn template_root(dir: &Path) -> Result<PathBuf> {
    [dir.to_path_buf(), dir.join("template")]
        .into_iter()
        .find(|d| d.join("cargo-generate.toml").is_file())
        .ok_or_else(|| Error::Usage(format!("{} is not a Kitchen template (no cargo-generate.toml)", dir.display())))
}

/// `$KITCHEN_CACHE`, else the platform cache folder + `kitchen`.
pub fn cache_dir() -> Result<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("KITCHEN_CACHE")
        .or_else(|| var("XDG_CACHE_HOME").map(|d| d.join("kitchen")))
        .or_else(|| var("LOCALAPPDATA").map(|d| d.join("kitchen")))
        .or_else(|| var("HOME").map(|d| d.join(if cfg!(target_os = "macos") { "Library/Caches/kitchen" } else { ".cache/kitchen" })))
        .ok_or_else(|| Error::Usage("no cache folder: set KITCHEN_CACHE".into()))
}

/// A file-name-safe name for a source.
fn slug(source: &str) -> String {
    let s: String = source.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    s.trim_matches('_').to_string()
}

/// Run git and return its trimmed output.
pub(crate) fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut c = Command::new("git");
    if let Some(d) = dir {
        c.arg("-C").arg(d);
    }
    check("git", c.args(args).output())
}

/// Turn a finished process into its stdout, or a readable error.
pub(crate) fn check(what: &str, out: std::io::Result<std::process::Output>) -> Result<String> {
    let out = out.map_err(|e| Error::Tool(format!("{what}: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(Error::Tool(format!("{what} failed: {}", err.trim())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_questions_and_dependencies() {
        let f = parse_features(
            r#"
            [template]
            exclude = []
            [placeholders.postgres]
            type = "bool"
            prompt = "Postgres?"
            default = true
            [placeholders.frontend]
            type = "string"
            choices = ["vanilla", "react"]
            default = "react"
            [conditional.'postgres'.placeholders.bm25]
            type = "bool"
            default = false
            [conditional.'!postgres']
            ignore = ["x"]
            "#,
        )
        .unwrap();
        assert_eq!(f.len(), 3);
        let bm25 = f.iter().find(|x| x.name == "bm25").unwrap();
        assert_eq!(bm25.requires.as_deref(), Some("postgres"));
        assert!(bm25.is_switch());
        let fe = f.iter().find(|x| x.name == "frontend").unwrap();
        assert_eq!(fe.choices, ["vanilla", "react"]);
        assert!(!fe.is_switch());
    }

    #[test]
    fn slugs_are_safe() {
        assert_eq!(slug("https://github.com/a/b"), "https___github.com_a_b");
    }
}
