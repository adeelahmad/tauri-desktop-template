//! `backup_now`: write a backup of the index, the same as Settings → Backups → Back up now
//! (encrypted when `backup.encrypt`, old ones pruned to `backup.keep`). The payload is `{}`.
//!
//! ```
//! use desktop_core::job_types::backup_now::{BackupNow, Payload};
//! use desktop_core::queue::{Enqueue, JobsConfig, Queue, QueueOptions, Registry};
//! use desktop_core::{Backend, Engine, EngineOptions};
//! use std::sync::Arc;
//!
//! let data = tempfile::tempdir()?;
//! let opts = EngineOptions { data_dir: data.path().into(), backend: Backend::File, embeddings: false, config: Default::default(), app_name: None };
//! let engine = Arc::new(Engine::open(&opts, &mut |_| {})?);
//! engine.ingest_text("note", "Backups are written by a background job.")?;
//! let mut r = Registry::new();
//! r.register(BackupNow);
//! let q = Queue::open_with(data.path(), r, JobsConfig::default(), QueueOptions { engine: Some(Arc::downgrade(&engine)), ..Default::default() })?;
//! q.enqueue_job::<BackupNow>(&Payload::default(), Enqueue::default())?;
//! q.run_pending();
//! assert_eq!(engine.backups().len(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::queue::{Job, JobContext, JobError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The queue backups and other maintenance run in.
pub const QUEUE: &str = "maintenance";

/// No options yet; a struct so fields can be added without breaking stored jobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Payload {}

/// Back up the index now.
pub struct BackupNow;

impl Job for BackupNow {
    type Payload = Payload;
    const NAME: &'static str = "backup_now";

    fn queue(&self) -> &str {
        QUEUE
    }

    fn run(&self, _payload: Payload, ctx: &JobContext) -> Result<Value, JobError> {
        let info = ctx.engine()?.backup()?;
        Ok(serde_json::to_value(info)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_an_open_engine() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = crate::queue::Registry::new();
        r.register(BackupNow);
        let q = crate::queue::Queue::open(dir.path(), r, Default::default()).unwrap();
        q.enqueue("backup_now", serde_json::json!({}), Default::default()).unwrap();
        q.run_pending();
        let j = &q.list().jobs[0];
        assert_eq!(j.last_error.as_deref(), Some("the engine is not open"), "retried later");
    }
}
