//! `.env` and `.env.local` for development builds: a tiny parser and loader with no
//! dependencies.
//!
//! The app calls [`load_dev`] first thing in `main`, and only when built with debug
//! assertions (`cargo tauri dev`, `cargo run`). Release builds never call it, never read the
//! files and never bundle them, so a secret in `.env` cannot end up in an installer. Both
//! files are in `.gitignore`.
//!
//! Rules:
//! - variables already set in the environment win over both files;
//! - `.env.local` wins over `.env` (keep machine-specific values there);
//! - each line is `KEY=value` or `export KEY=value`; blank lines and lines starting
//!   with `#` are skipped;
//! - keys are letters, digits, `_` and `.`, not starting with a digit;
//! - unquoted values are trimmed and end at ` #` (an inline comment);
//! - `'single quotes'` keep the text as it is; `"double quotes"` understand `\n`, `\t`,
//!   `\r`, `\"` and `\\`;
//! - one line per variable (no multi-line values, no `${VAR}` expansion);
//! - a line that breaks these rules is skipped and reported, never fatal.
//!
//! # Example
//!
//! ```
//! use desktop_core::dotenv;
//!
//! let parsed = dotenv::parse("# comment\nexport API_URL=http://localhost:8080 # dev\nGREETING=\"hi\\nthere\"\nbad line\n");
//! assert_eq!(parsed.vars, [
//!     ("API_URL".to_string(), "http://localhost:8080".to_string()),
//!     ("GREETING".to_string(), "hi\nthere".to_string()),
//! ]);
//! assert_eq!(parsed.errors.len(), 1); // line 4
//! assert_eq!(parsed.errors[0].line, 4);
//! ```
//!
//! Merging works the same way without touching the real environment:
//!
//! ```
//! use desktop_core::dotenv;
//!
//! let env = dotenv::parse("A=1\nB=from-env-file\n");
//! let local = dotenv::parse("A=2\n");
//! let set = dotenv::merge(&[env, local], |key| key == "B"); // B is already set
//! assert_eq!(set, [("A".to_string(), "2".to_string())]);
//! ```

use std::path::Path;

/// The files [`load_dev`] reads, in order; later files win.
pub const FILES: [&str; 2] = [".env", ".env.local"];

/// A line that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineError {
    /// 1-based line number.
    pub line: usize,
    /// Why it was skipped.
    pub message: String,
}

/// What [`parse`] found: the variables in file order, and the lines it skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// `(key, value)` pairs, in the order they appear. A repeated key appears twice; the
    /// last one wins in [`merge`].
    pub vars: Vec<(String, String)>,
    /// Lines that broke the rules (see the module docs).
    pub errors: Vec<LineError>,
}

/// Whether `key` may be an environment variable name here.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 256
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Parse the text of a `.env` file.
pub fn parse(text: &str) -> Parsed {
    let mut out = Parsed::default();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Ok(kv) => out.vars.push(kv),
            Err(message) => out.errors.push(LineError { line: i + 1, message }),
        }
    }
    out
}

fn parse_line(line: &str) -> Result<(String, String), String> {
    let line = line.strip_prefix("export ").map(str::trim_start).unwrap_or(line);
    let (key, rest) = line.split_once('=').ok_or("expected KEY=value")?;
    let key = key.trim();
    if !valid_key(key) {
        return Err(format!("{key:?} is not a valid name (letters, digits, _ and .)"));
    }
    let rest = rest.trim_start();
    let value = match rest.chars().next() {
        Some('\'') => {
            let end = rest[1..].find('\'').ok_or("missing closing '")?;
            trailing_ok(&rest[end + 2..])?;
            rest[1..end + 1].to_string()
        }
        Some('"') => {
            let mut value = String::new();
            let mut chars = rest[1..].char_indices();
            let mut closed = None;
            while let Some((i, c)) = chars.next() {
                match c {
                    '"' => {
                        closed = Some(i + 2);
                        break;
                    }
                    '\\' => match chars.next().map(|(_, e)| e) {
                        Some('n') => value.push('\n'),
                        Some('t') => value.push('\t'),
                        Some('r') => value.push('\r'),
                        Some('"') => value.push('"'),
                        Some('\\') => value.push('\\'),
                        Some(other) => {
                            value.push('\\');
                            value.push(other);
                        }
                        None => return Err("missing closing \"".into()),
                    },
                    c => value.push(c),
                }
            }
            let end = closed.ok_or("missing closing \"")?;
            trailing_ok(&rest[end..])?;
            value
        }
        _ => {
            let value = match rest.find(" #").or_else(|| rest.find("\t#")) {
                Some(at) => &rest[..at],
                None => rest,
            };
            value.trim_end().to_string()
        }
    };
    Ok((key.to_string(), value))
}

