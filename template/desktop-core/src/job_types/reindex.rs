//! `reindex`: ingest every file in a folder again (and below it, when `recursive`), with the
//! ingest workers and resource limits of Settings → Ingest and Performance. Files already
//! in the index are replaced, so this also picks up edits.
//!
//! Payload: `{"path": "/folder", "recursive": true}`.
//!
//! ```
//! use desktop_core::job_types::reindex::{Payload, Reindex};
//! use desktop_core::queue::{Enqueue, JobsConfig, Queue, QueueOptions, Registry};
//! use desktop_core::{Backend, Engine, EngineOptions};
//! use std::sync::Arc;
//!
//! let data = tempfile::tempdir()?;
//! let docs = tempfile::tempdir()?;
//! std::fs::write(docs.path().join("a.md"), "The queue re-indexes this folder.")?;
//! let opts = EngineOptions { data_dir: data.path().into(), backend: Backend::File, embeddings: false, config: Default::default(), app_name: None };
//! let engine = Arc::new(Engine::open(&opts, &mut |_| {})?);
//! let mut r = Registry::new();
//! r.register(Reindex);
//! let q = Queue::open_with(data.path(), r, JobsConfig::default(), QueueOptions { engine: Some(Arc::downgrade(&engine)), ..Default::default() })?;
//! q.enqueue_job::<Reindex>(&Payload { path: docs.path().into(), recursive: true }, Enqueue::default())?;
//! q.run_pending();
//! assert_eq!(q.list().history[0].output.as_ref().unwrap()["files"], 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::queue::{Job, JobContext, JobError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// What to re-index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    /// The folder.
    pub path: PathBuf,
    /// Include subfolders (default yes).
    #[serde(default = "yes")]
    pub recursive: bool,
}

fn yes() -> bool {
    true
}

/// Ingest a folder again.
pub struct Reindex;

impl Job for Reindex {
    type Payload = Payload;
    const NAME: &'static str = "reindex";

    fn queue(&self) -> &str {
        super::backup_now::QUEUE
    }

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(6 * 3600)
    }

    fn run(&self, p: Payload, ctx: &JobContext) -> Result<Value, JobError> {
        if !p.path.is_dir() {
            return Err(JobError::fatal(format!("{} is not a folder", p.path.display())));
        }
        let engine = ctx.engine()?;
        let files = engine.files_in(&p.path, p.recursive);
        let report = engine.ingest_paths(&files, &|_, _, _| {});
        if report.files == 0 && !report.errors.is_empty() {
            return Err(JobError::retry(report.errors.join("; ")));
        }
        Ok(serde_json::to_value(report)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_folder_fails_for_good() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = crate::queue::Registry::new();
        r.register(Reindex);
        let q = crate::queue::Queue::open(dir.path(), r, Default::default()).unwrap();
        q.enqueue("reindex", serde_json::json!({"path": dir.path().join("nope")}), Default::default()).unwrap();
        q.run_pending();
        let dead = &q.list().dead[0];
        assert!(dead.last_error.as_deref().unwrap().contains("is not a folder"));
        assert_eq!(dead.attempts, 1);
    }
}
