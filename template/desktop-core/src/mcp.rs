//! A Model Context Protocol server over stdio (feature `mcp`), in plain Rust.
//!
//! Claude Desktop, Claude Code and other MCP clients start the app with `--mcp` and talk
//! JSON-RPC 2.0, one message per line. The app supplies the tools through `Handler`;
//! this module does the protocol, the policy and the audit log:
//! - only read-only tools are offered unless `McpConfig::allow_write` is on;
//! - `McpConfig::tools`, when not empty, is the allowlist of tool names;
//! - every call is appended to the audit log (time, tool, ok, duration; never the arguments
//!   unless `audit_arguments` is on, since they can hold private text).
//!
//! Main items: [`Server`] ([`Server::serve_stdio`], or [`Server::handle`] for one message),
//! the [`Handler`] trait the app implements, [`Tool`] and [`McpConfig`].
//!
//! # Example
//!
//! ```
//! use desktop_core::mcp::{Handler, McpConfig, Server, Tool};
//! use serde_json::{json, Value};
//!
//! struct Echo;
//! impl Handler for Echo {
//!     fn tools(&self) -> Vec<Tool> {
//!         vec![Tool { name: "echo".into(), description: "Repeat the arguments".into(), input_schema: json!({"type": "object"}), read_only: true }]
//!     }
//!     fn call(&self, _name: &str, args: Value) -> Result<Value, String> {
//!         Ok(args)
//!     }
//! }
//!
//! let server = Server {
//!     name: "demo".into(),
//!     version: "1.0.0".into(),
//!     instructions: None,
//!     config: McpConfig { enabled: true, ..McpConfig::default() },
//!     audit_file: None,
//!     handler: &Echo,
//! };
//! let list = server.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
//! assert_eq!(list["result"]["tools"][0]["name"], "echo");
//! let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "echo", "arguments": {"text": "hi"}}});
//! assert_eq!(server.handle(&call).unwrap()["result"]["isError"], false);
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The protocol revision this server speaks (it also accepts older clients).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The MCP server's policy (`mcp` in settings). Off, and read-only, by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct McpConfig {
    /// Answer MCP clients at all (the `--mcp` entry point refuses when off).
    pub enabled: bool,
    /// Offer tools that change data (ingest, remove). Off: read-only tools only.
    pub allow_write: bool,
    /// Tool names to offer; empty means every tool the policy allows.
    pub tools: Vec<String>,
    /// Append each call to `mcp-audit.jsonl` in the data folder.
    pub audit_log: bool,
    /// Also record the call arguments (they can contain private text).
    pub audit_arguments: bool,
    /// Largest reply text, in characters (longer replies are cut).
    pub max_result_chars: usize,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self { enabled: false, allow_write: false, tools: Vec::new(), audit_log: true, audit_arguments: false, max_result_chars: 100_000 }
    }
}

impl McpConfig {
    /// Reply size clamped; blank tool names dropped.
    pub fn validated(mut self) -> Self {
        self.max_result_chars = self.max_result_chars.clamp(1_000, 10_000_000);
        self.tools.retain(|t| !t.trim().is_empty());
        self
    }

    fn allows(&self, tool: &Tool) -> bool {
        (tool.read_only || self.allow_write) && (self.tools.is_empty() || self.tools.iter().any(|t| t == &tool.name))
    }
}

/// A tool the app offers to MCP clients.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    /// The tool's name, such as `search`.
    pub name: String,
    /// What it does, for the client's model.
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
    /// It changes nothing (only these are offered unless `allow_write` is on).
    pub read_only: bool,
}

impl Tool {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {"readOnlyHint": self.read_only, "destructiveHint": !self.read_only, "openWorldHint": false},
        })
    }
}

/// The app's side: which tools exist and what they do.
pub trait Handler {
    /// Every tool the app has (the policy picks which are offered).
    fn tools(&self) -> Vec<Tool>;
    /// Run a tool; the value is returned to the client as JSON text.
    fn call(&self, name: &str, args: Value) -> Result<Value, String>;
}

/// The protocol side: answers JSON-RPC messages with the handler's tools under the policy.
pub struct Server<'a> {
    /// Sent to the client as `serverInfo.name`.
    pub name: String,
    /// Sent to the client as `serverInfo.version`.
    pub version: String,
    /// Shown to the client's model: what this server is for.
    pub instructions: Option<String>,
    /// Which tools are offered, and auditing.
    pub config: McpConfig,
    /// Where audit lines go (`None`: no audit log).
    pub audit_file: Option<PathBuf>,
    /// The app's tools.
    pub handler: &'a dyn Handler,
}

