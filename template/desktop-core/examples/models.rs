//! Fetch or check the models the installer bundles (`make models`, `kitchen models fetch`).
//!
//! ```text
//! cargo run -q -p desktop-core --example models -- fetch    # download, check SHA-256s
//! cargo run -q -p desktop-core --example models -- verify   # check what is there
//! ```
//!
//! Both read `src-tauri/resources/models/models.lock` (another folder as the second
//! argument) and use the same code the app runs ([`desktop_core::models`]), so downloads obey
//! the same rules and checksums are checked the same way.

use desktop_core::models::{self, Lock, LOCK_FILE, RELEASE_ASSET_LIMIT, WARN_BYTES};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = args.first().map(String::as_str).unwrap_or("verify");
    let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("src-tauri/resources/models"));
    let lock = match Lock::load(&dir.join(LOCK_FILE)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}: {e}", dir.join(LOCK_FILE).display());
            return ExitCode::FAILURE;
        }
    };
    let bytes = lock.bytes();
    println!("{} model(s), {} MB in the installer", lock.models().len(), bytes / 1_000_000);
    if bytes > WARN_BYTES {
        println!("warning: the installer grows past 200 MB ({} MB of models)", bytes / 1_000_000);
    }
    if bytes > RELEASE_ASSET_LIMIT / 2 {
        println!("warning: GitHub refuses release files over 2 GB, and macOS notarization uploads the whole installer");
    }
    match action {
        "fetch" => {
            let mut last = String::new();
            let mut progress = |file: &str, done: u64, total: Option<u64>| {
                let line = format!("{file}: {} / {} MB", done / 1_000_000, total.map_or("?".into(), |t| (t / 1_000_000).to_string()));
                if line != last {
                    eprintln!("{line}");
                    last = line;
                }
            };
            match models::fetch(&lock, &dir, &mut progress) {
                Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "verify" => {}
        other => {
            eprintln!("unknown action {other:?}: use fetch or verify");
            return ExitCode::FAILURE;
        }
    }
    let problems = models::verify(&lock, &dir, true);
    for p in &problems {
        eprintln!("{p}");
    }
    if problems.is_empty() {
        println!("all files present, checksums match");
        ExitCode::SUCCESS
    } else {
        eprintln!("run `make models` (or `kitchen models fetch`) to fetch them again");
        ExitCode::FAILURE
    }
}
