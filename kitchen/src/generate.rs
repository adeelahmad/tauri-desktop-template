//! Code generators, in the app's own style. Each one writes a new, documented and tested
//! file and wires it in at the `kitchen:` marker comments the template leaves in place.
//!
//! | command | creates | wires into |
//! |---------|---------|------------|
//! | `module <name> [--config]` | `desktop-core/src/<name>.rs` | `desktop-core/src/lib.rs`; with `--config` also `config.rs` and the Settings page |
//! | `setting <module>.<key> <type> [default]` | a field in that module's config | the module and the Settings page |
//! | `command <name>` | `src-tauri/src/<name>.rs` (a Tauri command + backend method) | `src-tauri/src/lib.rs`, `service.rs` |
//! | `page <Name>` | `ui-react/src/<Name>.jsx` | `ui-react/src/App.jsx` |
//!
//! Setting types: `bool`, `int`, `float`, `text`, `list` (of text).
//!
//! ```
//! use kitchen::generate::{to_camel, to_pascal, check_name};
//!
//! assert_eq!(to_pascal("export_csv"), "ExportCsv");
//! assert_eq!(to_camel("due_days"), "dueDays");
//! assert!(check_name("invoices").is_ok());
//! assert!(check_name("Bad-Name").is_err());
//! assert!(check_name("type").is_err());
//! ```

use crate::{io, Error, Options, Report, Result};
use std::path::{Path, PathBuf};

/// Insert `text` on the line before the first line containing `marker` in `file`.
/// Fails when the marker is missing (the file was changed by hand, or is from an older
/// template: `kitchen upgrade` brings them back).
pub fn insert_before(file: &Path, marker: &str, text: &str) -> Result<String> {
    let src = io(format!("read {}", file.display()), std::fs::read_to_string(file))?;
    let at = src.find(marker).ok_or_else(|| Error::Usage(format!("{} has no `{marker}` marker (re-add it, or run `kitchen upgrade`)", file.display())))?;
    let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    Ok(format!("{}{text}{}", &src[..line_start], &src[line_start..]))
}

/// A Rust-friendly snake_case name: starts with a letter, then `a-z`, `0-9`, `_`; not a
/// keyword.
pub fn check_name(name: &str) -> Result<()> {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield", "gen", "box", "try",
    ];
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok || KEYWORDS.contains(&name) {
        return Err(Error::Usage(format!("{name:?}: use a snake_case name like `export_csv` (not a Rust keyword)")));
    }
    Ok(())
}

/// `export_csv` → `ExportCsv`.
pub fn to_pascal(name: &str) -> String {
    name.split('_').filter(|w| !w.is_empty()).map(|w| w[..1].to_uppercase() + &w[1..]).collect()
}

/// `due_days` → `dueDays` (how serde writes it in settings).
pub fn to_camel(name: &str) -> String {
    let p = to_pascal(name);
    p[..1].to_lowercase() + &p[1..]
}

/// `due_days` → `Due days`.
fn to_title(name: &str) -> String {
    let s = name.replace('_', " ");
    s[..1].to_uppercase() + &s[1..]
}

/// The built-in file templates. An app can replace any of them with its own copy in
/// `.kitchen/templates/<file>` (copy one from the `kitchen` repo and edit it).
const BUILT_IN: &[(&str, &str)] = &[
    ("module.rs", include_str!("../templates/module.rs")),
    ("module_config.rs", include_str!("../templates/module_config.rs")),
    ("command.rs", include_str!("../templates/command.rs")),
    ("page.jsx", include_str!("../templates/page.jsx")),
];

/// Fill a file template: `{{name}}` style variables, from the app's override if it has one.
fn body(root: &Path, file: &str, vars: &[(&str, String)]) -> Result<String> {
    let custom = root.join(".kitchen/templates").join(file);
    let mut text = match std::fs::read_to_string(&custom) {
        Ok(t) => t,
        Err(_) => BUILT_IN.iter().find(|(n, _)| *n == file).map(|(_, t)| t.to_string()).ok_or_else(|| Error::Usage(format!("no template {file}")))?,
    };
    for (k, v) in vars {
        text = text.replace(&format!("{{{{{k}}}}}"), v);
    }
    Ok(text)
}

/// Files a generator writes, kept in memory until all of them are ready, so a failure
/// leaves the app untouched.
#[derive(Default)]
struct Edits(Vec<(PathBuf, String, bool)>);

