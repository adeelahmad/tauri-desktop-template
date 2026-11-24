//! Third-party licence notices (feature `supply-chain`): the crates and npm packages an app
//! ships, their licences, and each distinct licence text once.
//!
//! The app's build script includes this file (`#[path]`) and runs it over `cargo metadata`
//! and `ui-react/package-lock.json`, so the notices are made at build time from exactly
//! what the build resolved, and compiled into the app (Settings → About → Licences). It
//! uses only the standard library and `serde_json`, so the build script needs nothing else.
//!
//! ```
//! use desktop_core::notices::{render, rust_packages, Ecosystem};
//! use serde_json::json;
//!
//! let metadata = json!({
//!     "packages": [
//!         {"id": "app", "name": "app", "version": "0.1.0", "source": null, "manifest_path": "/w/app/Cargo.toml"},
//!         {"id": "serde", "name": "serde", "version": "1.0.0", "license": "MIT OR Apache-2.0",
//!          "source": "registry+https://github.com/rust-lang/crates.io-index", "manifest_path": "/nowhere/serde/Cargo.toml"},
//!     ],
//!     "resolve": {"nodes": [
//!         {"id": "app", "deps": [{"pkg": "serde", "dep_kinds": [{"kind": null}]}]},
//!         {"id": "serde", "deps": []},
//!     ]},
//! });
//! let packages = rust_packages(&metadata, "app");
//! assert_eq!(packages.len(), 1, "the app itself is not a third party");
//! assert_eq!(packages[0].ecosystem, Ecosystem::Rust);
//! let text = render("My App", &packages);
//! assert!(text.contains("| serde | 1.0.0 | MIT OR Apache-2.0 |"));
//! ```

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// Where a package comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ecosystem {
    /// A Rust crate (crates.io, git or a path outside the workspace).
    Rust,
    /// An npm package bundled into the UI.
    Npm,
}

impl Ecosystem {
    /// `rust` or `npm`.
    pub fn as_str(self) -> &'static str {
        match self {
            Ecosystem::Rust => "rust",
            Ecosystem::Npm => "npm",
        }
    }
}

/// One third-party package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Rust crate or npm package.
    pub ecosystem: Ecosystem,
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The SPDX licence expression it declares (`"unknown"` when it declares none).
    pub license: String,
    /// Its repository or home page, if it names one.
    pub repository: Option<String>,
    /// Its folder on this machine (where the licence files are), if known.
    pub dir: Option<PathBuf>,
}

/// The crates `root` (a workspace package's name) links, following normal dependencies
/// only (no dev- or build-dependencies), without the workspace's own packages. `metadata`
/// is the output of `cargo metadata --format-version 1`; pass `--filter-platform <target>`
/// there to leave out other platforms' crates.
pub fn rust_packages(metadata: &Value, root: &str) -> Vec<Package> {
    let packages: BTreeMap<&str, &Value> =
        metadata["packages"].as_array().into_iter().flatten().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let nodes: BTreeMap<&str, &Value> =
        metadata["resolve"]["nodes"].as_array().into_iter().flatten().filter_map(|n| Some((n["id"].as_str()?, n))).collect();
    let Some(root_id) = packages.iter().find(|(_, p)| p["name"] == root && p["source"].is_null()).map(|(id, _)| *id) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::from([root_id]);
    let mut queue = VecDeque::from([root_id]);
    while let Some(id) = queue.pop_front() {
        let deps = nodes.get(id).and_then(|n| n["deps"].as_array()).into_iter().flatten();
        for dep in deps {
            let normal = dep["dep_kinds"].as_array().is_none_or(|kinds| kinds.is_empty() || kinds.iter().any(|k| k["kind"].is_null()));
            if let Some(pkg) = dep["pkg"].as_str().filter(|_| normal) {
                if seen.insert(pkg) {
                    queue.push_back(pkg);
                }
            }
        }
    }
    let mut out: Vec<Package> = seen
        .into_iter()
        .filter_map(|id| packages.get(id))
        .filter(|p| !p["source"].is_null())
        .map(|p| Package {
            ecosystem: Ecosystem::Rust,
            name: p["name"].as_str().unwrap_or_default().to_string(),
            version: p["version"].as_str().unwrap_or_default().to_string(),
            license: p["license"]
                .as_str()
                .map(String::from)
                .or_else(|| p["license_file"].as_str().map(|f| format!("see {f}")))
                .unwrap_or_else(|| "unknown".into()),
            repository: p["repository"].as_str().or(p["homepage"].as_str()).map(String::from),
            dir: p["manifest_path"].as_str().and_then(|m| Path::new(m).parent()).map(Path::to_path_buf),
        })
        .collect();
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    out
}

