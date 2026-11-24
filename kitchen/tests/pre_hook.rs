//! The template's cargo-generate pre-hook (`template/hooks/compat.rhai`) with plain
//! `cargo generate`: a combination that does not work is refused with a clear message,
//! and every feature on, or every feature off, still generates. Needs cargo-generate
//! (skipped with a note when it is missing). Generates only; nothing is built.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("template")
}

/// `cargo generate` into a fresh folder with `answers`, every yes/no question answered.
fn generate(tag: &str, answers: &[(String, String)]) -> (Output, PathBuf) {
    let dest = std::env::temp_dir().join(format!("kitchen-hook-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();
    let mut c = Command::new("cargo");
    c.args(["generate", "--path"])
        .arg(template())
        .args(["--name", "hook-app", "--destination"])
        .arg(&dest)
        .args(["--silent", "--vcs", "none"])
        .current_dir(&dest);
    for (k, v) in answers {
        c.arg("-d").arg(format!("{k}={v}"));
    }
    (c.output().unwrap(), dest)
}

/// Every question answered: yes/no ones with `on`, the others with their default.
fn all(on: bool) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(template().join("cargo-generate.toml")).unwrap();
    kitchen::template::parse_features(&text)
        .unwrap()
        .into_iter()
        .map(|f| {
            let v = match &f.default {
                toml::Value::Boolean(_) => on.to_string(),
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (f.name, v)
        })
        .collect()
}

fn with(mut answers: Vec<(String, String)>, changes: &[(&str, &str)]) -> Vec<(String, String)> {
    for (k, v) in changes {
        answers.retain(|(n, _)| n != k);
        answers.push((k.to_string(), v.to_string()));
    }
    answers
}

fn have_cargo_generate() -> bool {
    Command::new("cargo")
        .args(["generate", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn pre_hook_refuses_bad_combinations_and_lets_all_on_and_all_off_through() {
    if !have_cargo_generate() {
        eprintln!("skipped: needs cargo-generate");
        return;
    }
    // daemon needs ipc_worker.
    let (out, dir) = generate(
        "bad",
        &with(all(false), &[("daemon", "true"), ("ipc_worker", "false")]),
    );
    let err =
        String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.status.success(),
        "daemon without ipc_worker generated:\n{err}"
    );
    assert!(err.contains("daemon needs ipc_worker"), "{err}");
    assert!(
        err.contains("-d ipc_worker=true"),
        "the fix is named: {err}"
    );

    // Two at once are both listed.
    let (out, dir) = generate(
        "bad2",
        &with(all(false), &[("bm25", "true"), ("local_llm", "true")]),
    );
    let err =
        String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!out.status.success());
    assert!(
        err.contains("bm25 needs postgres") && err.contains("local_llm needs ai"),
        "{err}"
    );

    for on in [false, true] {
        let (out, dir) = generate(if on { "on" } else { "off" }, &all(on));
        let log = String::from_utf8_lossy(&out.stderr).to_string()
            + &String::from_utf8_lossy(&out.stdout);
        let app = dir.join("hook-app");
        let ok = out.status.success() && app.join("Cargo.toml").is_file();
        let hook_copied = app.join("hooks").exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "all {} did not generate:\n{log}",
            if on { "on" } else { "off" }
        );
        assert!(!hook_copied, "the hook folder is template-only");
        if on {
            assert!(log.contains("desktop only"), "mobile warnings: {log}");
            assert!(log.contains("AGPL-3.0"), "licence notice for bm25: {log}");
        }
    }
}