impl Edits {
    fn create(&mut self, path: PathBuf, text: String) -> Result<()> {
        if path.exists() {
            return Err(Error::Usage(format!("{} already exists", path.display())));
        }
        self.0.push((path, text, true));
        Ok(())
    }
    /// Insert at a marker; reads the newest pending version of the file.
    fn insert(&mut self, path: PathBuf, marker: &str, text: &str) -> Result<()> {
        let pending = self.0.iter().rposition(|(p, _, _)| *p == path);
        let next = match pending {
            Some(i) => {
                let src = &self.0[i].1;
                let at = src.find(marker).ok_or_else(|| Error::Usage(format!("{} has no `{marker}` marker", path.display())))?;
                let ls = src[..at].rfind('\n').map_or(0, |i| i + 1);
                format!("{}{text}{}", &src[..ls], &src[ls..])
            }
            None => insert_before(&path, marker, text)?,
        };
        match pending {
            Some(i) => self.0[i].1 = next,
            None => self.0.push((path, next, false)),
        }
        Ok(())
    }
    fn finish(self, root: &Path, summary: String, opts: &Options) -> Result<Report> {
        let mut r = Report::new(if opts.dry_run { format!("Would: {summary}") } else { summary });
        for (path, text, new) in &self.0 {
            let rel = path.strip_prefix(root).unwrap_or(path);
            r.lines.push(format!("{:<8} {}", if *new { "create" } else { "update" }, rel.display()));
            if !opts.dry_run {
                io(format!("write {}", path.display()), std::fs::write(path, text))?;
            }
        }
        Ok(r)
    }
}

/// `kitchen generate module <name> [--config]`: a desktop-core module with docs and a test,
/// and with `config`, its own settings section.
pub fn module(root: &Path, name: &str, config: bool, opts: &Options) -> Result<Report> {
    check_name(name)?;
    let (p, camel, title) = (to_pascal(name), to_camel(name), to_title(name));
    let vars = [("name", name.to_string()), ("Name", p.clone()), ("camelName", camel.clone()), ("Title", title.clone())];
    let mut e = Edits::default();
    let body = body(root, if config { "module_config.rs" } else { "module.rs" }, &vars)?;
    let core = root.join("desktop-core/src");
    e.create(core.join(format!("{name}.rs")), body)?;
    e.insert(core.join("lib.rs"), "// kitchen:modules", &format!("pub mod {name};\n"))?;
    let mut next = vec![format!("Edit desktop-core/src/{name}.rs; cargo test -p desktop-core {name}")];
    if config {
        e.insert(core.join("config.rs"), "// kitchen:config", &format!("    /// Settings for [`crate::{name}`].\n    pub {name}: crate::{name}::{p}Config,\n"))?;
        e.insert(core.join("config.rs"), "// kitchen:validate", &format!("        self.{name} = self.{name}.validated();\n"))?;
        let settings = root.join("ui-react/src/Settings.jsx");
        if settings.is_file() {
            e.insert(settings.clone(), "// kitchen:titles", &format!("  {camel}: '{title}',\n"))?;
            e.insert(settings.clone(), "// kitchen:sections", &format!("    '{camel}',\n"))?;
            e.insert(settings, "// kitchen:settings", &format!("  ['{camel}', 'enabled', 'Turn on {title}', 'bool', {{}}],\n"))?;
        }
        next.push(format!("kitchen generate setting {name}.<key> <bool|int|float|text|list> [default]"));
    }
    let mut r = e.finish(root, format!("Generated module {name}"), opts)?;
    r.next = next;
    Ok(r)
}

