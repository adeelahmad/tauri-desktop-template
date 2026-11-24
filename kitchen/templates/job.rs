//! `{{name}}`: {{Title}} as a background job. Say in one line what it does.
//!
//! Enqueue it from the UI with
//! `invoke('jobs_enqueue', { kind: '{{name}}', payload: { text: 'hello' } })`, from Rust with
//! `queue.enqueue_job::<{{Name}}>(&Payload { .. }, Enqueue::default())`, or on a schedule
//! (Settings → Jobs, or `Registry::schedule` in `job_types/mod.rs`). A failed attempt is
//! retried with backoff until `jobs.maxAttempts`, then it waits in the dead letters.
//!
//! ```
//! use desktop_core::job_types::{{name}}::{ {{Name}}, Payload };
//! use desktop_core::queue::{Enqueue, JobsConfig, Queue, Registry};
//!
//! let data = tempfile::tempdir()?;
//! let mut registry = Registry::new();
//! registry.register({{Name}});
//! let queue = Queue::open(data.path(), registry, JobsConfig::default())?;
//! queue.enqueue_job::<{{Name}}>(&Payload { text: "hello".into() }, Enqueue::default())?;
//! assert_eq!(queue.run_pending(), 1);
//! assert_eq!(queue.list().history[0].output, Some(serde_json::json!({ "echo": "hello" })));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::queue::{Job, JobContext, JobError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// What the job gets; stored as JSON in the queue (add fields with `#[serde(default)]` so
/// jobs already queued keep working).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Payload {
    /// The text to work on (an example field).
    pub text: String,
}

/// The job type.
pub struct {{Name}};

impl Job for {{Name}} {
    type Payload = Payload;
    const NAME: &'static str = "{{name}}";

    // Optional: `queue`, `timeout`, `max_attempts` and `uses_network` (waits while offline).

    fn run(&self, payload: Payload, ctx: &JobContext) -> Result<Value, JobError> {
        if payload.text.is_empty() {
            // No retry fixes a bad payload: straight to the dead letters.
            return Err(JobError::fatal("text is empty"));
        }
        // Long work checks `ctx.is_cancelled()` now and then; `ctx.engine()?` is the open
        // engine (search, ingest, backups). Errors converted with `?` are retried.
        let _ = ctx;
        Ok(json!({ "echo": payload.text }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{Queue, Registry};

    #[test]
    fn an_empty_payload_is_dead_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::new();
        r.register({{Name}});
        let q = Queue::open(dir.path(), r, Default::default()).unwrap();
        q.enqueue("{{name}}", json!({ "text": "" }), Default::default()).unwrap();
        q.run_pending();
        assert_eq!(q.list().dead[0].last_error.as_deref(), Some("text is empty"));
    }
}
