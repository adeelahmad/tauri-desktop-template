//! The `{{name}}` command. From the UI (typed IPC: `kitchen generate command` also updates
//! the UI's TypeScript bindings, see docs/GUIDE.md):
//!
//! ```js
//! const reply = await api.{{name}}({ input: { text: 'hello' } }); // import { api } from './ipc.js'
//! ```

use serde_json::{json, Value};

/// The work itself, kept free of Tauri so it is easy to test.
pub fn handle(input: &Value) -> Result<Value, String> {
    Ok(json!({ "echo": input }))
}

/// Runs in the backend (this process or the worker), with the engine at hand.
pub fn run(engine: &desktop_core::Engine, input: Value) -> Result<Value, String> {
    let _ = engine; // search, ingest, config ... are all on the engine
    handle(&input)
}

/// The command the UI invokes.
#[tauri::command(async)]
pub fn {{name}}(state: tauri::State<crate::AppState>, input: Value) -> Result<Value, String> {
    crate::call(&state, "{{name}}", input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echoes() {
        assert_eq!(handle(&json!(1)).unwrap(), json!({ "echo": 1 }));
    }
}