/// `kitchen generate setting <module>.<key> <type> [default] [--label <text>]`.
pub fn setting(root: &Path, path: &str, kind: &str, default: Option<&str>, label: Option<&str>, opts: &Options) -> Result<Report> {
    let (module, key) = path.split_once('.').ok_or_else(|| Error::Usage("name the setting as <module>.<key>, e.g. invoices.due_days".into()))?;
    check_name(module)?;
    check_name(key)?;
    let bad = |d: &str| Error::Usage(format!("{d:?} is not a {kind}"));
    let (ty, lit, ui) = match (kind, default) {
        ("bool", d) => ("bool", d.unwrap_or("false").parse::<bool>().map_err(|_| bad(d.unwrap_or_default()))?.to_string(), "bool"),
        ("int", d) => ("u64", d.unwrap_or("0").parse::<u64>().map_err(|_| bad(d.unwrap_or_default()))?.to_string(), "number"),
        ("float", d) => ("f64", format!("{:?}", d.unwrap_or("0").parse::<f64>().map_err(|_| bad(d.unwrap_or_default()))?), "number"),
        ("text", d) => ("String", format!("{:?}.into()", d.unwrap_or_default()), "text"),
        ("list", None) => ("Vec<String>", "Vec::new()".into(), "list"),
        ("list", Some(d)) => ("Vec<String>", format!("vec![{}]", d.split(',').map(|s| format!("{:?}.into()", s.trim())).collect::<Vec<_>>().join(", ")), "list"),
        _ => return Err(Error::Usage(format!("{kind:?}: the type is one of bool, int, float, text, list"))),
    };
    let label = label.map(String::from).unwrap_or_else(|| to_title(key));
    let file = root.join(format!("desktop-core/src/{module}.rs"));
    if !file.is_file() {
        return Err(Error::Usage(format!("no module {module}: create it with `kitchen generate module {module} --config`")));
    }
    let mut e = Edits::default();
    e.insert(file.clone(), "// kitchen:fields", &format!("    /// {label}.\n    pub {key}: {ty},\n"))?;
    e.insert(file, "// kitchen:defaults", &format!("            {key}: {lit},\n"))?;
    let settings = root.join("ui-react/src/Settings.jsx");
    if settings.is_file() {
        let js_label = label.replace('\'', "\\'");
        e.insert(settings, "// kitchen:settings", &format!("  ['{}', '{}', '{js_label}', '{ui}', {{}}],\n", to_camel(module), to_camel(key)))?;
    }
    e.finish(root, format!("Added setting {module}.{key} ({kind})"), opts)
}

/// `kitchen generate command <name>`: a Tauri command the UI can `invoke`, backed by a
/// method that runs where the engine runs (in the app, or in the worker process).
pub fn command(root: &Path, name: &str, opts: &Options) -> Result<Report> {
    check_name(name)?;
    let src = root.join("src-tauri/src");
    let vars = [("name", name.to_string())];
    let mut e = Edits::default();
    e.create(
        src.join(format!("{name}.rs")),
        body(root, "command.rs", &vars)?,
    )?;
    e.insert(src.join("lib.rs"), "// kitchen:mods", &format!("mod {name};\n"))?;
    e.insert(src.join("lib.rs"), "// kitchen:handlers", &format!("            {name}::{name},\n"))?;
    e.insert(src.join("service.rs"), "// kitchen:methods", &format!("            \"{name}\" => crate::{name}::run(self.engine()?.as_ref(), params),\n"))?;
    let mut r = e.finish(root, format!("Generated command {name}"), opts)?;
    r.next = vec![format!("Edit src-tauri/src/{name}.rs, then call invoke('{name}', {{ input }}) from the UI")];
    Ok(r)
}

