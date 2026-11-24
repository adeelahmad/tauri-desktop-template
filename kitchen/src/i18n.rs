//! Translations (template feature `i18n`): `kitchen i18n check` and
//! `kitchen generate locale <code> [--pseudo]`.
//!
//! An app's messages live in `locales/<code>/app.ftl` ([Fluent](https://projectfluent.org)).
//! English (`locales/en/app.ftl`) is the source: every other locale has the same message
//! ids, and the UI code only asks for ids English has.
//!
//! The check reports, and fails on:
//!
//! - a file that is not valid Fluent (with the line),
//! - a message English has and a locale lacks (missing), or the other way round (extra),
//! - a translation that uses a `{ $variable }` the English message never passes,
//! - an id the UI asks for (`t('id', …)`, `data-i18n="id"`) that English lacks.
//!
//! Messages marked `# untranslated` (what `kitchen generate locale` writes) are counted, not
//! failed: they still show English text until someone translates them.
//!
//! ```
//! use kitchen::i18n;
//! let en = i18n::parse("en", "hello = Hello, { $name }!\nbye = Bye\n    .title = Goodbye\n");
//! assert!(en.errors.is_empty());
//! assert_eq!(en.ids.keys().collect::<Vec<_>>(), ["bye", "bye.title", "hello"]);
//! let de = i18n::parse("de", "hello = Hallo, { $nme }!\n");
//! let problems = i18n::compare(&en, &de);
//! assert!(problems.iter().any(|p| p.contains("missing bye")));
//! assert!(problems.iter().any(|p| p.contains("$nme")));
//!
//! assert_eq!(i18n::pseudo_line("hello = Hello, { $name }!"), "hello = Ĥéééłłóóó, { $name }!");
//! ```

use crate::{io, Error, Options, Report, Result};
use fluent_syntax::ast;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The source locale every other one is compared with.
pub const SOURCE: &str = "en";

/// The comment `kitchen generate locale` puts above each message to translate.
pub const UNTRANSLATED: &str = "untranslated";

/// One parsed `app.ftl`.
#[derive(Debug, Clone, Default)]
pub struct Locale {
    /// The locale code (its folder name).
    pub code: String,
    /// Each message id (and `id.attribute`) with the variables it uses.
    pub ids: BTreeMap<String, BTreeSet<String>>,
    /// Messages still marked `# untranslated`.
    pub untranslated: usize,
    /// Syntax errors, as `line N: what`.
    pub errors: Vec<String>,
}

/// Parse one Fluent file.
pub fn parse(code: &str, ftl: &str) -> Locale {
    let mut l = Locale {
        code: code.into(),
        ..Locale::default()
    };
    let line = |pos: usize| ftl[..pos.min(ftl.len())].matches('\n').count() + 1;
    let resource = match fluent_syntax::parser::parse(ftl) {
        Ok(r) => r,
        Err((r, errors)) => {
            for e in errors {
                l.errors.push(format!("line {}: {e}", line(e.pos.start)));
            }
            r
        }
    };
    for entry in &resource.body {
        let ast::Entry::Message(m) = entry else {
            continue;
        };
        let id = m.id.name;
        if m.comment
            .as_ref()
            .is_some_and(|c| c.content.iter().any(|s| s.trim() == UNTRANSLATED))
        {
            l.untranslated += 1;
        }
        if let Some(v) = &m.value {
            l.ids.insert(id.into(), variables(v));
        }
        for a in &m.attributes {
            l.ids
                .insert(format!("{id}.{}", a.id.name), variables(&a.value));
        }
    }
    l
}

