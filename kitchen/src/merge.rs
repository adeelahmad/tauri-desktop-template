//! Apply a template change to an app without losing the app's own edits.
//!
//! Three trees: **base** (the template as the app was made), **next** (the template as it
//! should be now) and **ours** (the app). For every file the template touches:
//!
//! | base → next | ours           | result                                             |
//! |-------------|----------------|----------------------------------------------------|
//! | added       | missing        | create                                             |
//! | added       | different      | conflict: next is written beside it as `.kitchen-new` |
//! | removed     | same as base   | delete                                             |
//! | removed     | edited         | keep (listed)                                      |
//! | changed     | same as base   | update                                             |
//! | changed     | edited         | merge with `git merge-file`; clashes get markers  |
//! | changed     | deleted        | keep deleted (listed)                              |
//!
//! Files the template does not touch are never read or written.
//!
//! ```
//! use kitchen::merge::{plan, Action};
//! # let t = std::env::temp_dir().join(format!("kitchen-doc-{}", std::process::id()));
//! # let (base, next, ours) = (t.join("b"), t.join("n"), t.join("o"));
//! # for d in [&base, &next, &ours] { std::fs::create_dir_all(d).unwrap(); }
//! std::fs::write(base.join("a.txt"), "1").unwrap();
//! std::fs::write(next.join("a.txt"), "2").unwrap();
//! std::fs::write(ours.join("a.txt"), "1").unwrap();
//! let p = plan(&base, &next, &ours, &[]).unwrap();
//! assert_eq!(p.changes[0].action, Action::Update);
//! # std::fs::remove_dir_all(&t).unwrap();
//! ```

use crate::{io, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What happens to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// New file from the template.
    Create,
    /// Untouched by you: replaced with the template's new version.
    Update,
    /// Untouched by you, and the template no longer has it.
    Delete,
    /// Changed by both: merged cleanly.
    Merge,
    /// Changed by both and they clash: conflict markers (text) or a `.kitchen-new` file.
    Conflict,
    /// Left as it is (you edited or deleted it); see the note.
    Keep,
}

/// One planned file change.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Change {
    /// Path relative to the app.
    pub path: PathBuf,
    /// What happens.
    pub action: Action,
    /// Why, when it is not obvious.
    pub note: String,
}

/// Every change, sorted by path.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Plan {
    /// The changes.
    pub changes: Vec<Change>,
}

impl Plan {
    /// Number of conflicts.
    pub fn conflicts(&self) -> usize {
        self.changes.iter().filter(|c| c.action == Action::Conflict).count()
    }
}

/// Work out the changes. `skip` lists relative paths to leave alone (e.g. `kitchen.toml`).
/// Merges are tried here (in memory) so the plan says which ones clash.
pub fn plan(base: &Path, next: &Path, ours: &Path, skip: &[&str]) -> Result<Plan> {
    let mut paths = BTreeSet::new();
    files(base, base, &mut paths)?;
    files(next, next, &mut paths)?;
    let mut changes = Vec::new();
    for rel in paths {
        if skip.iter().any(|s| Path::new(s) == rel) {
            continue;
        }
        let read = |root: &Path| std::fs::read(root.join(&rel)).ok();
        let (b, n, o) = (read(base), read(next), read(ours));
        let change = |action, note: &str| Some(Change { path: rel.clone(), action, note: note.to_string() });
        let c = match (&b, &n, &o) {
            _ if b == n => None,
            (_, Some(_), _) if o == n => None,
            (None, Some(_), None) => change(Action::Create, ""),
            (None, Some(_), Some(_)) => change(Action::Conflict, "you already have this file; the template's version is written beside it as .kitchen-new"),
            (Some(_), None, None) => None,
            (Some(_), None, Some(_)) if o == b => change(Action::Delete, ""),
            (Some(_), None, Some(_)) => change(Action::Keep, "the template removed it, but you edited it"),
            (Some(_), Some(_), None) => change(Action::Keep, "you deleted it; the template changed it"),
            (Some(_), Some(_), Some(_)) if o == b => change(Action::Update, ""),
            (Some(bb), Some(nn), Some(oo)) => match merge3(oo, bb, nn) {
                Some((_, 0)) => change(Action::Merge, "merged your edits with the template's"),
                Some(_) => change(Action::Conflict, "both changed the same lines: look for <<<<<<< markers"),
                None => change(Action::Conflict, "cannot merge (binary, or git missing): the template's version is beside it as .kitchen-new"),
            },
            (None, None, _) => None,
        };
        changes.extend(c);
    }
    Ok(Plan { changes })
}

/// Carry out a plan made from the same three trees.
pub fn apply(plan: &Plan, base: &Path, next: &Path, ours: &Path) -> Result<()> {
    for c in &plan.changes {
        let dst = ours.join(&c.path);
        let src = next.join(&c.path);
        if let Some(parent) = dst.parent() {
            io(format!("create {}", parent.display()), std::fs::create_dir_all(parent))?;
        }
        match c.action {
            Action::Create | Action::Update => {
                io(format!("write {}", dst.display()), std::fs::copy(&src, &dst))?;
            }
            Action::Delete => {
                io(format!("remove {}", dst.display()), std::fs::remove_file(&dst))?;
                remove_empty_parents(&dst, ours);
            }
            Action::Merge | Action::Conflict => {
                let read = |p: &Path| std::fs::read(p).unwrap_or_default();
                let merged = if base.join(&c.path).is_file() { merge3(&read(&dst), &read(&base.join(&c.path)), &read(&src)) } else { None };
                match merged {
                    Some((text, _)) => io(format!("write {}", dst.display()), std::fs::write(&dst, text))?,
                    None => {
                        let side = PathBuf::from(format!("{}.kitchen-new", dst.display()));
                        io(format!("write {}", side.display()), std::fs::copy(&src, &side))?;
                    }
                }
            }
            Action::Keep => {}
        }
    }
    Ok(())
}