/// The npm packages in a `package-lock.json` (lockfile version 2 or 3) that the built UI
/// can contain: every installed package except dev-only ones. `ui_dir` is the folder the
/// lockfile is in (licence files are read from its `node_modules`).
pub fn npm_packages(lock: &Value, ui_dir: &Path) -> Vec<Package> {
    let mut out: Vec<Package> = lock["packages"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, p)| !key.is_empty() && p["dev"] != Value::Bool(true) && p["link"] != Value::Bool(true))
        .filter_map(|(key, p)| {
            let name = p["name"].as_str().or_else(|| key.rsplit("node_modules/").next())?.to_string();
            Some(Package {
                ecosystem: Ecosystem::Npm,
                name,
                version: p["version"].as_str().unwrap_or_default().to_string(),
                license: p["license"].as_str().map(String::from).unwrap_or_else(|| "unknown".into()),
                repository: None,
                dir: Some(ui_dir.join(key)),
            })
        })
        .collect();
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    out.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    out
}

/// The licence and notice files in a package folder (`LICENSE*`, `LICENCE*`, `COPYING*`,
/// `NOTICE*`, `UNLICENSE`), as `(file name, text)`, sorted by name.
pub fn license_texts(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let upper = name.to_ascii_uppercase();
            let wanted = ["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"].iter().any(|p| upper.starts_with(p));
            let text = std::fs::read_to_string(e.path()).ok().filter(|_| wanted)?;
            Some((name, text))
        })
        .collect();
    out.sort();
    out
}

/// The notices as Markdown: a table of every package, then each distinct licence text once
/// with the packages it covers. Licence files are read from each package's `dir`.
pub fn render(app_name: &str, packages: &[Package]) -> String {
    let mut s = format!(
        "# Third-party notices\n\n{app_name} includes the open-source software below. Each package is listed with the licence it declares; the licence texts follow.\n\n"
    );
    s.push_str("| Package | Version | Licence | Source |\n|---|---|---|---|\n");
    for p in packages {
        s.push_str(&format!(
            "| {} | {} | {} | {}{} |\n",
            p.name,
            p.version,
            p.license,
            p.ecosystem.as_str(),
            p.repository.as_deref().map(|r| format!(" ({r})")).unwrap_or_default()
        ));
    }
    // Identical texts (most Apache-2.0 copies, say) are printed once.
    let mut texts: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let mut without = Vec::new();
    for p in packages {
        let found = p.dir.as_deref().map(license_texts).unwrap_or_default();
        if found.is_empty() {
            without.push(format!("{} {} ({})", p.name, p.version, p.license));
        }
        for (file, text) in found {
            let key = text.split_whitespace().collect::<Vec<_>>().join(" ");
            texts.entry(key).or_insert_with(|| (format!("{file}\n\n{}", text.trim_end()), Vec::new())).1.push(format!("{} {}", p.name, p.version));
        }
    }
    s.push_str("\n## Licence texts\n");
    for (_, (text, users)) in texts {
        s.push_str(&format!("\n### Used by: {}\n\n```text\n{}\n```\n", users.join(", "), text.replace("```", "'''")));
    }
    if !without.is_empty() {
        s.push_str(&format!(
            "\n## Packages without a licence file\n\nThese ship no licence file of their own; the standard text of the licence they declare applies:\n\n{}\n",
            without.iter().map(|w| format!("- {w}")).collect::<Vec<_>>().join("\n")
        ));
    }
    s
}

