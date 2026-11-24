//! `<app>` and its command-line entry points.
//!
//! The executable: the window by default, or one of the other entry points.
//!
//! ```text
//! my-app                          open the window
//! my-app --worker                 run the backend process (feature ipc-worker)
//! my-app --mcp                    serve MCP over stdio (feature mcp)
//! my-app --self-test [out.json]   index and search a sample, print a JSON report
//! my-app --eval [dataset] [--baseline file]   search evals, JSON report (feature evals)
//! my-app --pg-upgrade [--check]   upgrade the embedded Postgres data now (feature postgres)
{%- if pdf %}
//! my-app --parse                  parse one PDF from stdin, JSON on stdout (feature pdf)
{%- endif %}
//! ```
//!
//! Development builds first load `.env` and `.env.local` from the project folder (see
//! `desktop_core::dotenv`); release builds never do.

// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Development builds only: `.env` and `.env.local` from the project folder (variables
    // already set win). Release builds never read or bundle them (desktop_core::dotenv).
    #[cfg(debug_assertions)]
    desktop_core::dotenv::load_dev(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")));
    let args: Vec<String> = std::env::args().collect();
{%- if pdf %}
    #[cfg(feature = "pdf")]
    if args.get(1).map(String::as_str) == Some(desktop_core::pdf::PARSE_FLAG) {
        // The PDF parser child (desktop_core::pdf::pdf_pages_isolated): untrusted bytes on
        // stdin, one JSON reply on stdout, under the time and memory limits its parent set.
        std::process::exit(desktop_core::pdf::serve_parse(std::io::stdin().lock(), std::io::stdout().lock()));
    }
{%- endif %}
    if args.get(1).map(String::as_str) == Some("--self-test") {
        // `--self-test [report.json]`: CI runs this on the installed app. Release builds have
        // no console on Windows, so the report can also go to a file.
        std::process::exit(app_lib::self_test(args.get(2).map(std::path::Path::new)));
    }
    if args.get(1).map(String::as_str) == Some("--eval") {
        // `--eval [dataset.jsonl] [--baseline b.json] ...`: search and RAG evals (feature `evals`).
        std::process::exit(app_lib::eval_main(&args[2..]));
    }
    if args.get(1).map(String::as_str) == Some("--pg-upgrade") {
        // `--pg-upgrade [--check]`: upgrade the embedded Postgres data folder now (feature `postgres`).
        std::process::exit(app_lib::pg_upgrade_main(&args[2..]));
    }
    // Production safety: a production build refuses to start without the config listed in
    // config/*.json `requiredInProduction` (see app_lib::app_config::check).
    if let Err(e) = app_lib::app_config::check() {
        eprintln!("{e}");
        std::process::exit(2);
    }
    if args.get(1).map(String::as_str) == Some("--mcp") {
        // An MCP client (Claude Desktop, Claude Code, ...) started us: JSON-RPC on stdio.
        std::process::exit(app_lib::mcp_main());
    }
    if args.get(1).map(String::as_str) == Some("--worker") {
        // The backend process (feature `ipc-worker`); the window and the background agent start it.
        std::process::exit(app_lib::worker_main());
    }
    if args.get(1).map(String::as_str) == Some("--daemon") {
        // The backend with no window, run by a service manager (feature `daemon`).
        std::process::exit(app_lib::daemon_main(&args[2..]));
    }
    if args.get(1).map(String::as_str) == Some("--service") {
        // `--service install|uninstall|start|stop|restart|status|logs [--system]` (feature `daemon`).
        std::process::exit(app_lib::service_main(&args[2..]));
    }
    app_lib::run()
}
