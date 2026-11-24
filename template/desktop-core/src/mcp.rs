//! A Model Context Protocol server over stdio (feature `mcp`), in plain Rust.
//!
//! Claude Desktop, Claude Code and other MCP clients start the app with `--mcp` and talk
//! JSON-RPC 2.0, one message per line. The app supplies the tools through `Handler`;
//! this module does the protocol, the policy and the audit log:
//! - only read-only tools are offered unless `McpConfig::allow_write` is on;
//! - `McpConfig::tools`, when not empty, is the allowlist of tool names;
//! - `McpConfig::policies` sets each tool (or `*`, every other tool) to `allow`, `ask` or
//!   `deny`. An `ask` call waits until someone approves it in the app (`Approvals`, a small
//!   file queue in the data folder that the window polls); when the app is not running, or
//!   nobody answers within `approval_timeout_secs`, the call is refused with a clear error;
//! - every call is appended to the audit log (time, tool, ok, duration; never the arguments
//!   unless `audit_arguments` is on, since they can hold private text).
//!
//! ```
//! use desktop_core::mcp::{McpConfig, Tool, ToolPolicy};
//! let mut cfg = McpConfig::default();
//! cfg.policies.insert("ask".into(), ToolPolicy::Ask);
//! let tool = |name: &str| Tool { name: name.into(), description: String::new(), input_schema: serde_json::json!({}), read_only: true };
//! assert_eq!(cfg.policy(&tool("ask")), ToolPolicy::Ask);
//! assert_eq!(cfg.policy(&tool("search")), ToolPolicy::Allow, "no entry: allowed, as before");
//! ```
//!
//! Main items: [`Server`] ([`Server::serve_stdio`], or [`Server::handle`] for one message),
//! the [`Handler`] trait the app implements, [`Tool`], [`McpConfig`] and [`Approvals`].
//!
//! A server with one tool, answering two messages:
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
//!     approver: None,
//! };
//! let list = server.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
//! assert_eq!(list["result"]["tools"][0]["name"], "echo");
//! let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "echo", "arguments": {"text": "hi"}}});
//! assert_eq!(server.handle(&call).unwrap()["result"]["isError"], false);
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What happens when a client calls a tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ToolPolicy {
    /// Run it.
    #[default]
    Allow,
    /// Run it only after someone approves the call in the app.
    Ask,
    /// Hide it and refuse calls.
    Deny,
}

/// The protocol revision this server speaks (it also accepts older clients).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The MCP server's policy (`mcp` in settings). Off, and read-only, by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// Per-tool policy by tool name; `*` covers tools without their own entry. Empty: every
    /// tool the gates above let through is allowed (the default).
    pub policies: BTreeMap<String, ToolPolicy>,
    /// How long an `ask` call waits for an answer in the app before it is refused.
    pub approval_timeout_secs: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_write: false,
            tools: Vec::new(),
            audit_log: true,
            audit_arguments: false,
            max_result_chars: 100_000,
            policies: BTreeMap::new(),
            approval_timeout_secs: 60,
        }
    }
}

impl McpConfig {
    /// The same config with every value in a safe range (called from `Config::validated`).
    pub fn validated(mut self) -> Self {
        self.max_result_chars = self.max_result_chars.clamp(1_000, 10_000_000);
        self.tools.retain(|t| !t.trim().is_empty());
        self.policies = self.policies.into_iter().map(|(k, v)| (k.trim().to_string(), v)).filter(|(k, _)| !k.is_empty()).collect();
        self.approval_timeout_secs = self.approval_timeout_secs.clamp(5, 600);
        self
    }

    /// The policy for `tool`. The older gates come first: a write tool without
    /// `allow_write`, or a tool missing from a non-empty `tools` list, is `Deny` whatever
    /// `policies` says. Then the tool's own entry, then `*`, then `Allow`.
    pub fn policy(&self, tool: &Tool) -> ToolPolicy {
        let gated = (tool.read_only || self.allow_write) && (self.tools.is_empty() || self.tools.iter().any(|t| t == &tool.name));
        if !gated {
            return ToolPolicy::Deny;
        }
        self.policies.get(&tool.name).or_else(|| self.policies.get("*")).copied().unwrap_or_default()
    }

    /// Whether `tool` was allowed by name: listed in `tools`, or given its own `policies`
    /// entry other than `deny`. Opt-in tools (the key-value tools, say) are offered only then.
    pub fn names(&self, tool: &str) -> bool {
        self.tools.iter().any(|t| t == tool) || self.policies.get(tool).is_some_and(|p| *p != ToolPolicy::Deny)
    }
}

