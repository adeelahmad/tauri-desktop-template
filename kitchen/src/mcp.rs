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
    json!([
        {"name": "features", "description": "List the template's features and which are on in the app.",
         "inputSchema": {"type": "object", "properties": {"project": project}}},
        {"name": "enable", "description": "Turn features on (with what they need). The app's own edits are kept (three-way merge).",
         "inputSchema": {"type": "object", "properties": {"features": list, "project": project, "dry_run": dry}, "required": ["features"]}},
        {"name": "disable", "description": "Turn features off (with what needs them).",
         "inputSchema": {"type": "object", "properties": {"features": list, "project": project, "dry_run": dry}, "required": ["features"]}},
        {"name": "set", "description": "Change a text answer, e.g. name=frontend value=vanilla.",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "value": {"type": "string"}, "project": project, "dry_run": dry}, "required": ["name", "value"]}},
        {"name": "upgrade", "description": "Move the app to a newer template commit, keeping its edits.",
         "inputSchema": {"type": "object", "properties": {"rev": {"type": "string"}, "project": project, "dry_run": dry}}},
        {"name": "generate", "description": "Generate code: kind module (name, config), setting (name = module.key, type, default, label), command (name) or page (name).",
         "inputSchema": {"type": "object", "properties": {
             "kind": {"type": "string", "enum": ["module", "setting", "command", "page"]},
             "name": {"type": "string"}, "config": {"type": "boolean"},
             "type": {"type": "string", "enum": ["bool", "int", "float", "text", "list"]},
             "default": {"type": "string"}, "label": {"type": "string"},
             "project": project, "dry_run": dry}, "required": ["kind", "name"]}},
        {"name": "doctor", "description": "Check the machine and the app; each problem comes with its fix.",
         "inputSchema": {"type": "object", "properties": {"project": project}}},
        {"name": "new", "description": "Create a new app from the template in folder `dir` (default: the server's folder).",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "dir": {"type": "string"}, "enable": list, "disable": list, "source": {"type": "string"}, "rev": {"type": "string"}}, "required": ["name"]}}
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
        "features" | "doctor" => vec![tool.into()],
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
            let mut v = vec!["generate".into(), need("kind")?, need("name")?];
            v.extend(s("type"));
            v.extend(s("default"));
            if let Some(l) = s("label") {
                v.extend(["--label".into(), l]);
            }
            if a.get("config").and_then(Value::as_bool) == Some(true) {
                v.push("--config".into());
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
            for k in ["dir", "source", "rev"] {
                if let Some(x) = s(k) {
                    v.extend([format!("--{k}"), x]);
                }
            }
            v
        }
        other => return Err(format!("unknown tool {other:?}")),
    };
    if a.get("dry_run").and_then(Value::as_bool) == Some(true) {
        w.push("--dry-run".into());
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
            "instructions": "Kitchen builds cross-platform desktop apps from a template. Use `features` first, `doctor` when a build fails, and dry_run before changing files."
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
            argv("new", &json!({"name": "x", "enable": ["a", "b"]})).unwrap(),
            ["new", "x", "--no-git", "--enable", "a,b"]
        );
        assert!(argv("enable", &json!({})).is_err());
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
    }
}