/// `kitchen generate page <Name>`: a React component shown on the main screen.
pub fn page(root: &Path, name: &str, opts: &Options) -> Result<Report> {
    let pascal = if name.contains('_') || name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) { to_pascal(name) } else { name.to_string() };
    if pascal.is_empty() || !pascal.chars().all(|c| c.is_ascii_alphanumeric()) || !pascal.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Err(Error::Usage(format!("{name:?}: use a name like Reports or reports")));
    }
    let ui = root.join("ui-react/src");
    if !ui.is_dir() {
        return Err(Error::Usage("pages need the React frontend: `kitchen set frontend=react`".into()));
    }
    let title = to_title(&pascal);
    let vars = [("Name", pascal.clone()), ("Title", title)];
    let mut e = Edits::default();
    e.create(
        ui.join(format!("{pascal}.jsx")),
        body(root, "page.jsx", &vars)?,
    )?;
    e.insert(ui.join("App.jsx"), "// kitchen:imports", &format!("import {pascal} from './{pascal}.jsx';\n"))?;
    e.insert(ui.join("App.jsx"), "{/* kitchen:pages */}", &format!("          <{pascal} />\n"))?;
    e.finish(root, format!("Generated page {pascal}"), opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny app tree with the template's markers.
    fn app() -> crate::TempDir {
        let t = crate::TempDir::new("gen").unwrap();
        let w = |p: &str, s: &str| {
            let f = t.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, s).unwrap();
        };
        w("desktop-core/src/lib.rs", "pub mod a;\n// kitchen:modules (...)\n\npub use a::A;\n");
        w("desktop-core/src/config.rs", "pub struct Config {\n    pub a: u8,\n    // kitchen:config (...)\n}\nfn v(mut self) {\n        // kitchen:validate\n        self\n}\n");
        w("src-tauri/src/lib.rs", "mod service;\n// kitchen:mods (...)\nfn x() { h![\n            a,\n            // kitchen:handlers\n        ] }\n");
        w("src-tauri/src/service.rs", "match m {\n            // kitchen:methods (...)\n            other => Err(()),\n}\n");
        w("ui-react/src/App.jsx", "import x from 'x';\n// kitchen:imports (...)\n<main>\n          {/* kitchen:pages */}\n</main>\n");
        w("ui-react/src/Settings.jsx", "const FIELDS = [\n  // kitchen:settings (...)\n];\nconst TITLES = {\n  // kitchen:titles\n};\nconst s = [\n    // kitchen:sections\n];\n");
        t
    }

    fn read(t: &crate::TempDir, p: &str) -> String {
        std::fs::read_to_string(t.path().join(p)).unwrap()
    }

    #[test]
    fn module_with_config_and_setting() {
        let t = app();
        let o = Options::default();
        module(t.path(), "invoices", true, &o).unwrap();
        assert!(read(&t, "desktop-core/src/lib.rs").contains("pub mod invoices;\n// kitchen:modules"));
        assert!(read(&t, "desktop-core/src/config.rs").contains("pub invoices: crate::invoices::InvoicesConfig,\n    // kitchen:config"));
        assert!(read(&t, "desktop-core/src/config.rs").contains("self.invoices = self.invoices.validated();"));
        assert!(read(&t, "ui-react/src/Settings.jsx").contains("invoices: 'Invoices'"));
        setting(t.path(), "invoices.due_days", "int", Some("30"), Some("Days until due"), &o).unwrap();
        let m = read(&t, "desktop-core/src/invoices.rs");
        assert!(m.contains("    /// Days until due.\n    pub due_days: u64,\n    // kitchen:fields"));
        assert!(m.contains("            due_days: 30,\n            // kitchen:defaults"));
        assert!(read(&t, "ui-react/src/Settings.jsx").contains("['invoices', 'dueDays', 'Days until due', 'number', {}]"));
        assert!(setting(t.path(), "invoices.x", "date", None, None, &o).is_err());
        assert!(setting(t.path(), "invoices.y", "int", Some("abc"), None, &o).is_err());
        assert!(module(t.path(), "invoices", false, &o).is_err(), "exists");
    }

    #[test]
    fn command_and_page() {
        let t = app();
        let o = Options::default();
        command(t.path(), "export_csv", &o).unwrap();
        assert!(read(&t, "src-tauri/src/lib.rs").contains("mod export_csv;\n// kitchen:mods"));
        assert!(read(&t, "src-tauri/src/lib.rs").contains("            export_csv::export_csv,\n            // kitchen:handlers"));
        assert!(read(&t, "src-tauri/src/service.rs").contains("\"export_csv\" => crate::export_csv::run(self.engine()?.as_ref(), params),"));
        page(t.path(), "reports", &o).unwrap();
        assert!(read(&t, "ui-react/src/App.jsx").contains("import Reports from './Reports.jsx';\n// kitchen:imports"));
        assert!(read(&t, "ui-react/src/App.jsx").contains("<Reports />\n          {/* kitchen:pages */}"));
    }

    #[test]
    fn dry_run_and_missing_marker_change_nothing() {
        let t = app();
        let before = read(&t, "desktop-core/src/lib.rs");
        let r = module(t.path(), "notes", false, &Options { dry_run: true }).unwrap();
        assert!(r.summary.starts_with("Would"));
        assert_eq!(read(&t, "desktop-core/src/lib.rs"), before);
        assert!(!t.path().join("desktop-core/src/notes.rs").exists());
        std::fs::write(t.path().join("src-tauri/src/service.rs"), "no marker").unwrap();
        assert!(command(t.path(), "go", &Options::default()).is_err());
        assert!(!t.path().join("src-tauri/src/go.rs").exists(), "nothing written on failure");
    }
}