/// How an `ask` call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    /// Someone allowed the call.
    Approved,
    /// Someone refused the call.
    Denied,
    /// Nobody answered in time.
    TimedOut,
    /// The app is not running, so nobody can answer.
    NoApp,
    /// The queue could not be used (the message says why).
    Failed(String),
}

/// Decides `ask` calls. `Approvals` is the real one; tests use their own.
pub trait Approver {
    /// Wait up to `timeout` for a decision on a call of `tool` with `args`.
    fn approve(&self, tool: &str, args: &Value, timeout: Duration) -> Approval;
}

/// A call waiting for approval, as the app shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PendingCall {
    /// Identifies the call (pass it to the decision).
    pub id: String,
    /// The tool asked for.
    pub tool: String,
    /// The arguments as JSON text, cut to 2000 characters.
    pub arguments: String,
    /// Unix seconds when the call arrived, and when it is refused unless answered.
    pub asked: u64,
    /// Unix seconds when the call is refused unless answered.
    pub deadline: u64,
}

/// The approval queue: one small file per waiting call in `<data>/mcp-approvals`, so the
/// MCP process (started by the client) and the app (or its worker) share it with no socket.
/// The app writes `app-alive` every few seconds while it runs (`heartbeat`); without a
/// fresh one, `ask` calls are refused at once instead of waiting.
///
/// ```
/// use desktop_core::mcp::{Approval, Approvals, Approver};
/// use std::time::Duration;
/// let dir = tempfile::tempdir().unwrap();
/// let q = Approvals::new(dir.path());
/// // The app is not running: refused at once.
/// assert_eq!(q.approve("ask", &serde_json::json!({}), Duration::from_secs(5)), Approval::NoApp);
/// q.heartbeat().unwrap();
/// assert!(q.app_running());
/// assert!(q.pending().is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct Approvals {
    dir: PathBuf,
}

/// The app counts as running while its heartbeat is younger than this.
pub const HEARTBEAT_MAX_AGE: Duration = Duration::from_secs(15);

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Write `bytes` to `path` through a temporary file, owner-only on Unix.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(&tmp)?.write_all(bytes)?;
    std::fs::rename(tmp, path)
}

impl Approvals {
    /// The queue under `data_dir` (the folder is made on first use).
    pub fn new(data_dir: &Path) -> Self {
        Self { dir: data_dir.join("mcp-approvals") }
    }

    /// Mark the app as running (call every few seconds from the app).
    pub fn heartbeat(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_private(&self.dir.join("app-alive"), now_secs().to_string().as_bytes())
    }

    /// Mark the app as stopped (on quit), so calls are refused at once.
    pub fn stopped(&self) {
        let _ = std::fs::remove_file(self.dir.join("app-alive"));
    }

    /// Whether the app wrote a heartbeat in the last `HEARTBEAT_MAX_AGE`.
    pub fn app_running(&self) -> bool {
        let beat = std::fs::read_to_string(self.dir.join("app-alive")).ok().and_then(|t| t.trim().parse::<u64>().ok());
        beat.is_some_and(|t| now_secs().saturating_sub(t) <= HEARTBEAT_MAX_AGE.as_secs())
    }

    /// Calls waiting for an answer, oldest first (expired ones are left out and removed).
    pub fn pending(&self) -> Vec<PendingCall> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let now = now_secs();
        let mut out: Vec<PendingCall> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".call"))
            .filter_map(|e| {
                let call: PendingCall = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
                if call.deadline < now {
                    let _ = std::fs::remove_file(e.path());
                    return None;
                }
                Some(call)
            })
            .collect();
        out.sort_by(|a, b| (a.asked, &a.id).cmp(&(b.asked, &b.id)));
        out
    }

    /// Answer a waiting call. Fails when the id is unknown or the call already ended.
    pub fn decide(&self, id: &str, allow: bool) -> Result<(), String> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("not a call id".into());
        }
        if !self.dir.join(format!("{id}.call")).is_file() {
            return Err("that call is no longer waiting".into());
        }
        write_private(&self.dir.join(format!("{id}.answer")), if allow { b"allow" } else { b"deny" }).map_err(|e| e.to_string())
    }

    fn new_id() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        format!("{nanos:x}{:x}{:x}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
    }
}