fn variables(p: &ast::Pattern<&str>) -> BTreeSet<String> {
    fn pattern(p: &ast::Pattern<&str>, out: &mut BTreeSet<String>) {
        for e in &p.elements {
            if let ast::PatternElement::Placeable { expression } = e {
                expr(expression, out);
            }
        }
    }
    fn expr(e: &ast::Expression<&str>, out: &mut BTreeSet<String>) {
        match e {
            ast::Expression::Inline(i) => inline(i, out),
            ast::Expression::Select { selector, variants } => {
                inline(selector, out);
                for v in variants {
                    pattern(&v.value, out);
                }
            }
        }
    }
    fn inline(i: &ast::InlineExpression<&str>, out: &mut BTreeSet<String>) {
        match i {
            ast::InlineExpression::VariableReference { id } => {
                out.insert(id.name.to_string());
            }
            ast::InlineExpression::FunctionReference { arguments, .. } => {
                for a in &arguments.positional {
                    inline(a, out);
                }
            }
            ast::InlineExpression::Placeable { expression } => expr(expression, out),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    pattern(p, &mut out);
    out
}

/// What is wrong with `other` compared with the source locale `en`.
pub fn compare(en: &Locale, other: &Locale) -> Vec<String> {
    let mut out = Vec::new();
    let code = &other.code;
    for id in en.ids.keys().filter(|id| !other.ids.contains_key(*id)) {
        out.push(format!("{code}: missing {id}"));
    }
    for (id, vars) in &other.ids {
        match en.ids.get(id) {
            None => out.push(format!("{code}: extra {id} (not in English)")),
            Some(source) => {
                for v in vars.difference(source) {
                    out.push(format!(
                        "{code}: {id} uses {{ ${v} }}, which the English message never passes"
                    ));
                }
            }
        }
    }
    out
}

/// The message ids a UI source file asks for: `t('id'`, `t("id"` and `data-i18n…="id"`.
pub fn ui_ids(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let ok = |id: &str| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
            && id.starts_with(|c: char| c.is_ascii_alphabetic())
    };
    let b = src.as_bytes();
    for (i, _) in src.match_indices("t(") {
        if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.') {
            continue;
        }
        let rest = &src[i + 2..];
        let Some(q) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else {
            continue;
        };
        if let Some(end) = rest[1..].find(q) {
            let id = &rest[1..1 + end];
            if ok(id) {
                out.insert(id.to_string());
            }
        }
    }
    for (i, _) in src.match_indices("data-i18n") {
        let rest = &src[i..];
        let Some(eq) = rest.find("=\"") else { continue };
        if rest[..eq].contains(char::is_whitespace) {
            continue;
        }
        if let Some(end) = rest[eq + 2..].find('"') {
            let id = &rest[eq + 2..eq + 2 + end];
            if ok(id) {
                out.insert(id.to_string());
            }
        }
    }
    out
}

fn locale_dirs(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let dir = root.join("locales");
    let mut out = Vec::new();
    for e in io(format!("read {}", dir.display()), std::fs::read_dir(&dir))? {
        let e = io(format!("read {}", dir.display()), e)?;
        let file = e.path().join("app.ftl");
        if file.is_file() {
            out.push((e.file_name().to_string_lossy().into_owned(), file));
        }
    }
    out.sort_by(|a, b| (a.0 != SOURCE, &a.0).cmp(&(b.0 != SOURCE, &b.0)));
    Ok(out)
}

fn ui_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for (dir, exts) in [
        ("ui-react/src", &["js", "jsx"][..]),
        ("ui", &["js", "html"][..]),
    ] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// `kitchen i18n check`: every locale against English, and the UI's ids against English.
/// Fails ([`Error::Tool`]) with the list of problems.
pub fn check(root: &Path) -> Result<Report> {
    if !root.join("locales").is_dir() {
        return Err(Error::Usage(
            "this app has no locales/ folder; turn translations on with `kitchen enable i18n`"
                .into(),
        ));
    }
    let mut problems = Vec::new();
    let mut lines = Vec::new();
    let mut locales = Vec::new();
    for (code, file) in locale_dirs(root)? {
        let text = io(
            format!("read {}", file.display()),
            std::fs::read_to_string(&file),
        )?;
        let l = parse(&code, &text);
        for e in &l.errors {
            problems.push(format!("locales/{code}/app.ftl {e}"));
        }
        locales.push(l);
    }
    let Some(en) = locales.iter().find(|l| l.code == SOURCE) else {
        return Err(Error::Tool(format!(
            "locales/{SOURCE}/app.ftl is missing: it is the source every other locale translates"
        )));
    };
    for l in &locales {
        let mut line = format!("{}: {} messages", l.code, l.ids.len());
        if l.untranslated > 0 {
            line.push_str(&format!(", {} untranslated", l.untranslated));
        }
        lines.push(line);
        if l.code != SOURCE {
            problems.extend(compare(en, l));
        }
    }
    for f in ui_files(root) {
        let Ok(src) = std::fs::read_to_string(&f) else {
            continue;
        };
        let rel = f.strip_prefix(root).unwrap_or(&f).display().to_string();
        for id in ui_ids(&src) {
            if !en.ids.contains_key(&id) {
                problems.push(format!(
                    "{rel} asks for {id}, which locales/{SOURCE}/app.ftl does not have"
                ));
            }
        }
    }
    if !problems.is_empty() {
        return Err(Error::Tool(format!(
            "translations: {} problem(s)\n  {}",
            problems.len(),
            problems.join("\n  ")
        )));
    }
    let mut r = Report::new(format!(
        "Translations OK: {} locale(s), each with the {} English messages.",
        locales.len(),
        en.ids.len()
    ));
    r.lines = lines;
    Ok(r)
}

