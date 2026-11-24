//! The routes. Each one checks its input and calls one backend method; the backend does
//! the work, exactly as when the window asks.
//!
//! | Route | Backend method | |
//! |---|---|---|
//! | `GET /health` | `status` | `{"status": "ok", "store": {..}}`, `"starting"` before the store is open, `503` if it fails |
//! | `GET /v1/features` | | the parts built into this app |
//! | `GET /v1/status` | `status` | the store: backend, model, chunks |
//! | `POST /v1/search` | `search` | `{query, page?, perPage?, search?}` → a [`Paginated`] list of hits |
//! | `GET /v1/settings` | `config` | the settings |
//! | `PUT /v1/settings` | `configure` | a whole settings object; it is validated like Settings does |
//! | `POST /v1/documents` | `ingest_text` | `{source, text}` → `{source, chunks}` (`201`) |
//! | `DELETE /v1/documents?source=` | `remove_source` | → `{removed}` |
//! | `POST /v1/ask` (feature `ai`) | `ask` | `{question}` → the answer with its sources |
//! | `GET /v1/jobs` (feature `jobs-queue`) | `jobs_list` | the queue, dead letters, recent runs, job types |
//! | `POST /v1/jobs` (feature `jobs-queue`) | `jobs_enqueue` | `{kind, payload?, delaySecs?, runAt?, maxAttempts?, timeoutSecs?}` (`201`) |
//! | `POST /v1/jobs/{id}/cancel`, `.../retry` (feature `jobs-queue`) | `jobs_cancel`, `jobs_retry` | |
//!
//! To add a route for a backend method, run `kitchen generate route <name>` (it adds
//! `POST /v1/<name>` at the `kitchen:routes` marker), or write a handler like the ones here.
//!
//! # Example
//!
//! [`method`] turns any backend method into a route that takes its params as a JSON body:
//!
//! ```
//! use desktop_core::http_api::{routes, Context, HttpApiConfig};
//! use serde_json::{json, Value};
//! use std::sync::Arc;
//!
//! let ctx = Context::new(Arc::new(|_: &str, p: Value| Ok(p)), "token", HttpApiConfig::default(), json!({}));
//! let app: axum::Router = routes::routes().route("/v1/echo", routes::method("echo")).with_state(ctx);
//! # let _ = app;
//! ```

use super::{middleware, ApiError, Context, FieldErrors, Page, Paginated, Validate, ValidJson, ValidQuery};
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
{% if background_jobs %}#[cfg(feature = "jobs-queue")]
use axum::extract::{rejection::PathRejection, Path};
{% endif %}use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{get, post, MethodRouter};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

/// The whole app: [`routes`] with the error fallbacks and the middleware stack.
pub fn router(ctx: Context) -> Router {
    let limit = usize::try_from(ctx.config().max_body_kb * 1024).unwrap_or(usize::MAX);
    routes()
        .fallback(|| async { ApiError::not_found() })
        .method_not_allowed_fallback(|| async { ApiError::method_not_allowed() })
        .layer(from_fn_with_state(ctx.clone(), middleware::auth))
        .layer(from_fn_with_state(ctx.clone(), middleware::timeout))
        .layer(DefaultBodyLimit::max(limit))
        .layer(from_fn_with_state(ctx.clone(), middleware::cors))
        .layer(from_fn(middleware::request_ids))
        .with_state(ctx)
}

/// The routes alone, without middleware.
pub fn routes() -> Router<Context> {
    let r = Router::new()
        .route("/health", get(health))
        .route("/v1/features", get(features))
        .route("/v1/status", get(status))
        .route("/v1/search", post(search))
        .route("/v1/settings", get(get_settings).put(put_settings))
        .route("/v1/documents", post(add_document).delete(remove_document));
{% if ai %}    #[cfg(feature = "ai")]
    let r = r.route("/v1/ask", post(ask));
{% endif %}{% if background_jobs %}    #[cfg(feature = "jobs-queue")]
    let r = r.route("/v1/jobs", get(jobs_list).post(jobs_enqueue)).route("/v1/jobs/{id}/cancel", post(jobs_cancel)).route("/v1/jobs/{id}/retry", post(jobs_retry));
{% endif %}    // kitchen:routes (`kitchen generate route` adds routes above this line)
    r
}