/// After a quoted value only spaces and a comment may follow.
fn trailing_ok(after: &str) -> Result<(), String> {
    let after = after.trim_start();
    if after.is_empty() || after.starts_with('#') {
        Ok(())
    } else {
        Err("unexpected text after the closing quote".into())
    }
}

/// The variables to set from several parsed files (later files win), leaving out every key
/// for which `is_set` says the environment already has a value. Sorted by key.
pub fn merge(files: &[Parsed], is_set: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut all = std::collections::BTreeMap::new();
    for file in files {
        for (k, v) in &file.vars {
            all.insert(k.clone(), v.clone());
        }
    }
    all.into_iter().filter(|(k, _)| !is_set(k)).collect()
}

/// Read `.env` and `.env.local` from `dir` (missing files are fine) and set the variables
/// that are not set yet. Returns the names it set (never the values); skipped lines are
/// logged as warnings.
///
/// Call it at the very start of `main`, before any thread starts: it changes the process
/// environment. The app calls it in debug builds only (`#[cfg(debug_assertions)]`).
///
/// ```no_run
/// # let workspace = std::path::Path::new(".");
/// #[cfg(debug_assertions)]
/// let names = desktop_core::dotenv::load_dev(workspace);
/// ```
pub fn load_dev(dir: &Path) -> Vec<String> {
    let files: Vec<Parsed> = FILES
        .iter()
        .filter_map(|name| {
            let bytes = crate::untrusted::read_capped(&dir.join(name), crate::untrusted::limits::DOTENV_BYTES).ok()?;
            let text = String::from_utf8(bytes).ok()?;
            let parsed = parse(&text);
            for e in &parsed.errors {
                // Written before logging starts, so to stderr; values never appear here.
                eprintln!("{name}:{}: skipped: {}", e.line, e.message);
            }
            Some(parsed)
        })
        .collect();
    let set = merge(&files, |k| std::env::var_os(k).is_some());
    for (k, v) in &set {
        std::env::set_var(k, v);
    }
    set.into_iter().map(|(k, _)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(text: &str) -> Vec<(String, String)> {
        parse(text).vars
    }

    fn kv(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn plain_values_comments_and_export() {
        let p = parse("\n# a comment\n  A = 1  \nexport B=two words\nC=x # note\nD=a#b\nE=\n");
        assert_eq!(p.vars, [kv("A", "1"), kv("B", "two words"), kv("C", "x"), kv("D", "a#b"), kv("E", "")]);
        assert!(p.errors.is_empty());
    }

    #[test]
    fn quotes() {
        assert_eq!(vars("A='keep \\n # as is'"), [kv("A", "keep \\n # as is")]);
        assert_eq!(vars(r#"A="line\nnext\t\"q\" \\ \x""#), [kv("A", "line\nnext\t\"q\" \\ \\x")]);
        assert_eq!(vars(r#"A="x" # trailing comment"#), [kv("A", "x")]);
        assert_eq!(vars("A=\"\""), [kv("A", "")]);
        assert_eq!(vars("A=\"é ünïcode\""), [kv("A", "é ünïcode")]);
    }

    #[test]
    fn bad_lines_are_reported_not_fatal() {
        let p = parse("no equals\n1BAD=x\nA-B=x\nA=\"open\nB='open\nC=\"x\" junk\nOK=yes\n");
        assert_eq!(p.vars, [kv("OK", "yes")]);
        assert_eq!(p.errors.iter().map(|e| e.line).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn windows_line_endings() {
        assert_eq!(vars("A=1\r\nB=\"2\"\r\n"), [kv("A", "1"), kv("B", "2")]);
    }

    #[test]
    fn later_files_win_and_the_environment_wins_over_all() {
        let env = parse("A=1\nB=1\nC=1\n");
        let local = parse("B=2\n");
        let set = merge(&[env, local], |k| k == "C");
        assert_eq!(set, [kv("A", "1"), kv("B", "2")]);
    }

    #[test]
    fn load_dev_sets_only_missing_variables() {
        let dir = std::env::temp_dir().join(format!("dotenv-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = ("DESKTOP_CORE_DOTENV_TEST_A", "DESKTOP_CORE_DOTENV_TEST_B");
        std::fs::write(dir.join(".env"), format!("{a}=env\n{b}=env\n")).unwrap();
        std::fs::write(dir.join(".env.local"), format!("{a}=local\n")).unwrap();
        std::env::set_var(b, "already");
        let set = load_dev(&dir);
        assert_eq!(set, [a.to_string()]);
        assert_eq!(std::env::var(a).unwrap(), "local");
        assert_eq!(std::env::var(b).unwrap(), "already");
        std::env::remove_var(a);
        std::env::remove_var(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_files_are_fine() {
        assert!(load_dev(Path::new("/definitely/not/here")).is_empty());
    }
}