impl Approver for Approvals {
    fn approve(&self, tool: &str, args: &Value, timeout: Duration) -> Approval {
        if !self.app_running() {
            return Approval::NoApp;
        }
        let id = Self::new_id();
        let asked = now_secs();
        let call = PendingCall { id: id.clone(), tool: tool.into(), arguments: args.to_string().chars().take(2000).collect(), asked, deadline: asked + timeout.as_secs() };
        let (call_file, answer_file) = (self.dir.join(format!("{id}.call")), self.dir.join(format!("{id}.answer")));
        let written = std::fs::create_dir_all(&self.dir).and_then(|()| write_private(&call_file, &serde_json::to_vec(&call).unwrap_or_default()));
        if let Err(e) = written {
            return Approval::Failed(format!("approval queue: {e}"));
        }
        let started = Instant::now();
        let result = loop {
            if let Ok(a) = std::fs::read_to_string(&answer_file) {
                break if a.trim() == "allow" { Approval::Approved } else { Approval::Denied };
            }
            if started.elapsed() >= timeout {
                break Approval::TimedOut;
            }
            if !self.app_running() {
                break Approval::NoApp;
            }
            std::thread::sleep(Duration::from_millis(150));
        };
        let _ = std::fs::remove_file(&call_file);
        let _ = std::fs::remove_file(&answer_file);
        result
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
    /// Decides `ask` calls; with none, every `ask` call is refused.
    pub approver: Option<&'a dyn Approver>,
}

impl Server<'_> {
    /// Serve until the client closes stdin.
    pub fn serve_stdio(&self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        self.serve(stdin.lock(), stdout.lock())
    }