/// `POST` with an optional JSON body, passed as the params of backend method `name`.
pub fn method(name: &'static str) -> MethodRouter<Context> {
    post(move |State(ctx): State<Context>, body: Result<Bytes, BytesRejection>| async move {
        let body = body?;
        let params = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_json", e.to_string()))?
        };
        ctx.call(name, params).await.map(Json)
    })
}

type Reply<T = Value> = Result<Json<T>, ApiError>;

/// The store when one is open; `503` when it fails.
async fn health(State(ctx): State<Context>) -> Reply {
    match ctx.call("status", Value::Null).await {
        Ok(Value::Null) => Ok(Json(json!({"status": "starting", "store": null}))),
        Ok(store) => Ok(Json(json!({"status": "ok", "store": store}))),
        Err(e) if e.status() == StatusCode::SERVICE_UNAVAILABLE => Ok(Json(json!({"status": "starting", "store": null}))),
        Err(e) => Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "unhealthy", e.to_string())),
    }
}

async fn features(State(ctx): State<Context>) -> Json<Value> {
    Json(ctx.features().clone())
}

async fn status(State(ctx): State<Context>) -> Reply {
    ctx.call("status", Value::Null).await.map(Json)
}

/// `POST /v1/search`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    /// What to look for.
    pub query: String,
    /// Which page of hits.
    #[serde(flatten)]
    pub page: Page,
    /// Search settings for this query only (`search` in settings), e.g. `{"mode": "hybrid"}`.
    #[serde(default)]
    pub search: Option<Value>,
}

impl Validate for SearchRequest {
    fn validate(&self) -> Result<(), FieldErrors> {
        let mut e = FieldErrors::default();
        e.check(!self.query.trim().is_empty(), "query", "must not be empty");
        e.check(self.query.chars().count() <= 2000, "query", "is longer than 2000 characters");
        e.check(self.search.as_ref().is_none_or(Value::is_object), "search", "must be an object");
        self.page.check_into(&mut e);
        e.into_result()
    }
}

async fn search(State(ctx): State<Context>, ValidJson(req): ValidJson<SearchRequest>) -> Reply<Paginated<Value>> {
    let hits = ctx.call("search", json!({"query": req.query, "k": req.page.fetch(), "search": req.search})).await?;
    let hits: Vec<Value> = serde_json::from_value(hits).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(req.page.paginate(hits)))
}

async fn get_settings(State(ctx): State<Context>) -> Reply {
    ctx.call("config", Value::Null).await.map(Json)
}

/// Settings are clamped into range (`Config::validated`), as the Settings screen does.
impl Validate for crate::Config {
    fn validate(&self) -> Result<(), FieldErrors> {
        Ok(())
    }
}

async fn put_settings(State(ctx): State<Context>, ValidJson(config): ValidJson<crate::Config>) -> Reply {
    ctx.call("configure", json!({ "config": config.validated() })).await.map(Json)
}

/// `POST /v1/documents`.
#[derive(Debug, Deserialize)]
pub struct DocumentRequest {
    /// The name the text is stored and found under (a file name or a URL).
    pub source: String,
    /// The text.
    pub text: String,
}

impl Validate for DocumentRequest {
    fn validate(&self) -> Result<(), FieldErrors> {
        let mut e = FieldErrors::default();
        e.check(!self.source.trim().is_empty(), "source", "must not be empty");
        e.check(self.source.len() <= 1000, "source", "is longer than 1000 characters");
        e.check(!self.text.trim().is_empty(), "text", "must not be empty");
        e.into_result()
    }
}

async fn add_document(State(ctx): State<Context>, ValidJson(req): ValidJson<DocumentRequest>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let chunks = ctx.call("ingest_text", json!({"source": req.source, "text": req.text})).await?;
    Ok((StatusCode::CREATED, Json(json!({"source": req.source, "chunks": chunks}))))
}

/// `DELETE /v1/documents?source=`.
#[derive(Debug, Deserialize)]
pub struct SourceQuery {
    /// The source to forget.
    pub source: String,
}

