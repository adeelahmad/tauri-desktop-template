//! `kitchen mcp`: the same commands as MCP tools, over stdio (JSON-RPC, one message per
//! line), so an AI agent can create apps, switch features and generate code.
//!
//! Register it with a client, e.g. Claude Code:
//!
//! ```text
//! claude mcp add kitchen -- kitchen mcp
//! ```
//!
//! Every tool takes an optional `project` folder (default: where the server started) and
//! the ones that change files take `dry_run`. Each tool builds the same arguments the
//! command line would, and runs them through [`crate::cli::run`].
//!
//! ```
//! use serde_json::json;
//! let reply = kitchen::mcp::handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}), std::path::Path::new(".")).unwrap();
//! assert!(reply["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "enable"));
//! ```

use crate::cli::{self, Args};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;

/// The MCP protocol version this server speaks when the client does not say.
pub const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    let project =
        json!({"type": "string", "description": "App folder (default: the server's folder)"});
    let dry = json!({"type": "boolean", "description": "Only show what would change"});
    let list = json!({"type": "array", "items": {"type": "string"}});
    let deps = json!({"type": "boolean", "description": "Also switch the features this change needs (enable: what they need; disable: what needs them). Without it a change that needs others is refused; a dry run lists them"});
    json!([
        {"name": "features", "description": "List the template's features and which are on in the app. With check: whether the answers in kitchen.toml work together (feature dependencies, desktop-only features in a mobile build, licence notices).",
         "inputSchema": {"type": "object", "properties": {"check": {"type": "boolean", "description": "Check kitchen.toml against the feature dependency rules instead of listing"}, "project": project}}},
        {"name": "enable", "description": "Turn features on. A feature that needs another one that is off is refused unless with_deps is true. The app's own edits are kept (three-way merge).",
         "inputSchema": {"type": "object", "properties": {"features": list, "with_deps": deps, "project": project, "dry_run": dry}, "required": ["features"]}},
        {"name": "disable", "description": "Turn features off. If other features that are on need one of them, it is refused unless with_deps is true (then those are turned off too).",
         "inputSchema": {"type": "object", "properties": {"features": list, "with_deps": deps, "project": project, "dry_run": dry}, "required": ["features"]}},
        {"name": "set", "description": "Change an answer, e.g. name=frontend value=vanilla (yes/no answers take true or false and follow the same dependency rules as enable and disable).",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "value": {"type": "string"}, "with_deps": deps, "project": project, "dry_run": dry}, "required": ["name", "value"]}},
        {"name": "upgrade", "description": "Move the app to a newer template commit, keeping its edits.",
         "inputSchema": {"type": "object", "properties": {"rev": {"type": "string"}, "project": project, "dry_run": dry}}},
        {"name": "generate", "description": "Generate code: kind module (name, config), setting (name = module.key, type, default, label), command (name), page (name), menu (name = action id, label, shortcut), shortcut (name = action id, shortcut) or job (name; a background job type, needs background_jobs) or route (name; POST /v1/<name> on the HTTP API, needs http_api) or locale (name = a locale code; locales/<code>/app.ftl from English, needs i18n). menu and shortcut need the tray_menu feature.",
         "inputSchema": {"type": "object", "properties": {
             "kind": {"type": "string", "enum": ["module", "setting", "command", "page", "menu", "shortcut", "job", "route", "locale"]},
             "name": {"type": "string"}, "config": {"type": "boolean"},
             "type": {"type": "string", "enum": ["bool", "int", "float", "text", "list"]},
             "default": {"type": "string"}, "label": {"type": "string"},
             "shortcut": {"type": "string", "description": "Keys like CmdOrCtrl+Shift+K"},
             "pseudo": {"type": "boolean", "description": "locale: pseudo-translate instead of copying English"},
             "project": project, "dry_run": dry}, "required": ["kind", "name"]}},
        {"name": "i18n_check", "description": "Check the app's translations (template feature `i18n`): every locales/<code>/app.ftl is valid Fluent and has exactly the English messages, translations use only the variables English passes, and the UI asks only for ids English has. Fails (isError) with the list of problems. To start a new locale use generate with kind locale (name = the code, e.g. fr; pseudo for a pseudo-locale).",
         "inputSchema": {"type": "object", "properties": {"project": project}}},
        {"name": "models", "description": "Models built into the installer (src-tauri/resources/models/models.lock): action list (default; the catalog with sizes and licences), add or remove (name), fetch (download them and check every SHA-256) or verify (check the fetched files). Bundling is chosen with set models=bundle or bundle_with_fallback.",
         "inputSchema": {"type": "object", "properties": {"action": {"type": "string", "enum": ["list", "add", "remove", "fetch", "verify"]}, "name": {"type": "string", "description": "add/remove: the model, e.g. whisper-base"}, "project": project, "dry_run": dry}}},
        {"name": "doctor", "description": "Check the machine and the app; each problem comes with its fix.",
         "inputSchema": {"type": "object", "properties": {"project": project}}},
        {"name": "start", "description": "How to run the app with hot reload. It keeps running, so this returns the command for a terminal instead of blocking.",
         "inputSchema": {"type": "object", "properties": {"project": project}}},
        {"name": "test", "description": "Run the app's tests and return the report: unit (cargo test or nextest) and feature (the real app over tauri-driver; --self-test on macOS). Neither flag: both, skipping feature tests this machine cannot run.",
         "inputSchema": {"type": "object", "properties": {"unit": {"type": "boolean"}, "feature": {"type": "boolean"}, "project": project}}},
        {"name": "package", "description": "Build installers for this OS (mac: app,dmg; windows: nsis,msi; linux: appimage,deb) and copy them to ./packages. Other OSes are refused.",
         "inputSchema": {"type": "object", "properties": {"platform": {"type": "string", "enum": ["mac", "windows", "linux"]}, "release": {"type": "boolean"}, "project": project}}},
        {"name": "publish", "description": "Create a GitHub release with ./packages through gh. Only shows the command unless yes is true.",
         "inputSchema": {"type": "object", "properties": {"tag": {"type": "string"}, "yes": {"type": "boolean"}, "project": project}}},
        {"name": "audit", "description": "Supply-chain checks: cargo deny check (deny.toml), cargo audit, npm audit for ui-react and pip-audit for the Python example. Missing tools are skipped with the command that installs them; fails if any check fails.",
         "inputSchema": {"type": "object", "properties": {"project": project}}},
        {"name": "eject", "description": "Where a tool's own config lives (vite, tauri or cargo) and what Kitchen sets on top of it.",
         "inputSchema": {"type": "object", "properties": {"tool": {"type": "string", "enum": ["vite", "tauri", "cargo"]}, "project": project}, "required": ["tool"]}},
        {"name": "service", "description": "The app's daemon as an OS service (template feature `daemon`): action status (default), logs, install, uninstall, start, stop or restart; system for the machine-wide service. Builds the app if needed. Only status and logs run unless allow_changes is true; otherwise it reports what it would run.",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string", "enum": crate::service::ACTIONS},
             "system": {"type": "boolean", "description": "LaunchDaemon / system unit / Windows Service (needs administrator rights)"},
             "lines": {"type": "integer", "description": "logs: how many lines"},
             "allow_changes": {"type": "boolean", "description": "Really install, start, stop ... (default: only say what would run)"},
             "project": project}}},
        {"name": "eval", "description": "Run the app's search evals (template feature `evals`): builds the app, runs `<app> --eval` and returns recall@k, MRR, nDCG@k and p50/p95 latency per search mode, plus the full JSON report. Fails (isError) on a regression against `baseline`. `answers` also runs the AI answer path with an LLM faithfulness judge (skipped without an API key).",
         "inputSchema": {"type": "object", "properties": {
             "dataset": {"type": "string", "description": "JSONL dataset (default: the bundled sample); the corpus is corpus.jsonl next to it"},
             "baseline": {"type": "string", "description": "A saved report to compare with, e.g. evals/baseline.json"},
             "threshold": {"type": "number", "description": "Largest allowed drop in a metric (default 0.02)"},
             "k": {"type": "integer", "description": "Results scored per query (default 5)"},
             "answers": {"type": "boolean"},
             "project": project, "dry_run": dry}}},
        {"name": "new", "description": "Create a new app from the template in folder `dir` (default: the server's folder).",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "dir": {"type": "string"}, "enable": list, "disable": list, "with_deps": deps, "source": {"type": "string"}, "rev": {"type": "string"},
             "web": {"type": "string", "description": "Web app mode: the https URL of the site to wrap (its title, icon and certificate are read from it)"},
             "insecure_tls": {"type": "boolean", "description": "Web app: accept the site's self-signed certificate (pinned by SHA-256)"},
             "capture_keys": {"type": "boolean", "description": "Web app: capture all keys for the page"},
             "release_shortcut": {"type": "string", "description": "Web app: the shortcut that releases captured keys (default Ctrl+Alt+Shift+K)"},
             "microphone": {"type": "boolean"}, "muted": {"type": "boolean"}, "camera": {"type": "boolean"}, "screen": {"type": "boolean"},
             "scheme": {"type": "string", "description": "Custom URL scheme (myapp://...) when tauri_deep_link is on"},
             "engine": {"type": "string", "enum": ["system", "chromium"], "description": "Web engine on macOS: system (WKWebView, default) or chromium (CEF; about 150 MB more)"}}, "required": ["name"]}}
    ])
}