/// Whether `code` is a sensible locale code: letters, digits and `-`, starting with a
/// 2–3 letter language (`de`, `pt-BR`, `sr-Latn`, `en-XA`).
pub fn valid_code(code: &str) -> bool {
    let mut parts = code.split('-');
    let lang = parts.next().unwrap_or_default();
    (2..=3).contains(&lang.len())
        && lang.chars().all(|c| c.is_ascii_lowercase())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// `kitchen generate locale <code> [--pseudo]`: start `locales/<code>/app.ftl` from English.
/// Each message is marked `# untranslated`; with `pseudo` the text is pseudo-translated
/// instead (accented and longer, to find untranslated strings and cut-off layouts).
pub fn generate_locale(root: &Path, code: &str, pseudo: bool, opts: &Options) -> Result<Report> {
    if !valid_code(code) {
        return Err(Error::Usage(format!(
            "{code:?} is not a locale code; use e.g. fr, pt-BR or en-XA"
        )));
    }
    let src = root.join("locales").join(SOURCE).join("app.ftl");
    if !src.is_file() {
        return Err(Error::Usage(
            "this app has no locales/en/app.ftl; turn translations on with `kitchen enable i18n`"
                .into(),
        ));
    }
    let dest = root.join("locales").join(code).join("app.ftl");
    if dest.exists() {
        return Err(Error::Usage(format!(
            "locales/{code}/app.ftl already exists; `kitchen i18n check` shows what it lacks"
        )));
    }
    let en = io(
        format!("read {}", src.display()),
        std::fs::read_to_string(&src),
    )?;
    let text = if pseudo {
        pseudo_file(&en)
    } else {
        untranslated_file(code, &en)
    };
    let mut r = Report::new(format!(
        "{} locales/{code}/app.ftl ({})",
        if opts.dry_run {
            "Would create"
        } else {
            "Created"
        },
        if pseudo {
            "pseudo-translated"
        } else {
            "every message marked untranslated"
        }
    ));
    r.lines.push(format!("create   locales/{code}/app.ftl"));
    if !opts.dry_run {
        let dir = dest.parent().expect("has a parent");
        io(
            format!("create {}", dir.display()),
            std::fs::create_dir_all(dir),
        )?;
        io(
            format!("write {}", dest.display()),
            std::fs::write(&dest, text),
        )?;
    }
    if !pseudo {
        r.next.push(format!(
            "translate locales/{code}/app.ftl (start with language-name) and delete each `# {UNTRANSLATED}` line"
        ));
    }
    r.next.push(
        "rebuild: the backend and the UI pick up every locales/*/app.ftl at build time".into(),
    );
    r.next.push("kitchen i18n check".into());
    Ok(r)
}

/// English with a header for `code` and `# untranslated` above every message.
fn untranslated_file(code: &str, en: &str) -> String {
    let mut out = format!(
        "# {code} translation of locales/en/app.ftl. Keep the ids; translate the text, and delete the\n# `# {UNTRANSLATED}` line above each message you translate (`kitchen i18n check` counts them).\n"
    );
    let mut in_header = true;
    for line in en.lines() {
        // Drop English's own file header (the first block of `#` lines).
        if in_header {
            if line.starts_with('#') && !line.starts_with("##") {
                continue;
            }
            in_header = false;
        }
        if starts_message(line) {
            out.push_str(&format!("# {UNTRANSLATED}\n"));
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn starts_message(line: &str) -> bool {
    line.split_once(" =")
        .or_else(|| line.split_once('='))
        .is_some_and(|(id, _)| {
            let id = id.trim_end();
            id.starts_with(|c: char| c.is_ascii_alphabetic())
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

fn pseudo_file(en: &str) -> String {
    let mut out = String::from(
        "# Pseudo-locale made by `kitchen generate locale <code> --pseudo` from locales/en/app.ftl:\n# accented and longer text, to spot strings that are not translated and layouts that cut text off.\n",
    );
    let mut in_header = true;
    for line in en.lines() {
        if in_header {
            if line.starts_with('#') && !line.starts_with("##") {
                continue;
            }
            in_header = false;
        }
        out.push_str(&pseudo_line(line));
        out.push('\n');
    }
    out
}

/// One line of a Fluent file, pseudo-translated: the text is accented and its vowels
/// tripled, while ids, attribute names, variant keys and `{ … }` placeables stay as they are.
pub fn pseudo_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "}" {
        return line.to_string();
    }
    // `id = text`, `    .attr = text`
    if let Some((head, rest)) = line.split_once(" = ") {
        let h = head.trim_start().trim_start_matches('.');
        if !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return format!("{head} = {}", pseudo_text(rest));
        }
    }
    // `[one] text`, `*[other] text`
    let variant = trimmed.trim_start_matches('*');
    if variant.starts_with('[') {
        if let Some(end) = variant.find(']') {
            let key_len = trimmed.len() - variant.len() + end + 1;
            return format!(
                "{indent}{}{}",
                &trimmed[..key_len],
                pseudo_text(&trimmed[key_len..])
            );
        }
    }
    format!("{indent}{}", pseudo_text(trimmed))
}

fn pseudo_text(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    for c in s.chars() {
        if depth > 0 {
            match c {
                '"' => quoted = !quoted,
                '{' if !quoted => depth += 1,
                '}' if !quoted => depth -= 1,
                _ => {}
            }
            out.push(c);
            continue;
        }
        if c == '{' {
            depth = 1;
            out.push(c);
            continue;
        }
        let (accented, vowel) = match c {
            'a' => ('á', true),
            'e' => ('é', true),
            'i' => ('í', true),
            'o' => ('ó', true),
            'u' => ('ú', true),
            'A' => ('Å', true),
            'E' => ('É', true),
            'I' => ('Î', true),
            'O' => ('Ö', true),
            'U' => ('Û', true),
            'c' => ('ç', false),
            'n' => ('ñ', false),
            's' => ('š', false),
            'y' => ('ý', false),
            'z' => ('ž', false),
            'C' => ('Ç', false),
            'H' => ('Ĥ', false),
            'N' => ('Ñ', false),
            'S' => ('Š', false),
            'l' => ('ł', false),
            other => (other, false),
        };
        out.push(accented);
        if vowel {
            out.push(accented);
            out.push(accented);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = "# header\n# more\n\n## Group\n\ntitle = Settings\n    .help = Open the settings\ncount = { $n ->\n    [one] One file\n   *[other] { $n } files\n}\n";

    #[test]
    fn untranslated_copies_mark_every_message() {
        let t = untranslated_file("fr", EN);
        assert!(!t.contains("# header"));
        let fr = parse("fr", &t);
        assert!(fr.errors.is_empty(), "{:?}", fr.errors);
        assert_eq!(fr.untranslated, 2);
        assert!(compare(&parse("en", EN), &fr).is_empty());
    }

    #[test]
    fn pseudo_keeps_ids_keys_and_placeables() {
        let t = pseudo_file(EN);
        let p = parse("en-XA", &t);
        assert!(p.errors.is_empty(), "{t}\n{:?}", p.errors);
        assert!(compare(&parse("en", EN), &p).is_empty(), "{t}");
        assert!(t.contains("   *[other] { $n } fíííłéééš"), "{t}");
        assert!(t.contains("    .help = ÖÖÖpéééñ"), "{t}");
        assert_eq!(pseudo_line("x = { \" \" }and"), "x = { \" \" }áááñd");
    }

    #[test]
    fn syntax_errors_have_lines() {
        let l = parse("de", "ok = fine\nbroken = { $x\n");
        assert_eq!(l.errors.len(), 1);
        assert!(l.errors[0].starts_with("line "), "{:?}", l.errors);
    }

    #[test]
    fn ui_ids_are_found() {
        let ids = ui_ids("x = t('a-b', 'A'); y = it('no'); t(\"c\", 'C'); t(`d${x}`); <b data-i18n=\"e\" data-i18n-title=\"f.g\">");
        assert_eq!(
            ids.into_iter().collect::<Vec<_>>(),
            ["a-b", "c", "e", "f.g"]
        );
    }

    #[test]
    fn check_and_generate_on_a_folder() {
        let dir = crate::TempDir::new("i18n").unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("locales/en")).unwrap();
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(root.join("locales/en/app.ftl"), EN).unwrap();
        std::fs::write(root.join("ui/app.js"), "t('title', 'Settings')").unwrap();
        assert!(check(root).is_ok());
        generate_locale(root, "fr", false, &Options::default()).unwrap();
        generate_locale(root, "en-XA", true, &Options::default()).unwrap();
        assert!(generate_locale(root, "fr", false, &Options::default()).is_err());
        assert!(generate_locale(root, "../x", false, &Options::default()).is_err());
        let r = check(root).unwrap();
        assert!(
            r.lines
                .iter()
                .any(|l| l == "fr: 3 messages, 2 untranslated"),
            "{r}"
        );
        // A message deleted from a translation, and an id only the UI knows, fail.
        let fr = std::fs::read_to_string(root.join("locales/fr/app.ftl")).unwrap();
        std::fs::write(
            root.join("locales/fr/app.ftl"),
            fr.replace("title = Settings\n", ""),
        )
        .unwrap();
        std::fs::write(root.join("ui/app.js"), "t('nope', 'No')").unwrap();
        let e = check(root).unwrap_err().to_string();
        assert!(e.contains("fr: missing title"), "{e}");
        assert!(e.contains("ui/app.js asks for nope"), "{e}");
    }

    #[test]
    fn codes() {
        for ok in ["de", "pt-BR", "sr-Latn", "en-XA", "fil"] {
            assert!(valid_code(ok), "{ok}");
        }
        for bad in ["", "D", "deutsch", "de_DE", "../x", "de-"] {
            assert!(!valid_code(bad), "{bad}");
        }
    }
}
