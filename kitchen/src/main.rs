//! The `kitchen` command. See the library docs (`cargo doc --open`) or `kitchen --help`.

use kitchen::cli::{self, Args, HELP};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = Args::parse(std::env::args().skip(1));
    if args.flag("version") {
        println!("kitchen {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.flag("help") || args.words.is_empty() {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let cwd = match cli::cwd() {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    match args.words[0].as_str() {
        "mcp" => match kitchen::mcp::serve(&cwd) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        // Interactive: hand the terminal to `cargo tauri`, from the app's root.
        cmd @ ("dev" | "build") => {
            let root = kitchen::manifest::Manifest::find_root(&cwd).unwrap_or(cwd);
            match std::process::Command::new("cargo")
                .arg("tauri")
                .arg(cmd)
                .args(&args.words[1..])
                .current_dir(root)
                .status()
            {
                Ok(s) if s.success() => ExitCode::SUCCESS,
                Ok(_) => ExitCode::FAILURE,
                Err(e) => fail(&format!(
                    "cargo tauri: {e} (kitchen doctor shows how to install it)"
                )),
            }
        }
        _ => match cli::run(&args, &cwd) {
            Ok(report) if args.flag("json") => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
                ExitCode::SUCCESS
            }
            Ok(report) => {
                print!("{report}");
                if args.words[0] == "doctor" && report.summary.contains("problem") {
                    return ExitCode::FAILURE;
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
    }
}

fn fail(e: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("kitchen: {e}");
    ExitCode::FAILURE
}
