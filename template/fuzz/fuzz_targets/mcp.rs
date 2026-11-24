//! The MCP server: one line of JSON-RPC from the client on stdin.
#![no_main]

use desktop_core::mcp::{Handler, McpConfig, Server, Tool};
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};

struct Echo;

impl Handler for Echo {
    fn tools(&self) -> Vec<Tool> {
        vec![Tool { name: "echo".into(), description: "Echo".into(), input_schema: json!({"type": "object"}), read_only: true }]
    }
    fn call(&self, _name: &str, args: Value) -> Result<Value, String> {
        Ok(args)
    }
}

fuzz_target!(|data: &[u8]| {
    let server = Server {
        name: "fuzz".into(),
        version: "0".into(),
        instructions: None,
        config: McpConfig::default(),
        audit_file: None,
        handler: &Echo,
        approver: None,
    };
    let mut out = Vec::new();
    server.serve(data, &mut out).unwrap();
    // Every reply is one line of JSON.
    for line in out.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        serde_json::from_slice::<Value>(line).unwrap();
    }
});