/// Three-way merge of text with `git merge-file`: the result and its number of conflicts.
/// `None` for binary files or when git is missing.
fn merge3(ours: &[u8], base: &[u8], theirs: &[u8]) -> Option<(Vec<u8>, i32)> {
    if [ours, base, theirs].iter().any(|b| b.contains(&0)) {
        return None;
    }
    let tmp = crate::TempDir::new("merge").ok()?;
    let p = |n: &str| tmp.path().join(n);
    std::fs::write(p("ours"), ours).ok()?;
    std::fs::write(p("base"), base).ok()?;
    std::fs::write(p("next"), theirs).ok()?;
    let out = std::process::Command::new("git")
        .args(["merge-file", "-p", "-L", "yours", "-L", "template before", "-L", "template now"])
        .args([p("ours"), p("base"), p("next")])
        .output()
        .ok()?;
    // Exit code: 0 clean, 1..127 the number of conflicts, anything else an error.
    match out.status.code() {
        Some(n @ 0..=127) => Some((out.stdout, n)),
        _ => None,
    }
}

/// Every file under `dir`, relative to `root`.
fn files(root: &Path, dir: &Path, out: &mut BTreeSet<PathBuf>) -> Result<()> {
    for e in io(format!("read {}", dir.display()), std::fs::read_dir(dir))? {
        let e = io("read a folder entry", e)?;
        let path = e.path();
        let t = io(format!("inspect {}", path.display()), e.file_type())?;
        if t.is_dir() {
            files(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.insert(rel.to_path_buf());
        }
    }
    Ok(())
}

fn remove_empty_parents(file: &Path, stop: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == stop || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path, files: &[(&str, &str)]) {
        std::fs::create_dir_all(root).unwrap();
        for (p, c) in files {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, c).unwrap();
        }
    }

    #[test]
    fn plans_and_applies_every_case() {
        let t = crate::TempDir::new("merge-test").unwrap();
        let (b, n, o) = (t.path().join("b"), t.path().join("n"), t.path().join("o"));
        tree(&b, &[("same.txt", "x"), ("upd.txt", "1"), ("gone.txt", "g"), ("gone-edited.txt", "g"), ("both.txt", "a\nb\nc\nd\ne\n"), ("clash.txt", "a\n"), ("deleted.txt", "1"), ("kitchen.toml", "a")]);
        tree(&n, &[("same.txt", "x"), ("upd.txt", "2"), ("new/file.txt", "n"), ("exists.txt", "t"), ("both.txt", "a\nB\nc\nd\ne\n"), ("clash.txt", "T\n"), ("deleted.txt", "2"), ("kitchen.toml", "b")]);
        tree(&o, &[("same.txt", "x"), ("upd.txt", "1"), ("gone.txt", "g"), ("gone-edited.txt", "mine"), ("exists.txt", "mine"), ("both.txt", "a\nb\nc\nd\nE\n"), ("clash.txt", "M\n"), ("kitchen.toml", "keep")]);
        let p = plan(&b, &n, &o, &["kitchen.toml"]).unwrap();
        let action = |f: &str| p.changes.iter().find(|c| c.path == Path::new(f)).map(|c| c.action);
        assert_eq!(action("same.txt"), None);
        assert_eq!(action("upd.txt"), Some(Action::Update));
        assert_eq!(action("new/file.txt"), Some(Action::Create));
        assert_eq!(action("exists.txt"), Some(Action::Conflict));
        assert_eq!(action("gone.txt"), Some(Action::Delete));
        assert_eq!(action("gone-edited.txt"), Some(Action::Keep));
        assert_eq!(action("deleted.txt"), Some(Action::Keep));
        assert_eq!(action("both.txt"), Some(Action::Merge));
        assert_eq!(action("clash.txt"), Some(Action::Conflict));
        assert_eq!(action("kitchen.toml"), None);
        assert_eq!(p.conflicts(), 2);

        apply(&p, &b, &n, &o).unwrap();
        let read = |f: &str| std::fs::read_to_string(o.join(f)).ok();
        assert_eq!(read("upd.txt").as_deref(), Some("2"));
        assert_eq!(read("new/file.txt").as_deref(), Some("n"));
        assert_eq!(read("exists.txt").as_deref(), Some("mine"));
        assert_eq!(read("exists.txt.kitchen-new").as_deref(), Some("t"));
        assert_eq!(read("gone.txt"), None);
        assert_eq!(read("gone-edited.txt").as_deref(), Some("mine"));
        assert_eq!(read("both.txt").as_deref(), Some("a\nB\nc\nd\nE\n"));
        assert!(read("clash.txt").unwrap().contains("<<<<<<< yours"));
        assert_eq!(read("kitchen.toml").as_deref(), Some("keep"));
    }
}
