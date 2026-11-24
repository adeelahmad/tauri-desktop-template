// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--self-test") {
        // `--self-test [report.json]`: CI runs this on the installed app. Release builds have
        // no console on Windows, so the report can also go to a file.
        std::process::exit(app_lib::self_test(args.get(2).map(std::path::Path::new)));
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
