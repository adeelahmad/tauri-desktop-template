//! What runs around every request, outermost first:
//!
//! 1. [`request_ids`]: takes the client's `x-request-id` (if it is short and plain) or makes
//!    one, logs `[req <id>] METHOD /path -> status in N ms`, and echoes it in the response.
//!    Everything logged while the request runs, errors included, carries the same ID
//!    ([`request_id`]).
//! 2. [`cors`]: only origins listed in `corsOrigins` get CORS headers (none by default);
//!    their preflight `OPTIONS` requests are answered here.
//! 3. The body-size limit (`maxBodyKb`, axum's `DefaultBodyLimit`).
//! 4. [`timeout`]: a request running longer than `timeoutSecs` is answered with `408`.
//! 5. [`auth`]: `Authorization: Bearer <token>`, compared in constant time.
//!
//! # Example
//!
//! ```
//! use desktop_core::http_api::middleware::{bearer_matches, valid_request_id};
//!
//! assert!(bearer_matches(Some("Bearer s3cret"), "s3cret"));
//! assert!(!bearer_matches(Some("Bearer wrong"), "s3cret"));
//! assert!(!bearer_matches(None, "s3cret"));
//! assert!(valid_request_id("build-42_a"));
//! assert!(!valid_request_id("has spaces"));
//! ```

use super::{ApiError, Context};
use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::time::{Duration, Instant};

/// The request ID header, in and out.
pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

tokio::task_local! {
    static REQUEST_ID: String;
}

/// The ID of the request being handled (`-` outside one).
pub fn request_id() -> String {
    REQUEST_ID.try_with(Clone::clone).unwrap_or_else(|_| "-".into())
}

/// A client-sent ID is kept if it is 1 to 64 letters, digits, `-` or `_`.
pub fn valid_request_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Give the request an ID, log it with its outcome, and return the ID in `x-request-id`.
pub async fn request_ids(req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(&X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|v| valid_request_id(v))
        .map(String::from)
        .unwrap_or_else(|| super::random_hex(8).unwrap_or_else(|_| "-".into()));
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    let start = Instant::now();
    REQUEST_ID
        .scope(id.clone(), async move {
            let mut res = next.run(req).await;
            log::info!("[req {id}] {method} {path} -> {} in {} ms", res.status().as_u16(), start.elapsed().as_millis());
            if let Ok(v) = HeaderValue::from_str(&id) {
                res.headers_mut().insert(X_REQUEST_ID, v);
            }
            res
        })
        .await
}

/// CORS for the listed origins only; others get no CORS headers (so browsers block them).
pub async fn cors(State(ctx): State<Context>, req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()).filter(|o| ctx.config().cors_origins.iter().any(|a| a == o));
    let Some(origin) = origin.and_then(|o| HeaderValue::from_str(o).ok()) else {
        return next.run(req).await;
    };
    let preflight = req.method() == Method::OPTIONS && req.headers().contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut res = if preflight { StatusCode::NO_CONTENT.into_response() } else { next.run(req).await };
    let h = res.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    h.insert(header::VARY, HeaderValue::from_static("origin"));
    h.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("x-request-id"));
    if preflight {
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, PUT, DELETE"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type, x-request-id"));
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    }
    res
}

/// Answer `408` when the request runs longer than `timeoutSecs`.
pub async fn timeout(State(ctx): State<Context>, req: Request, next: Next) -> Response {
    match tokio::time::timeout(Duration::from_secs(ctx.config().timeout_secs), next.run(req)).await {
        Ok(res) => res,
        Err(_) => ApiError::timeout().into_response(),
    }
}

/// Refuse requests without the right bearer token.
pub async fn auth(State(ctx): State<Context>, req: Request, next: Next) -> Response {
    let given = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if ctx.token().is_empty() || bearer_matches(given, ctx.token()) {
        next.run(req).await
    } else {
        ApiError::unauthorized().into_response()
    }
}

/// `Authorization` is `Bearer <token>` with this token (compared in constant time).
pub fn bearer_matches(header: Option<&str>, token: &str) -> bool {
    let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ")).map(str::trim) else {
        return false;
    };
    given.len() == token.len() && given.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}
