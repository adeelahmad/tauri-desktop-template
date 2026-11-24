//! End to end against this repo's template: create an app, turn a feature on and off
//! again, and check the app is back where it started. Needs git and cargo-generate
//! (skipped with a note when they are missing).

use kitchen::cli::{run, Args};
use std::path::{Path, PathBuf};

fn have(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program).args(args).output().is_ok_and(|o| o.status.success())
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_path_buf();
            if rel == Path::new(".git") || rel == Path::new("kitchen.toml") {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, root, &mut v);
    v.sort();
    v
}

#[test]
fn new_enable_disable_round_trip() {
    if !have("git", &["--version"]) || !have("cargo", &["generate", "--version"]) {
        eprintln!("skipped: needs git and cargo-generate");
        return;
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let work = std::env::temp_dir().join(format!("kitchen-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    std::env::set_var("KITCHEN_CACHE", work.join("cache"));
    let source = repo.display().to_string();
    // A committed template, so `enable` (which checks out the recorded commit) sees the same files.
    let rev = kitchen_rev(&repo);
    let r = run(&Args::parse(["new", "app", "--source", &source, "--rev", &rev, "--no-git", "--disable", "postgres,embeddings,pdf"]), &work).unwrap();
    assert!(r.summary.starts_with("Created"), "{r}");
    let app = work.join("app");
    let before = snapshot(&app);
    let on = run(&Args::parse(["enable", "email"]), &app).unwrap();
    assert!(on.lines.iter().any(|l| l.contains("desktop-core/src/mail.rs")), "{on}");
    assert!(app.join("desktop-core/src/mail.rs").is_file());
    let off = run(&Args::parse(["disable", "email"]), &app).unwrap();
    assert!(off.lines.iter().any(|l| l.starts_with("delete")), "{off}");
    assert_eq!(snapshot(&app).len(), before.len());
    assert!(snapshot(&app) == before, "the app should be back where it started");
    let _ = std::fs::remove_dir_all(&work);
}

fn kitchen_rev(repo: &Path) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(["rev-parse", "HEAD"]).output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