/// The command-line words for a tool call.
fn argv(tool: &str, a: &Value) -> Result<Vec<String>, String> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).map(String::from);
    let need = |k: &str| s(k).ok_or_else(|| format!("{tool}: `{k}` is required"));
    let strings = |k: &str| {
        a.get(k)
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut w: Vec<String> = match tool {
        "features" => {
            let mut v = vec!["features".to_string()];
            if a.get("check").and_then(Value::as_bool) == Some(true) {
                v.push("--check".into());
            }
            v
        }
        "doctor" | "start" | "audit" => vec![tool.into()],
        "i18n_check" => vec!["i18n".into(), "check".into()],
        "test" => {
            let mut v = vec!["test".to_string()];
            for k in ["unit", "feature"] {
                if a.get(k).and_then(Value::as_bool) == Some(true) {
                    v.push(format!("--{k}"));
                }
            }
            v
        }
        "package" => {
            let mut v = vec!["package".to_string()];
            v.extend(s("platform"));
            if a.get("release").and_then(Value::as_bool) == Some(true) {
                v.push("--release".into());
            }
            v
        }
        "publish" => {
            let mut v = vec!["publish".to_string()];
            if let Some(t) = s("tag") {
                v.extend(["--tag".into(), t]);
            }
            if a.get("yes").and_then(Value::as_bool) == Some(true) {
                v.push("--yes".into());
            }
            v
        }
        "eject" => vec!["eject".into(), need("tool")?],
        "models" => {
            let mut v = vec![
                "models".to_string(),
                s("action").unwrap_or_else(|| "list".into()),
            ];
            v.extend(s("name"));
            v
        }
        "eval" => {
            let mut v = vec!["eval".to_string()];
            v.extend(s("dataset"));
            if let Some(b) = s("baseline") {
                v.extend(["--baseline".into(), b]);
            }
            for k in ["threshold", "k"] {
                if let Some(n) = a.get(k).filter(|n| n.is_number()) {
                    v.extend([format!("--{k}"), n.to_string()]);
                }
            }
            if a.get("answers").and_then(Value::as_bool) == Some(true) {
                v.push("--answers".into());
            }
            v
        }
        "enable" | "disable" => {
            let f = strings("features");
            if f.is_empty() {
                return Err(format!("{tool}: list at least one feature"));
            }
            [vec![tool.to_string()], f].concat()
        }
        "set" => vec![
            "set".into(),
            format!("{}={}", need("name")?, need("value")?),
        ],
        "upgrade" => [
            vec!["upgrade".to_string()],
            s("rev")
                .map(|r| vec!["--rev".into(), r])
                .unwrap_or_default(),
        ]
        .concat(),
        "generate" => {
            let kind = need("kind")?;
            let mut v = vec!["generate".into(), kind.clone(), need("name")?];
            match (kind.as_str(), s("shortcut")) {
                ("shortcut", k) => v.push(k.ok_or("generate shortcut: `shortcut` is required")?),
                (_, Some(k)) => v.extend(["--shortcut".into(), k]),
                _ => {}
            }
            v.extend(s("type"));
            v.extend(s("default"));
            if let Some(l) = s("label") {
                v.extend(["--label".into(), l]);
            }
            if a.get("config").and_then(Value::as_bool) == Some(true) {
                v.push("--config".into());
            }
            if a.get("pseudo").and_then(Value::as_bool) == Some(true) {
                v.push("--pseudo".into());
            }
            v
        }
        "new" => {
            let mut v = vec!["new".into(), need("name")?, "--no-git".into()];
            for (k, list) in [
                ("enable", strings("enable")),
                ("disable", strings("disable")),
            ] {
                if !list.is_empty() {
                    v.extend([format!("--{k}"), list.join(",")]);
                }
            }
            for k in ["dir", "source", "rev", "web", "scheme", "engine"] {
                if let Some(x) = s(k) {
                    v.extend([format!("--{k}"), x]);
                }
            }
            if let Some(x) = s("release_shortcut") {
                v.extend(["--release-shortcut".into(), x]);
            }
            for (k, flag) in [
                ("insecure_tls", "--insecure-tls"),
                ("capture_keys", "--capture-keys"),
                ("microphone", "--mic"),
                ("muted", "--muted"),
                ("camera", "--camera"),
                ("screen", "--screen"),
            ] {
                if a.get(k).and_then(Value::as_bool) == Some(true) {
                    v.push(flag.into());
                }
            }
            v
        }
        "service" => {
            let action = s("action").unwrap_or_else(|| "status".into());
            let mut v = vec!["service".to_string(), action.clone()];
            if a.get("system").and_then(Value::as_bool) == Some(true) {
                v.push("--system".into());
            }
            if let Some(n) = a.get("lines").and_then(Value::as_u64) {
                v.extend(["--lines".into(), n.to_string()]);
            }
            // Dry-run safe: anything that changes the machine needs allow_changes.
            if !crate::service::read_only(&action)
                && a.get("allow_changes").and_then(Value::as_bool) != Some(true)
            {
                v.push("--dry-run".into());
            }
            v
        }
        other => return Err(format!("unknown tool {other:?}")),
    };
    if a.get("dry_run").and_then(Value::as_bool) == Some(true) {
        w.push("--dry-run".into());
    }
    if a.get("with_deps").and_then(Value::as_bool) == Some(true) {
        w.push("--with-deps".into());
    }
    Ok(w)
}

