//! Request extractors that check what they read: [`ValidJson`] for bodies and
//! [`ValidQuery`] for query strings. Malformed input is a `400`/`413`/`415` in the API's
//! error shape; well-formed input that breaks a rule ([`Validate`]) is a `422` that names
//! each wrong field.
//!
//! # Example
//!
//! ```
//! use desktop_core::http_api::{FieldErrors, Validate};
//!
//! struct NewNote { title: String }
//!
//! impl Validate for NewNote {
//!     fn validate(&self) -> Result<(), FieldErrors> {
//!         let mut e = FieldErrors::default();
//!         e.check(!self.title.trim().is_empty(), "title", "must not be empty");
//!         e.check(self.title.len() <= 200, "title", "is longer than 200 characters");
//!         e.into_result()
//!     }
//! }
//!
//! assert!(NewNote { title: "Hello".into() }.validate().is_ok());
//! let errors = NewNote { title: " ".into() }.validate().unwrap_err();
//! assert_eq!(errors.get("title"), ["must not be empty"]);
//! ```
//!
//! In a handler, `ValidJson(note): ValidJson<NewNote>` gives a checked `NewNote`.

use super::ApiError;
use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::request::Parts;
use axum::Json;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;

/// Problems by field name (camelCase, as in the JSON), each a short phrase.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FieldErrors(BTreeMap<String, Vec<String>>);

impl FieldErrors {
    /// Record a problem with `field`.
    pub fn add(&mut self, field: &str, problem: impl Into<String>) {
        self.0.entry(field.to_string()).or_default().push(problem.into());
    }

    /// Record `problem` with `field` unless `ok`.
    pub fn check(&mut self, ok: bool, field: &str, problem: &str) -> &mut Self {
        if !ok {
            self.add(field, problem);
        }
        self
    }

    /// The problems with `field` (empty when none).
    pub fn get(&self, field: &str) -> &[String] {
        self.0.get(field).map(Vec::as_slice).unwrap_or_default()
    }

    /// No problems at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `Ok` when there are no problems.
    pub fn into_result(self) -> Result<(), FieldErrors> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

/// Rules a request must follow beyond its JSON types.
pub trait Validate {
    /// Every broken rule, by field.
    fn validate(&self) -> Result<(), FieldErrors>;
}

impl Validate for serde_json::Value {
    fn validate(&self) -> Result<(), FieldErrors> {
        Ok(())
    }
}

/// A JSON body that was read and then checked with [`Validate`].
#[derive(Debug, Clone)]
pub struct ValidJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await?;
        value.validate().map_err(ApiError::validation)?;
        Ok(Self(value))
    }
}

/// A query string that was read and then checked with [`Validate`].
#[derive(Debug, Clone)]
pub struct ValidQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidQuery<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state).await?;
        value.validate().map_err(ApiError::validation)?;
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_problems_by_field() {
        let mut e = FieldErrors::default();
        e.check(true, "a", "never").check(false, "b", "one").check(false, "b", "two");
        assert_eq!(e.get("a"), [] as [String; 0]);
        assert_eq!(e.get("b"), ["one", "two"]);
        assert_eq!(serde_json::to_value(&e).unwrap(), serde_json::json!({"b": ["one", "two"]}));
        assert!(FieldErrors::default().into_result().is_ok());
    }
}
