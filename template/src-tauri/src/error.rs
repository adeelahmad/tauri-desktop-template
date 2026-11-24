//! The one error shape for Tauri commands.
//!
//! A command that fails rejects the UI's `invoke` promise with this JSON:
//!
//! ```json
//! { "code": "not_found", "message": "no document named notes.txt", "details": { "kind": "NotFound" } }
//! ```
//!
//! `code` is a short, stable, snake_case word the UI can branch on (the table is in
//! [`codes`] and in docs/GUIDE.md, "Command errors"); `message` is for people; `details` is
//! optional extra data for code (left out of the JSON when there is none). In the UI,
//! `errorText(e)` from `errors.js` turns either this or a plain string into text.
//!
//! Every command returns `Result<T, CommandError>`. `?` converts:
//!
//! - a [`desktop_core::Error`], by variant (see [`codes`]: `Io` becomes `not_found`,
//!   `denied` or `io`, `Store` becomes `store`, `NotBuilt` becomes `not_built`, ...);
//! - a `String` or `&str` (the helpers that still return `Result<_, String>`), as `failed`;
//! - a `serde_json::Error` (a value that is not the JSON expected), as `invalid`.
//!
//! The backend (`service::Service`) returns the same type, and the worker process sends it
//! over its socket unchanged, so a code set in the backend reaches the UI. Going the other
//! way, a `CommandError` turns into its `message` with `?` in a function that returns
//! `Result<_, String>` (the MCP server and the HTTP API do this).
//!
//! ```
//! use app_lib::error::{codes, CommandError};
//!
//! fn find(name: &str) -> Result<u32, CommandError> {
//!     if name.is_empty() {
//!         return Err(CommandError::new(codes::INVALID, "give a name"));
//!     }
//!     let n: u32 = Err::<u32, String>("store is closed".into())?;
//!     Ok(n)
//! }
//! assert_eq!(serde_json::to_value(find("").unwrap_err()).unwrap(), serde_json::json!({"code": "invalid", "message": "give a name"}));
//! assert_eq!(find("x").unwrap_err().code, "failed");
//!
//! let e = CommandError::from(desktop_core::Error::Store("disk full".into()));
//! assert_eq!((e.code.as_str(), e.message.as_str()), ("store", "store: disk full"));
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The stable error codes. They never change meaning once released; add new ones at the end
/// of the table in docs/GUIDE.md.
///
/// | Code | Meaning | From |
/// |------|---------|------|
/// | `invalid` | The arguments are wrong: missing, the wrong type, or not allowed. | `serde_json::Error`, bad arguments, unknown method |
/// | `not_found` | A file, profile, provider or other named thing does not exist. | `Error::Io` with kind `NotFound` |
/// | `denied` | The OS or a policy said no. | `Error::Io` with kind `PermissionDenied` |
/// | `not_ready` | The backend is still starting; try again in a moment. | the engine is not open yet |
/// | `unavailable` | The feature is turned off in Settings, or not set up or downloaded yet. | the commands themselves |
/// | `not_built` | The part is not compiled into this build (its cargo feature is off). | `Error::NotBuilt` |
/// | `io` | Reading or writing a file failed. | other `Error::Io` |
/// | `bad_data` | A file or a message was not the JSON expected. | `Error::Json` |
/// | `embedding` | The embedding model could not load or run. | `Error::Embed` |
/// | `store` | The store (file or Postgres) refused or failed. | `Error::Store` |
/// | `source` | A file, folder, recording or URL could not be used. | `Error::Source` |
/// | `ai` | The language model call failed, or AI is switched off. | `Error::Llm` |
/// | `mail` | Sending email or logging in to the mail server failed. | `mail::MailError` |
/// | `oauth` | Signing in with a provider failed. | `oauth::OauthError` |
/// | `cloud_sync` | rclone could not be installed, configured or run. | `rclone::RcloneError` |
/// | `tailscale` | Tailscale could not be reached or changed. | `tailscale::TailscaleError` |
/// | `failed` | Anything else. | `String`, `&str` |
pub mod codes {
    /// The arguments are wrong: missing, the wrong type, or not allowed.
    pub const INVALID: &str = "invalid";
    /// A file, profile, provider or other named thing does not exist.
    pub const NOT_FOUND: &str = "not_found";
    /// The OS or a policy said no.
    pub const DENIED: &str = "denied";
    /// The backend is still starting; try again in a moment.
    pub const NOT_READY: &str = "not_ready";
    /// The feature is turned off in Settings, or not set up or downloaded yet.
    pub const UNAVAILABLE: &str = "unavailable";
    /// The part is not compiled into this build (its cargo feature is off).
    pub const NOT_BUILT: &str = "not_built";
    /// Reading or writing a file failed.
    pub const IO: &str = "io";
    /// A file or a message was not the JSON expected.
    pub const BAD_DATA: &str = "bad_data";
    /// The embedding model could not load or run.
    pub const EMBEDDING: &str = "embedding";
    /// The store (file or Postgres) refused or failed.
    pub const STORE: &str = "store";
    /// A file, folder, recording or URL could not be used.
    pub const SOURCE: &str = "source";
    /// The language model call failed, or AI is switched off.
    pub const AI: &str = "ai";
    /// Sending email or logging in to the mail server failed.
    pub const MAIL: &str = "mail";
    /// Signing in with a provider failed.
    pub const OAUTH: &str = "oauth";
    /// rclone could not be installed, configured or run.
    pub const CLOUD_SYNC: &str = "cloud_sync";
    /// Tailscale could not be reached or changed.
    pub const TAILSCALE: &str = "tailscale";
    /// Anything else.
    pub const FAILED: &str = "failed";
}