/// The package list as JSON for the UI: `[{ecosystem, name, version, license, repository}]`.
pub fn summary(packages: &[Package]) -> Value {
    Value::Array(
        packages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "ecosystem": p.ecosystem.as_str(),
                    "name": p.name,
                    "version": p.version,
                    "license": p.license,
                    "repository": p.repository,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn follows_normal_dependencies_only() {
        let reg = "registry+https://github.com/rust-lang/crates.io-index";
        let m = json!({
            "packages": [
                {"id": "app", "name": "app", "version": "0.1.0", "source": null},
                {"id": "core", "name": "core-lib", "version": "0.1.0", "source": null},
                {"id": "a", "name": "a", "version": "1.0.0", "source": reg, "license": "MIT"},
                {"id": "b", "name": "b", "version": "2.0.0", "source": reg, "license_file": "LICENSE.txt"},
                {"id": "dev", "name": "dev-only", "version": "1.0.0", "source": reg, "license": "MIT"},
                {"id": "build", "name": "build-only", "version": "1.0.0", "source": reg},
            ],
            "resolve": {"nodes": [
                {"id": "app", "deps": [
                    {"pkg": "core", "dep_kinds": [{"kind": null}]},
                    {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]},
                    {"pkg": "build", "dep_kinds": [{"kind": "build"}]},
                ]},
                {"id": "core", "deps": [{"pkg": "a", "dep_kinds": [{"kind": null}]}]},
                {"id": "a", "deps": [{"pkg": "b", "dep_kinds": [{"kind": "dev"}, {"kind": null}]}]},
            ]},
        });
        let names: Vec<_> = rust_packages(&m, "app").into_iter().map(|p| (p.name, p.license)).collect();
        assert_eq!(names, [("a".to_string(), "MIT".to_string()), ("b".into(), "see LICENSE.txt".into())]);
        assert!(rust_packages(&m, "missing").is_empty());
    }

    #[test]
    fn npm_leaves_out_dev_packages() {
        let lock = json!({"lockfileVersion": 3, "packages": {
            "": {"name": "ui"},
            "node_modules/react": {"version": "19.0.0", "license": "MIT"},
            "node_modules/vite": {"version": "7.0.0", "license": "MIT", "dev": true},
            "node_modules/a/node_modules/react": {"version": "19.0.0", "license": "MIT"},
            "node_modules/@scope/pkg": {"version": "1.0.0"},
        }});
        let p = npm_packages(&lock, Path::new("/ui"));
        let names: Vec<_> = p.iter().map(|p| (p.name.as_str(), p.license.as_str())).collect();
        assert_eq!(names, [("@scope/pkg", "unknown"), ("react", "MIT")]);
        assert!(p[1].dir.as_deref().is_some_and(|d| d.starts_with("/ui/node_modules")));
    }

    #[test]
    fn texts_are_printed_once() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [("x", "MIT License\n\nSame   text."), ("y", "MIT License\nSame text.\n"), ("z", "Other.")] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
            std::fs::write(dir.path().join(name).join("LICENSE-MIT"), text).unwrap();
            std::fs::write(dir.path().join(name).join("README.md"), "not a licence").unwrap();
        }
        let pkg = |name: &str, dir: Option<PathBuf>| Package {
            ecosystem: Ecosystem::Rust,
            name: name.into(),
            version: "1.0.0".into(),
            license: "MIT".into(),
            repository: None,
            dir,
        };
        let packages =
            [pkg("x", Some(dir.path().join("x"))), pkg("y", Some(dir.path().join("y"))), pkg("z", Some(dir.path().join("z"))), pkg("w", None)];
        let text = render("App", &packages);
        assert_eq!(text.matches("### Used by").count(), 2, "{text}");
        assert!(text.contains("Used by: x 1.0.0, y 1.0.0"), "{text}");
        assert!(text.contains("- w 1.0.0 (MIT)"), "{text}");
        assert!(!text.contains("not a licence"));
        assert_eq!(summary(&packages)[0]["name"], "x");
    }
}
