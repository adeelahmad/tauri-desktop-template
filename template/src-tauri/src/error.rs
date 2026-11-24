//! The one error shape for Tauri commands.
//!
//! A command that fails rejects the UI's `invoke` promise with this JSON:
//!
//! ```json
//! { "code": "not_found", "message": "no document named notes.txt" }
//! ```
//!
//! `code` is a short, stable, snake_case word the UI can branch on (`invalid`,
//! `not_found`, `unavailable`, `denied`, `failed`); `message` is for people. In the UI,
//! `errorText(e)` from `ui-react/src/errors.js` turns either this or a plain string into
//! text. New commands return `Result<T, CommandError>`; a `String` error converts with `?`
//! (code `failed`), so the backend's `Result<_, String>` helpers keep working.
//!
//! ```
//! use app_lib::error::CommandError;
//!
//! fn find(name: &str) -> Result<u32, CommandError> {
//!     if name.is_empty() {
//!         return Err(CommandError::new("invalid", "give a name"));
//!     }
//!     let n: u32 = Err::<u32, String>("store is closed".into())?;
//!     Ok(n)
//! }
//! assert_eq!(serde_json::to_value(find("").unwrap_err()).unwrap(), serde_json::json!({"code": "invalid", "message": "give a name"}));
//! assert_eq!(find("x").unwrap_err().code, "failed");
//! ```

use serde::Serialize;

/// A command error as the UI receives it: `{ "code": ..., "message": ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    /// Short, stable, snake_case kind: `invalid`, `not_found`, `unavailable`, `denied`, `failed`.
    pub code: String,
    /// What went wrong, for people.
    pub message: String,
}

impl CommandError {
    /// An error with this code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::new("failed", message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::new("failed", message)
    }
}

impl From<desktop_core::Error> for CommandError {
    fn from(e: desktop_core::Error) -> Self {
        Self::new("failed", e.to_string())
    }
}