impl Validate for SourceQuery {
    fn validate(&self) -> Result<(), FieldErrors> {
        let mut e = FieldErrors::default();
        e.check(!self.source.trim().is_empty(), "source", "must not be empty");
        e.into_result()
    }
}

async fn remove_document(State(ctx): State<Context>, ValidQuery(q): ValidQuery<SourceQuery>) -> Reply {
    let removed = ctx.call("remove_source", json!({"source": q.source})).await?;
    Ok(Json(json!({ "removed": removed })))
}
{% if ai %}
/// `POST /v1/ask` (feature `ai`).
#[cfg(feature = "ai")]
#[derive(Debug, Deserialize)]
pub struct AskRequest {
    /// The question, answered from the indexed documents.
    pub question: String,
}

#[cfg(feature = "ai")]
impl Validate for AskRequest {
    fn validate(&self) -> Result<(), FieldErrors> {
        let mut e = FieldErrors::default();
        e.check(!self.question.trim().is_empty(), "question", "must not be empty");
        e.check(self.question.chars().count() <= 4000, "question", "is longer than 4000 characters");
        e.into_result()
    }
}

#[cfg(feature = "ai")]
async fn ask(State(ctx): State<Context>, ValidJson(req): ValidJson<AskRequest>) -> Reply {
    ctx.call("ask", json!({"question": req.question})).await.map(Json)
}
{% endif %}{% if background_jobs %}
#[cfg(feature = "jobs-queue")]
async fn jobs_list(State(ctx): State<Context>) -> Reply {
    ctx.call("jobs_list", Value::Null).await.map(Json)
}

/// `POST /v1/jobs` (feature `jobs-queue`).
#[cfg(feature = "jobs-queue")]
#[derive(Debug, Deserialize)]
pub struct EnqueueRequest {
    /// The job type (one of `types` in `GET /v1/jobs`).
    pub kind: String,
    /// The job's input.
    #[serde(default)]
    pub payload: Option<Value>,
    /// `delaySecs`, `runAt`, `maxAttempts`, `timeoutSecs`, passed on as they are.
    #[serde(flatten)]
    pub options: serde_json::Map<String, Value>,
}

#[cfg(feature = "jobs-queue")]
impl Validate for EnqueueRequest {
    fn validate(&self) -> Result<(), FieldErrors> {
        let mut e = FieldErrors::default();
        e.check(!self.kind.trim().is_empty(), "kind", "must not be empty");
        e.check(self.payload.as_ref().is_none_or(|p| p.is_object() || p.is_null()), "payload", "must be an object");
        e.into_result()
    }
}

#[cfg(feature = "jobs-queue")]
async fn jobs_enqueue(State(ctx): State<Context>, ValidJson(req): ValidJson<EnqueueRequest>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut params = Value::Object(req.options);
    params["kind"] = json!(req.kind);
    params["payload"] = req.payload.unwrap_or_else(|| json!({}));
    Ok((StatusCode::CREATED, Json(ctx.call("jobs_enqueue", params).await?)))
}

#[cfg(feature = "jobs-queue")]
async fn jobs_cancel(State(ctx): State<Context>, id: Result<Path<String>, PathRejection>) -> Reply {
    let Path(id) = id?;
    ctx.call("jobs_cancel", json!({ "id": id })).await.map(Json)
}

#[cfg(feature = "jobs-queue")]
async fn jobs_retry(State(ctx): State<Context>, id: Result<Path<String>, PathRejection>) -> Reply {
    let Path(id) = id?;
    ctx.call("jobs_retry", json!({ "id": id })).await.map(Json)
}
{% endif %}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_requests_are_checked() {
        let r: SearchRequest = serde_json::from_value(json!({"query": " ", "perPage": 0})).unwrap();
        let e = r.validate().unwrap_err();
        assert_eq!(e.get("query"), ["must not be empty"]);
        assert!(!e.get("perPage").is_empty());
        let r: SearchRequest = serde_json::from_value(json!({"query": "heart", "page": 2})).unwrap();
        assert!(r.validate().is_ok());
        assert_eq!(r.page, Page { page: 2, per_page: 20 });
    }
}
