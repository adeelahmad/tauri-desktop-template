//! Kitchen web wrapper on Chromium: one https site in a Tauri 3 window rendered by CEF
//! (`tauri-runtime-cef`), with the wrapper's hooks. A spike, standalone: see README.md.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod engine;
mod net;
mod policy;
mod wrapper;

use config::Config;
use tauri_runtime_cef::cef_entry_point;

/// The identifier in tauri.conf.json (the profile and the proxy port derive from it).
pub const APP_ID: &str = "dev.kitchen.cef-spike";

// CEF starts this same executable for its renderer, GPU and utility processes (from the
// helper apps on macOS). The macro sends those (`--type=...`) to CEF before `main` runs.
#[cef_entry_point]
fn main() {
    log_to_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match Config::parse(&args, &|k| std::env::var(k).ok()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kitchen-cef-spike: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = engine::run(cfg) {
        eprintln!("kitchen-cef-spike: {e}");
        std::process::exit(1);
    }
}

/// `log` records to stderr (`RUST_LOG=debug` for more), no logger crate.
fn log_to_stderr() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            m.level() <= log::max_level()
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("trace") => log::LevelFilter::Trace,
        Ok("debug") => log::LevelFilter::Debug,
        Ok("warn") => log::LevelFilter::Warn,
        _ => log::LevelFilter::Info,
    };
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}
