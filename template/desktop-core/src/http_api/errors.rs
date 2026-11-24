//! The API's one error shape. Every failure, from a missing token to a backend error, is
//! answered with the same JSON body and a fitting status code:
//!
//! ```text
//! {"error": {"code": "<machine-readable>", "message": "<for people>", "requestId": "<id>",
//!            "fields": {"<field>": ["<problem>", ...]}}}
//! ```
//!
//! `fields` is there only for validation errors. `requestId` matches the `x-request-id`
//! response header and the server's log lines, so a report can be traced.
//!
//! # Example
//!
//! ```
//! use axum::response::IntoResponse;
//! use desktop_core::http_api::{ApiError, FieldErrors};
//!
//! let mut fields = FieldErrors::default();
//! fields.add("query", "must not be empty");
//! let e = ApiError::validation(fields);
//! assert_eq!(e.status().as_u16(), 422);
//! assert_eq!(e.body()["error"]["code"], "validation");
//! assert_eq!(e.body()["error"]["fields"]["query"][0], "must not be empty");
//!
//! assert_eq!(ApiError::backend("Still starting; try again in a moment.").status().as_u16(), 503);
//! assert_eq!(ApiError::unauthorized().into_response().status().as_u16(), 401);
//! ```

use super::extract::FieldErrors;
use super::middleware::request_id;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

/// An API error: a status, a stable `code`, a message for people and, for validation
/// errors, the problem with each field.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    fields: FieldErrors,
}

impl ApiError {
    /// Any error.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), fields: FieldErrors::default() }
    }

    /// 400: the request could not be read.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// 401: no token, or the wrong one.
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "send the API token as `Authorization: Bearer <token>`")
    }

    /// 404: no such route.
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "no such route")
    }

    /// 405: the route exists, but not for this method.
    pub fn method_not_allowed() -> Self {
        Self::new(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "this route does not take that method")
    }

    /// 408: the request took longer than `timeoutSecs`.
    pub fn timeout() -> Self {
        Self::new(StatusCode::REQUEST_TIMEOUT, "timeout", "the request took too long")
    }

    /// 422: some fields are wrong; `fields` says which and why.
    pub fn validation(fields: FieldErrors) -> Self {
        Self { fields, ..Self::new(StatusCode::UNPROCESSABLE_ENTITY, "validation", "check the highlighted fields") }
    }

    /// 500: something broke on our side.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// A backend method's error text: 503 while the engine is starting, 501 for a method
    /// this build does not have, 500 otherwise (the text is meant for people).
    pub fn backend(message: impl Into<String>) -> Self {
        let message = message.into();
        if message.starts_with("Still starting") {
            Self::new(StatusCode::SERVICE_UNAVAILABLE, "not_ready", message)
        } else if message.starts_with("unknown method") {
            Self::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", message)
        } else {
            Self::new(StatusCode::INTERNAL_SERVER_ERROR, "backend", message)
        }
    }

    /// The HTTP status.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The machine-readable code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// The JSON body, with the current request's ID.
    pub fn body(&self) -> Value {
        let mut e = json!({"code": self.code, "message": self.message, "requestId": request_id()});
        if !self.fields.is_empty() {
            e["fields"] = json!(self.fields);
        }
        json!({ "error": e })
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", self.status.as_u16(), self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let id = request_id();
        if self.status.is_server_error() {
            log::warn!("[req {id}] {self}");
        } else {
            log::debug!("[req {id}] {self}");
        }
        let mut res = (self.status, axum::Json(self.body())).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            res.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        res
    }
}

impl From<axum::extract::rejection::JsonRejection> for ApiError {
    fn from(r: axum::extract::rejection::JsonRejection) -> Self {
        let code = match r.status() {
            StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
            StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
            _ => "invalid_json",
        };
        Self::new(r.status(), code, r.body_text())
    }
}

impl From<axum::extract::rejection::QueryRejection> for ApiError {
    fn from(r: axum::extract::rejection::QueryRejection) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_query", r.body_text())
    }
}

impl From<axum::extract::rejection::PathRejection> for ApiError {
    fn from(r: axum::extract::rejection::PathRejection) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_path", r.body_text())
    }
}

impl From<axum::extract::rejection::BytesRejection> for ApiError {
    fn from(r: axum::extract::rejection::BytesRejection) -> Self {
        let code = if r.status() == StatusCode::PAYLOAD_TOO_LARGE { "payload_too_large" } else { "bad_request" };
        Self::new(r.status(), code, r.body_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_errors_map_to_statuses() {
        assert_eq!(ApiError::backend("unknown method \"x\"").status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(ApiError::backend("disk full").code(), "backend");
    }

    #[test]
    fn the_shape_has_no_fields_unless_validation() {
        let b = ApiError::not_found().body();
        assert_eq!(b["error"]["code"], "not_found");
        assert!(b["error"].get("fields").is_none());
        assert!(b["error"]["requestId"].is_string());
    }
}