/// A command error as the UI receives it: `{ "code": ..., "message": ..., "details"?: ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CommandError {
    /// Short, stable, snake_case kind, one of [`codes`].
    pub code: String,
    /// What went wrong, for people.
    pub message: String,
    /// Extra data for code (an I/O error's kind, the part that is not built, ...); left out
    /// of the JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub details: Option<Value>,
}

impl CommandError {
    /// An error with this code and message, and no details.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), details: None }
    }

    /// The same error with `details` attached.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Code `invalid`: the arguments are wrong.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID, message)
    }

    /// Code `unavailable`: turned off in Settings, or not set up yet.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(codes::UNAVAILABLE, message)
    }

    /// Code `not_ready`: the backend is still starting.
    pub fn not_ready() -> Self {
        Self::new(codes::NOT_READY, "Still starting; try again in a moment.")
    }
}

/// Only the message, so logs and the MCP and HTTP replies read as before.
impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::new(codes::FAILED, message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::new(codes::FAILED, message)
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(e.to_string())
    }
}

/// By kind: `not_found`, `denied` or `io`, with the kind in `details`.
impl From<std::io::Error> for CommandError {
    fn from(e: std::io::Error) -> Self {
        let code = match e.kind() {
            std::io::ErrorKind::NotFound => codes::NOT_FOUND,
            std::io::ErrorKind::PermissionDenied => codes::DENIED,
            _ => codes::IO,
        };
        Self::new(code, e.to_string()).with_details(json!({ "kind": format!("{:?}", e.kind()) }))
    }
}

/// The error types of the optional parts each get their own code.
macro_rules! part_errors {
    ($($feature:literal: $ty:ty => $code:expr;)*) => {$(
        #[cfg(feature = $feature)]
        impl From<$ty> for CommandError {
            fn from(e: $ty) -> Self {
                Self::new($code, e.to_string())
            }
        }
    )*};
}

part_errors! {
    "mail": desktop_core::mail::MailError => codes::MAIL;
    "oauth": desktop_core::oauth::OauthError => codes::OAUTH;
    "rclone": desktop_core::rclone::RcloneError => codes::CLOUD_SYNC;
    "tailscale": desktop_core::tailscale::TailscaleError => codes::TAILSCALE;
}

impl From<desktop_core::Error> for CommandError {
    fn from(e: desktop_core::Error) -> Self {
        use desktop_core::Error as E;
        let message = e.to_string();
        match e {
            E::Io(io) => io.into(),
            E::Json(_) => Self::new(codes::BAD_DATA, message),
            E::Embed(_) => Self::new(codes::EMBEDDING, message),
            E::Store(_) => Self::new(codes::STORE, message),
            E::Source(_) => Self::new(codes::SOURCE, message),
            E::Llm(_) => Self::new(codes::AI, message),
            E::NotBuilt(part) => Self::new(codes::NOT_BUILT, message).with_details(json!({ "part": part })),
        }
    }
}

/// Back to a plain message, for code that still returns `Result<_, String>`.
impl From<CommandError> for String {
    fn from(e: CommandError) -> Self {
        e.message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::Error as E;

    fn code(e: E) -> String {
        CommandError::from(e).code
    }

    #[test]
    fn core_errors_map_to_stable_codes() {
        let io = |k| E::Io(std::io::Error::new(k, "x"));
        assert_eq!(code(io(std::io::ErrorKind::NotFound)), "not_found");
        assert_eq!(code(io(std::io::ErrorKind::PermissionDenied)), "denied");
        assert_eq!(code(io(std::io::ErrorKind::Other)), "io");
        assert_eq!(code(E::Json(serde_json::from_str::<u8>("x").unwrap_err())), "bad_data");
        assert_eq!(code(E::Embed("x".into())), "embedding");
        assert_eq!(code(E::Store("x".into())), "store");
        assert_eq!(code(E::Source("x".into())), "source");
        assert_eq!(code(E::Llm("x".into())), "ai");
        assert_eq!(code(E::NotBuilt("pdf")), "not_built");
    }

    #[test]
    fn details_are_left_out_when_empty_and_kept_when_set() {
        assert_eq!(serde_json::to_value(CommandError::invalid("no")).unwrap(), json!({"code": "invalid", "message": "no"}));
        let e = CommandError::from(E::NotBuilt("pdf"));
        assert_eq!(serde_json::to_value(&e).unwrap(), json!({"code": "not_built", "message": "pdf is not in this build", "details": {"part": "pdf"}}));
    }

    #[test]
    fn round_trips_through_json_and_back_to_a_message() {
        let e = CommandError::unavailable("Speech is turned off in Settings").with_details(json!(1));
        let back: CommandError = serde_json::from_value(serde_json::to_value(&e).unwrap()).unwrap();
        assert_eq!(back, e);
        assert_eq!(e.to_string(), "Speech is turned off in Settings");
        assert_eq!(String::from(e), "Speech is turned off in Settings");
        assert_eq!(CommandError::from("x").code, "failed");
        assert_eq!(CommandError::not_ready().code, "not_ready");
    }
}
