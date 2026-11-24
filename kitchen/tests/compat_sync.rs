//! The rule table in `kitchen/src/compat.rs` is the only source: the template's Rhai
//! pre-hook and the "Feature dependencies" tables in the README and the guide must match
//! it, and every conditional question in `cargo-generate.toml` must be one of its rules.
//!
//! `KITCHEN_BLESS=1 cargo test --test compat_sync` rewrites the generated parts.

use kitchen::compat::{self, DOC_MARKERS};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn bless() -> bool {
    std::env::var_os("KITCHEN_BLESS").is_some_and(|v| v == "1")
}

/// Replace what is between `begin` and `end` (marker lines kept) with `body`.
fn splice(text: &str, begin: &str, end: &str, body: &str) -> Option<String> {
    let b = text.find(begin)? + begin.len();
    let b = b + text[b..].find('\n')? + 1;
    let e = b + text[b..].find(end)?;
    Some(format!("{}{body}{}", &text[..b], &text[e..]))
}

fn check_region(path: &Path, begin: &str, end: &str, body: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    let want = splice(&text, begin, end, body)
        .unwrap_or_else(|| panic!("{} lacks the {begin:?} markers", path.display()));
    if want != text {
        if bless() {
            std::fs::write(path, want).unwrap();
        } else {
            panic!(
                "{} is out of date with kitchen/src/compat.rs; run `KITCHEN_BLESS=1 cargo test --test compat_sync`",
                path.display()
            );
        }
    }
}

#[test]
fn rhai_hook_has_the_same_rules() {
    let path = repo().join("template/hooks/compat.rhai");
    check_region(
        &path,
        "// kitchen:rules begin",
        "// kitchen:rules end",
        &compat::rhai_tables(),
    );
    // Parse both sides and compare cell by cell (the check above compares the text).
    let hook = compat::parse_rhai_tables(&std::fs::read_to_string(&path).unwrap());
    let table = |name: &str| {
        hook.iter()
            .find(|(n, _)| n == name)
            .map(|(_, rows)| rows.clone())
            .unwrap_or_else(|| panic!("the hook has no {name} table"))
    };
    let requires: Vec<Vec<String>> = compat::REQUIRES
        .iter()
        .map(|r| vec![r.feature.into(), r.needs.into(), r.why.into()])
        .collect();
    assert_eq!(table("REQUIRES"), requires);
    let advice: Vec<Vec<String>> = compat::ADVICE
        .iter()
        .map(|a| vec![a.feature.into(), a.suggests.into(), a.why.into()])
        .collect();
    assert_eq!(table("ADVICE"), advice);
    let desktop: Vec<Vec<String>> = compat::DESKTOP_ONLY
        .iter()
        .map(|d| vec![d.feature.into(), d.on_mobile.into()])
        .collect();
    assert_eq!(table("DESKTOP_ONLY"), desktop);
    let licences: Vec<Vec<String>> = compat::LICENCES
        .iter()
        .map(|l| vec![l.feature.into(), l.notice.into()])
        .collect();
    assert_eq!(table("LICENCES"), licences);
    // And cargo-generate runs it.
    let cg = std::fs::read_to_string(repo().join("template/cargo-generate.toml")).unwrap();
    let doc: toml::Table = toml::from_str(&cg).unwrap();
    let pre = doc["hooks"]["pre"].as_array().unwrap();
    assert!(pre.iter().any(|p| p.as_str() == Some("hooks/compat.rhai")));
}

#[test]
fn readme_and_guide_tables_are_generated_from_the_rules() {
    let body = compat::markdown();
    for f in ["README.md", "template/docs/GUIDE.md"] {
        check_region(&repo().join(f), DOC_MARKERS.0, DOC_MARKERS.1, &body);
    }
}

#[test]
fn every_rule_names_a_real_question_and_every_conditional_question_is_a_rule() {
    let cg = std::fs::read_to_string(repo().join("template/cargo-generate.toml")).unwrap();
    let features = kitchen::template::parse_features(&cg).unwrap();
    let switch = |n: &str| features.iter().any(|f| f.name == n && f.is_switch());
    for r in compat::REQUIRES {
        assert!(switch(r.feature) && switch(r.needs), "{r:?}");
    }
    for a in compat::ADVICE {
        assert!(switch(a.feature) && switch(a.suggests), "{a:?}");
    }
    for n in compat::DESKTOP_ONLY
        .iter()
        .map(|d| d.feature)
        .chain(compat::LICENCES.iter().map(|l| l.feature))
    {
        assert!(switch(n), "{n} is not a yes/no question");
    }
    for f in features.iter().filter(|f| f.is_switch()) {
        if let Some(parent) = &f.requires {
            assert!(
                compat::REQUIRES
                    .iter()
                    .any(|r| r.feature == f.name && r.needs == parent),
                "{} is asked only with {parent}: add it to compat::REQUIRES",
                f.name
            );
        }
    }
}