/// Answer one JSON-RPC message; `None` for notifications.
pub fn handle(msg: &Value, cwd: &Path) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "kitchen", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Kitchen builds cross-platform desktop apps from a template. Use `features` first, `doctor` when a build fails, and dry_run before changing files. Lifecycle: `start` (gives the command to run in a terminal), `test`, `package`, `publish` (dry run unless yes)."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let dir = args
                .get("project")
                .and_then(Value::as_str)
                .map(|p| cwd.join(p))
                .unwrap_or_else(|| cwd.to_path_buf());
            let (text, report, is_error) = match argv(name, &args)
                .map_err(crate::Error::Usage)
                .and_then(|w| cli::run(&Args::parse(w), &dir))
            {
                Ok(r) => (
                    r.to_string(),
                    serde_json::to_value(&r).unwrap_or(Value::Null),
                    false,
                ),
                Err(e) => (e.to_string(), Value::Null, true),
            };
            Ok(
                json!({"content": [{"type": "text", "text": text}], "structuredContent": report, "isError": is_error}),
            )
        }
        other => Err(json!({"code": -32601, "message": format!("method not found: {other}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

/// Serve until stdin closes.
pub fn serve(cwd: &Path) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, cwd),
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
            ),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tools_to_commands() {
        assert_eq!(
            argv(
                "enable",
                &json!({"features": ["email", "ai"], "dry_run": true})
            )
            .unwrap(),
            ["enable", "email", "ai", "--dry-run"]
        );
        assert_eq!(
            argv("generate", &json!({"kind": "setting", "name": "inv.days", "type": "int", "default": "30", "label": "Days"})).unwrap(),
            ["generate", "setting", "inv.days", "int", "30", "--label", "Days"]
        );
        assert_eq!(
            argv("generate", &json!({"kind": "menu", "name": "export_csv", "label": "Export", "shortcut": "Ctrl+E"})).unwrap(),
            ["generate", "menu", "export_csv", "--shortcut", "Ctrl+E", "--label", "Export"]
        );
        assert_eq!(
            argv(
                "generate",
                &json!({"kind": "shortcut", "name": "search", "shortcut": "Ctrl+K"})
            )
            .unwrap(),
            ["generate", "shortcut", "search", "Ctrl+K"]
        );
        assert!(argv("generate", &json!({"kind": "shortcut", "name": "search"})).is_err());
        assert_eq!(argv("i18n_check", &json!({})).unwrap(), ["i18n", "check"]);
        assert_eq!(
            argv(
                "generate",
                &json!({"kind": "locale", "name": "en-XA", "pseudo": true})
            )
            .unwrap(),
            ["generate", "locale", "en-XA", "--pseudo"]
        );
        assert_eq!(
            argv(
                "eval",
                &json!({"dataset": "d.jsonl", "baseline": "b.json", "threshold": 0.05, "k": 3, "answers": true})
            )
            .unwrap(),
            ["eval", "d.jsonl", "--baseline", "b.json", "--threshold", "0.05", "--k", "3", "--answers"]
        );
        assert_eq!(
            argv("new", &json!({"name": "x", "enable": ["a", "b"]})).unwrap(),
            ["new", "x", "--no-git", "--enable", "a,b"]
        );
        assert_eq!(
            argv("new", &json!({"name": "kvm", "web": "https://pikvm.local", "insecure_tls": true, "capture_keys": true, "release_shortcut": "F13", "muted": false})).unwrap(),
            ["new", "kvm", "--no-git", "--web", "https://pikvm.local", "--release-shortcut", "F13", "--insecure-tls", "--capture-keys"]
        );
        assert_eq!(
            argv("new", &json!({"name": "x", "engine": "chromium"})).unwrap(),
            ["new", "x", "--no-git", "--engine", "chromium"]
        );
        assert!(argv("enable", &json!({})).is_err());
        assert_eq!(
            argv(
                "enable",
                &json!({"features": ["daemon"], "with_deps": true, "dry_run": true})
            )
            .unwrap(),
            ["enable", "daemon", "--dry-run", "--with-deps"]
        );
        assert_eq!(
            argv("features", &json!({"check": true})).unwrap(),
            ["features", "--check"]
        );
        assert_eq!(
            argv("test", &json!({"feature": true})).unwrap(),
            ["test", "--feature"]
        );
        assert_eq!(
            argv("package", &json!({"platform": "linux", "release": true})).unwrap(),
            ["package", "linux", "--release"]
        );
        assert_eq!(argv("publish", &json!({})).unwrap(), ["publish"]);
        assert_eq!(argv("service", &json!({})).unwrap(), ["service", "status"]);
        assert_eq!(
            argv("service", &json!({"action": "install", "system": true})).unwrap(),
            ["service", "install", "--system", "--dry-run"]
        );
        assert_eq!(
            argv("service", &json!({"action": "stop", "allow_changes": true})).unwrap(),
            ["service", "stop"]
        );
    }

    #[test]
    fn speaks_json_rpc() {
        let cwd = Path::new(".");
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}), cwd).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            cwd
        )
        .is_none());
        let bad = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "nope"}), cwd).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
        let dir = crate::TempDir::new("mcp").unwrap();
        let call = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "features", "arguments": {}}}), dir.path()).unwrap();
        assert_eq!(call["result"]["isError"], true);
        // `start` never blocks: in an app it answers with the command to run.
        std::fs::write(
            dir.path().join("kitchen.toml"),
            "[app]\nname = \"x\"\n[template]\nsource = \"\"\nrev = \"\"\n",
        )
        .unwrap();
        let start = handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "start", "arguments": {}}}), dir.path()).unwrap();
        let text = start["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("kitchen start"), "{text}");
    }
}
