//! `kitchen.toml`: the answers an app was generated with, and the template it came from.
//!
//! The template writes this file into every app, so apps made with plain
//! `cargo generate` work with `kitchen` too. `kitchen new` also records the template's
//! source and commit, which lets later changes be merged exactly (see [`crate::merge`]).
//!
//! ```
//! use kitchen::manifest::Manifest;
//!
//! let mut m: Manifest = toml::from_str(r#"
//!     [app]
//!     name = "notes"
//!     [answers]
//!     email = false
//!     frontend = "react"
//! "#).unwrap();
//! assert!(!m.is_on("email"));
//! m.set("email", toml::Value::Boolean(true));
//! assert!(m.is_on("email"));
//! assert_eq!(m.cargo_generate_args(), ["-d", "email=true", "-d", "frontend=react"]);
//! ```

use crate::{io, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file name, at the app's root.
pub const FILE: &str = "kitchen.toml";

const HEADER: &str =
    "# Kitchen project file: the answers this app was generated with. The `kitchen` CLI reads it
# to turn features on and off (`kitchen enable email`) and to upgrade from the template.
# Edit it through `kitchen`, not by hand.

";

/// The whole file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The app's crate and folder name.
    #[serde(default)]
    pub app: App,
    /// Where the template came from.
    #[serde(default)]
    pub template: TemplateRef,
    /// Generation answers, by placeholder name (`email = true`, `frontend = "react"`).
    #[serde(default)]
    pub answers: BTreeMap<String, toml::Value>,
}

/// `[app]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct App {
    /// The name given to `cargo generate --name` (the folder and crate name).
    #[serde(default)]
    pub name: String,
}

/// `[template]`: a git URL or local path, and a commit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TemplateRef {
    /// Git URL or local folder; empty means [`crate::template::DEFAULT_SOURCE`].
    #[serde(default)]
    pub source: String,
    /// The commit the app was generated from; empty when unknown.
    #[serde(default)]
    pub rev: String,
}

impl Manifest {
    /// Read `<dir>/kitchen.toml`.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        let text = io(
            format!("read {}", path.display()),
            std::fs::read_to_string(&path),
        )?;
        toml::from_str(&text).map_err(|e| crate::Error::Usage(format!("{}: {e}", path.display())))
    }

    /// Write `<dir>/kitchen.toml` (with its explanatory header).
    pub fn save(&self, dir: &Path) -> Result<()> {
        let text = toml::to_string(self).map_err(|e| crate::Error::Usage(e.to_string()))?;
        let path = dir.join(FILE);
        io(
            format!("write {}", path.display()),
            std::fs::write(&path, format!("{HEADER}{text}")),
        )
    }

    /// The nearest folder at or above `start` that has a `kitchen.toml`.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        let start = start.canonicalize().ok()?;
        start
            .ancestors()
            .find(|d| d.join(FILE).is_file())
            .map(Path::to_path_buf)
    }

    /// Whether a yes/no answer is `true`.
    pub fn is_on(&self, key: &str) -> bool {
        self.answers
            .get(key)
            .and_then(toml::Value::as_bool)
            .unwrap_or(false)
    }

    /// One answer as text (`true`/`false` for yes/no answers), `None` when it has none.
    pub fn text(&self, key: &str) -> Option<String> {
        self.answers.get(key).map(|v| match v {
            toml::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    /// Whether a question's condition from `cargo-generate.toml` holds for these answers:
    /// `name` (a yes/no answer that is on), `!name`, `name == "v"`, `name != "v"`, joined
    /// by `&&` or `||` (not both). Like cargo-generate, a condition that reads an answer
    /// the app does not have is false (unless an `||` already made it true).
    pub fn holds(&self, cond: &str) -> bool {
        let atom = |a: &str| -> Option<bool> {
            let a = a
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim();
            for (op, eq) in [("!=", false), ("==", true)] {
                if let Some((k, v)) = a.split_once(op) {
                    let got = self.text(k.trim())?;
                    return Some((got == v.trim().trim_matches('"')) == eq);
                }
            }
            match a.strip_prefix('!') {
                Some(k) => self.answers.get(k.trim())?.as_bool().map(|b| !b),
                None => self.answers.get(a)?.as_bool(),
            }
        };
        if cond.contains("||") {
            for a in cond.split("||") {
                match atom(a) {
                    Some(true) => return true,
                    Some(false) => {}
                    None => return false,
                }
            }
            false
        } else {
            cond.split("&&").all(|a| atom(a) == Some(true))
        }
    }

    /// Set one answer.
    pub fn set(&mut self, key: &str, value: toml::Value) {
        self.answers.insert(key.to_string(), value);
    }

    /// The answers as `cargo generate` arguments: `-d key=value` pairs, sorted by key.
    pub fn cargo_generate_args(&self) -> Vec<String> {
        self.answers
            .iter()
            .flat_map(|(k, v)| {
                let v = match v {
                    toml::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                ["-d".to_string(), format!("{k}={v}")]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn conditions_hold_like_cargo_generate() {
        let m: Manifest =
            toml::from_str("[answers]\nmodels = \"bundle\"\nstt = true\ntts = false\n").unwrap();
        assert!(m.holds("stt"));
        assert!(m.holds("models != \"download\" && stt"));
        assert!(!m.holds("models != \"download\" && tts"));
        assert!(!m.holds("models == \"download\""));
        assert!(m.holds("tts || stt"));
        assert!(m.holds("!tts"));
        // An answer the app does not have makes the condition false, as in cargo-generate.
        assert!(!m.holds("local_llm"));
        assert!(!m.holds("tts || local_llm || stt"));
        assert!(m.holds("stt || local_llm"));
    }

    use super::*;

    #[test]
    fn round_trip_and_find_root() {
        let dir = crate::TempDir::new("manifest").unwrap();
        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        let mut m = Manifest {
            app: App { name: "x".into() },
            ..Manifest::default()
        };
        m.set("ai", toml::Value::Boolean(true));
        m.save(dir.path()).unwrap();
        assert_eq!(Manifest::load(dir.path()).unwrap(), m);
        assert_eq!(
            Manifest::find_root(&nested).unwrap(),
            dir.path().canonicalize().unwrap()
        );
        assert!(std::fs::read_to_string(dir.path().join(FILE))
            .unwrap()
            .starts_with("# Kitchen"));
    }
}