    /// Serve line-delimited JSON-RPC from `input` to `output`.
    ///
    /// The input is untrusted: a line over
    /// [`limits::MCP_LINE_BYTES`](crate::untrusted::limits::MCP_LINE_BYTES) is dropped with a
    /// parse error, and a panic while answering a message becomes an internal error reply.
    pub fn serve(&self, mut input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        let max = crate::untrusted::limits::MCP_LINE_BYTES;
        while let Some(line) = crate::untrusted::read_line_capped(&mut input, max)? {
            let reply = match line {
                Ok(line) => self.reply(&line),
                Err(len) => Some(error(Value::Null, -32700, &format!("parse error: the message is {len} bytes; the limit is {max}"))),
            };
            if let Some(r) = reply {
                serde_json::to_writer(&mut output, &r)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// The reply to one line of input (`None` for blank lines and notifications).
    pub fn reply(&self, line: &[u8]) -> Option<Value> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.trim_ascii().is_empty() {
            return None;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> = batch.iter().filter_map(|m| self.handle_guarded(m)).collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg) => self.handle_guarded(&msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        }
    }

    /// [`Server::handle`], with a panic (in a tool, say) answered as an internal error.
    fn handle_guarded(&self, msg: &Value) -> Option<Value> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(msg)))
            .unwrap_or_else(|_| msg.get("id").cloned().map(|id| error(id, -32603, "internal error: the server failed on this message")))
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
        self.handler.tools().into_iter().filter(|t| self.config.policy(t) != ToolPolicy::Deny).collect()
    }

    fn call_tool(&self, id: Value, params: &Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return error(id, -32602, "tools/call needs a name");
        };
        let Some(tool) = self.allowed().into_iter().find(|t| t.name == name) else {
            return error(id, -32602, &format!("unknown or disabled tool: {name}"));
        };
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let started = Instant::now();
        if self.config.policy(&tool) == ToolPolicy::Ask {
            let timeout = Duration::from_secs(self.config.approval_timeout_secs);
            let refused = match self.approver.map(|a| a.approve(name, &args, timeout)) {
                Some(Approval::Approved) => None,
                Some(Approval::Denied) => Some(format!("The user declined this {name} call.")),
                Some(Approval::TimedOut) => Some(format!("Nobody approved this {name} call within {} seconds, so it was refused.", timeout.as_secs())),
                Some(Approval::NoApp) | None => {
                    Some(format!("The {name} tool needs approval in the app, and the app is not running. Open it and try again."))
                }
                Some(Approval::Failed(e)) => Some(format!("The {name} call could not be put up for approval ({e}), so it was refused.")),
            };
            if let Some(text) = refused {
                self.audit(name, &args, false, started.elapsed().as_millis());
                return ok(id, json!({"content": [{"type": "text", "text": text}], "isError": true}));
            }
        }
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
        Server { name: "demo".into(), version: "1".into(), instructions: None, config, audit_file: audit, handler: &Demo, approver: None }
    }

    fn tool(name: &str, read_only: bool) -> Tool {
        Tool { name: name.into(), description: String::new(), input_schema: json!({}), read_only }
    }

    #[test]
    fn opt_in_tools_need_their_name() {
        let mut c = McpConfig::default();
        assert!(!c.names("kv_get"));
        c.policies.insert("*".into(), ToolPolicy::Allow);
        assert!(!c.names("kv_get"), "a wildcard does not name it");
        c.policies.insert("kv_get".into(), ToolPolicy::Deny);
        assert!(!c.names("kv_get"));
        c.policies.insert("kv_get".into(), ToolPolicy::Ask);
        assert!(c.names("kv_get"));
        assert!(McpConfig { tools: vec!["kv_list".into()], ..McpConfig::default() }.names("kv_list"));
    }

    #[test]
    fn policy_resolution() {
        let mut c = McpConfig::default();
        assert_eq!(c.policy(&tool("echo", true)), ToolPolicy::Allow, "empty map: allowed");
        assert_eq!(c.policy(&tool("wipe", false)), ToolPolicy::Deny, "write tools stay behind allowWrite");
        c.policies.insert("wipe".into(), ToolPolicy::Allow);
        assert_eq!(c.policy(&tool("wipe", false)), ToolPolicy::Deny, "a policy cannot lift the write gate");
        c.allow_write = true;
        c.policies.insert("*".into(), ToolPolicy::Ask);
        c.policies.insert("echo".into(), ToolPolicy::Deny);
        assert_eq!(c.policy(&tool("wipe", false)), ToolPolicy::Allow, "own entry beats *");
        assert_eq!(c.policy(&tool("echo", true)), ToolPolicy::Deny);
        assert_eq!(c.policy(&tool("other", true)), ToolPolicy::Ask, "* covers the rest");
        c.tools = vec!["wipe".into()];
        assert_eq!(c.policy(&tool("other", true)), ToolPolicy::Deny, "the allowlist still applies");
        let parsed: McpConfig = serde_json::from_str(r#"{"policies": {" ask ": "ask", "": "deny"}, "approvalTimeoutSecs": 1}"#).unwrap();
        let parsed = parsed.validated();
        assert_eq!(parsed.policies.len(), 1);
        assert_eq!(parsed.policies["ask"], ToolPolicy::Ask);
        assert_eq!(parsed.approval_timeout_secs, 5);
    }

    struct Fixed(Approval);
    impl Approver for Fixed {
        fn approve(&self, _: &str, _: &Value, _: Duration) -> Approval {
            self.0.clone()
        }
    }

    #[test]
    fn ask_waits_for_the_approver() {
        let mut c = McpConfig::default();
        c.policies.insert("echo".into(), ToolPolicy::Ask);
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"a":1}}}"#;
        let r = run(&server(c.clone(), None), call);
        assert_eq!(r[0]["result"]["isError"], true, "no approver: refused");
        assert!(r[0]["result"]["content"][0]["text"].as_str().unwrap().contains("not running"));
        for (answer, refused) in [(Approval::Approved, false), (Approval::Denied, true), (Approval::TimedOut, true), (Approval::NoApp, true)] {
            let a = Fixed(answer);
            let s = Server { approver: Some(&a), ..server(c.clone(), None) };
            let r = run(&s, call);
            assert_eq!(r[0]["result"]["isError"], refused, "{:?}", a.0);
        }
        c.policies.insert("echo".into(), ToolPolicy::Deny);
        let r = run(&server(c, None), r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        assert!(r[0]["result"]["tools"].as_array().unwrap().is_empty(), "denied tools are hidden");
    }

    #[test]
    fn approval_queue_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let q = Approvals::new(d.path());
        assert_eq!(q.approve("x", &json!({}), Duration::from_secs(5)), Approval::NoApp);
        q.heartbeat().unwrap();
        let waiter = {
            let q = q.clone();
            std::thread::spawn(move || q.approve("ask", &json!({"question": "q"}), Duration::from_secs(20)))
        };
        let call = loop {
            if let Some(c) = q.pending().pop() {
                break c;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(call.tool, "ask");
        assert!(call.arguments.contains("question"));
        assert!(q.decide("../etc", true).is_err());
        q.decide(&call.id, true).unwrap();
        assert_eq!(waiter.join().unwrap(), Approval::Approved);
        assert!(q.pending().is_empty());
        assert!(q.decide(&call.id, false).is_err(), "already answered");
        // A short wait with no answer times out; a stopped app refuses at once.
        assert_eq!(q.approve("ask", &json!({}), Duration::from_millis(300)), Approval::TimedOut);
        q.stopped();
        assert!(!q.app_running());
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

    #[test]
    fn oversized_lines_and_panics_are_answered_not_fatal() {
        struct Panics;
        impl Handler for Panics {
            fn tools(&self) -> Vec<Tool> {
                vec![tool("boom", true)]
            }
            fn call(&self, _: &str, _: Value) -> Result<Value, String> {
                panic!("tool bug")
            }
        }
        let s = Server { handler: &Panics, ..server(McpConfig::default(), None) };
        let big = String::from(r#"{"pad": ""#) + &"x".repeat(crate::untrusted::limits::MCP_LINE_BYTES) + r#""}"#;
        let input = format!("{big}\n{}\n{}\n", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params": {"name": "boom"} }"#, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#);
        let replies = run(&s, &input);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["error"]["code"], -32603);
        assert_eq!(replies[2]["id"], 2);
    }
}
