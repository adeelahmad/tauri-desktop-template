//! The app's background job types (feature `jobs-queue`), registered in one place:
//! [`registry`]. Each type is a file here that implements [`crate::queue::Job`].
//!
//! Add one with `kitchen generate job <name>`: it writes `job_types/<name>.rs` (documented
//! and tested) and registers it below the `kitchen:` markers. Then enqueue it from the UI
//! (`invoke('jobs_enqueue', { kind: '<name>', payload: {...} })`), from Rust
//! (`queue.enqueue_job::<MyJob>(&payload, Enqueue::default())`), or on a schedule.
//!
//! ```
//! let r = desktop_core::job_types::registry();
//! assert!(r.contains("backup_now") && r.contains("reindex"));
//! ```

use crate::queue::{Job, Registry, Schedule};

pub mod backup_now;
pub mod reindex;
// kitchen:job-mods (`kitchen generate job` adds modules above this line)

/// Every job type this app has, with its queue limits and default schedules.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(backup_now::BackupNow);
    r.register(reindex::Reindex);
    // kitchen:jobs (`kitchen generate job` registers job types above this line)
    // Maintenance work runs one job at a time (Settings → Jobs can change it: `jobs.queues`).
    r.limit(backup_now::QUEUE, 1);
    // Added once and paused: nothing runs on a schedule until it is resumed in Settings → Jobs.
    r.schedule(Schedule::cron("nightly-backup", backup_now::BackupNow::NAME, "0 3 * * *").paused(true));
    r
}