impl Server<'_> {
    /// Serve until the client closes stdin.
    pub fn serve_stdio(&self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        self.serve(stdin.lock(), stdout.lock())
    }

    /// Serve line-delimited JSON-RPC from `input` to `output`.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Array(batch)) => {
                    let replies: Vec<Value> = batch.iter().filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
            };
            if let Some(r) = reply {
                serde_json::to_writer(&mut output, &r)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// One message; `None` for notifications (no id) and replies.
    pub fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str);
        let (Some(id), Some(method)) = (id, method) else {
            return None;
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                // Answer with the client's revision when it is one we know, else ours.
                let version = if ["2024-11-05", "2025-03-26", PROTOCOL_VERSION].contains(&asked) { asked } else { PROTOCOL_VERSION };
                let mut result = json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": self.name, "version": self.version},
                });
                if let Some(i) = &self.instructions {
                    result["instructions"] = json!(i);
                }
                ok(id, result)
            }
            "ping" => ok(id, json!({})),
            "tools/list" => ok(id, json!({"tools": self.allowed().iter().map(Tool::to_json).collect::<Vec<_>>()})),
            "tools/call" => self.call_tool(id, &params),
            other => error(id, -32601, &format!("method not found: {other}")),
        })
    }

    fn allowed(&self) -> Vec<Tool> {
        self.handler.tools().into_iter().filter(|t| self.config.allows(t)).collect()
    }

    fn call_tool(&self, id: Value, params: &Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return error(id, -32602, "tools/call needs a name");
        };
        if !self.allowed().iter().any(|t| t.name == name) {
            return error(id, -32602, &format!("unknown or disabled tool: {name}"));
        }
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let started = Instant::now();
        let result = self.handler.call(name, args.clone());
        self.audit(name, &args, result.is_ok(), started.elapsed().as_millis());
        match result {
            Ok(v) => {
                let mut text = match &v {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                if text.chars().count() > self.config.max_result_chars {
                    text = text.chars().take(self.config.max_result_chars).collect::<String>() + "\n[cut]";
                }
                let mut result = json!({"content": [{"type": "text", "text": text}], "isError": false});
                if v.is_object() {
                    result["structuredContent"] = v;
                }
                ok(id, result)
            }
            // Tool failures are results the model can read, not protocol errors.
            Err(e) => ok(id, json!({"content": [{"type": "text", "text": e}], "isError": true})),
        }
    }

    fn audit(&self, tool: &str, args: &Value, success: bool, ms: u128) {
        let Some(file) = self.audit_file.as_ref().filter(|_| self.config.audit_log) else {
            return;
        };
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut line = json!({"ts": ts, "tool": tool, "ok": success, "ms": ms});
        if self.config.audit_arguments {
            line["arguments"] = args.clone();
        }
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        if let Ok(mut f) = o.open(file) {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo;
    impl Handler for Demo {
        fn tools(&self) -> Vec<Tool> {
            vec![
                Tool { name: "echo".into(), description: "Echo".into(), input_schema: json!({"type": "object"}), read_only: true },
                Tool { name: "wipe".into(), description: "Wipe".into(), input_schema: json!({"type": "object"}), read_only: false },
            ]
        }
        fn call(&self, name: &str, args: Value) -> Result<Value, String> {
            match name {
                "echo" => Ok(args),
                _ => Err("nope".into()),
            }
        }
    }

    fn run(server: &Server, input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        server.serve(input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn server(config: McpConfig, audit: Option<PathBuf>) -> Server<'static> {
        Server { name: "demo".into(), version: "1".into(), instructions: None, config, audit_file: audit, handler: &Demo }
    }

    #[test]
    fn handshake_list_and_call() {
        let d = tempfile::tempdir().unwrap();
        let audit = d.path().join("audit.jsonl");
        let s = server(McpConfig { enabled: true, ..McpConfig::default() }, Some(audit.clone()));
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"secret":"x"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"wipe"}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#,
        ]
        .join("\n");
        let r = run(&s, &input);
        assert_eq!(r.len(), 5, "the notification gets no reply");
        assert_eq!(r[0]["result"]["protocolVersion"], "2025-03-26");
        let tools = r[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1, "write tools are hidden by default");
        assert_eq!(r[2]["result"]["structuredContent"]["secret"], "x");
        assert_eq!(r[3]["error"]["code"], -32602);
        assert_eq!(r[4]["error"]["code"], -32601);
        let log = std::fs::read_to_string(audit).unwrap();
        assert!(log.contains("\"tool\":\"echo\"") && !log.contains("secret"), "arguments stay out of the log: {log}");
    }

    #[test]
    fn write_tools_and_allowlist() {
        let s = server(McpConfig { allow_write: true, tools: vec!["wipe".into()], ..McpConfig::default() }, None);
        let r = run(&s, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"wipe\"}}");
        let tools = r[0]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "wipe");
        assert_eq!(r[1]["result"]["isError"], true);
    }
}
