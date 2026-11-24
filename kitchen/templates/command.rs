//! The `{{name}}` command. From the UI (typed IPC: `kitchen generate command` also updates
//! the UI's TypeScript bindings, see docs/GUIDE.md):
//!
//! ```js
//! const reply = await api.{{name}}({ input: { text: 'hello' } }); // import { api } from './ipc.js'
//! ```
//!
//! A failure rejects the UI's promise with a `CommandError`, `{ code, message, details? }`
//! (src-tauri/src/error.rs; the codes are in docs/GUIDE.md, "Command errors"). Return
//! `CommandError::invalid("...")` for bad input; `?` turns a `desktop_core::Error` into its
//! own code, and a `String` into `failed`.

use crate::error::CommandError;
use serde_json::{json, Value};

/// The work itself, kept free of Tauri so it is easy to test.
pub fn handle(input: &Value) -> Result<Value, CommandError> {
    if input.is_null() {
        return Err(CommandError::invalid("give an input"));
    }
    Ok(json!({ "echo": input }))
}

/// Runs in the backend (this process or the worker), with the engine at hand.
pub fn run(engine: &desktop_core::Engine, input: Value) -> Result<Value, CommandError> {
    let _ = engine; // search, ingest, config ... are all on the engine
    handle(&input)
}

/// The command the UI invokes.
#[tauri::command(async)]
pub fn {{name}}(state: tauri::State<crate::AppState>, input: Value) -> Result<Value, CommandError> {
    crate::call(&state, "{{name}}", input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echoes() {
        assert_eq!(handle(&json!(1)).unwrap(), json!({ "echo": 1 }));
    }

    #[test]
    fn no_input_is_invalid() {
        assert_eq!(handle(&Value::Null).unwrap_err().code, "invalid");
    }
}
