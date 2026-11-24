//! Background jobs (feature `jobs-queue`): a persistent queue of named job types with typed
//! JSON payloads, delayed jobs, cron and interval schedules, a concurrency limit per queue,
//! automatic retries with exponential backoff and jitter, a dead-letter list, cancel,
//! timeouts and a pruned history of every run.
//!
//! - **Job types** implement [`Job`] (a name, a payload type and `run`) and are registered
//!   in a [`Registry`]; the app's own are in [`crate::job_types`] (`kitchen generate job`).
//! - **Persistence**: `<data>/queue/queue.json` holds the jobs, the dead letters and the
//!   schedules, written through a synced temporary file and a rename after every change, so
//!   the queue survives a crash or a restart. A job that was running when the process
//!   stopped is retried (or goes to the dead letters when it has no attempts left).
//!   `<data>/queue/history.jsonl` logs each run; entries older than `jobs.historyDays` are
//!   pruned. One process owns a queue folder at a time (an OS file lock): another process
//!   of the app can read it but not change or run it.
//! - **Retries**: a failed attempt is retried after `backoffBaseSecs * 2^(attempt-1)` seconds,
//!   capped at `backoffMaxSecs`, of which a random half is jitter ([`backoff`]), until
//!   `maxAttempts`; then the job is dead and can be retried by hand ([`Queue::retry`]). A
//!   job can refuse retries with [`JobError::fatal`].
//! - **Timeouts and cancel** are cooperative: the attempt is given up at once (a timeout
//!   counts as a failed attempt), and the job sees [`JobContext::is_cancelled`].
//! - **Resources**: [`Runtime`] runs at most `jobs.workers` jobs at once, capped by
//!   `resources.cpuPercent`, on threads at background priority when
//!   `resources.backgroundPriority` (see [`crate::governor`]). A job type that
//!   [uses the network](Job::uses_network) waits while the network policy is offline
//!   (`net.offline`), without using up attempts.
//!
//! ## Where it runs
//!
//! In the backend: in the app's process, or in the worker process with `ipc_worker`. With
//! `background_agent`, the worker starts at login and keeps running when the window
//! closes, so schedules keep firing. **On iOS and Android jobs run only while the app is
//! in the foreground**: the system suspends the app in the background (this module uses
//! neither `BGTaskScheduler` nor `WorkManager`). Whatever came due meanwhile runs when the
//! app is back: a schedule that was missed fires once, not once per missed time.
//!
//! ## Why not apalis
//!
//! [apalis](https://crates.io/crates/apalis) was the first choice. Its persistent backends
//! are SQL through sqlx (SQLite, Postgres, MySQL), Redis or AMQP, which this template's
//! rules rule out (no extra services, minimal dependencies). Its file backend,
//! `apalis-file-storage`, keeps tasks in a JSON-lines file but has no delayed jobs, no
//! dead-letter list and no cancel, and apalis needs an async runtime (tokio, tower,
//! futures, tracing) in a crate that is plain std threads; apalis-cron adds chrono-tz.
//! So the hard part with real edge cases, cron parsing and next-time computation, comes
//! from [croner](https://crates.io/crates/croner) (with chrono), and the queue itself is
//! this small module on std threads and the crate's atomic-write helper.
//!
//! ```
//! use desktop_core::queue::{Enqueue, Job, JobContext, JobError, JobsConfig, Queue, Registry};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct Greet {
//!     name: String,
//! }
//!
//! struct Hello;
//!
//! impl Job for Hello {
//!     type Payload = Greet;
//!     const NAME: &'static str = "hello";
//!     fn run(&self, p: Greet, _ctx: &JobContext) -> Result<serde_json::Value, JobError> {
//!         Ok(serde_json::json!(format!("Hello, {}!", p.name)))
//!     }
//! }
//!
//! let data = tempfile::tempdir()?;
//! let mut registry = Registry::new();
//! registry.register(Hello);
//! let queue = Queue::open(data.path(), registry, JobsConfig::default())?;
//! queue.enqueue_job::<Hello>(&Greet { name: "jobs".into() }, Enqueue::default())?;
//! // A `Runtime` does this on its own threads; here it runs on this one.
//! assert_eq!(queue.run_pending(), 1);
//! assert_eq!(queue.list().history[0].output, Some(serde_json::json!("Hello, jobs!")));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::governor::ResourceConfig;
use crate::{Error, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Duration;

/// The queue a job type runs in unless it names another.
pub const DEFAULT_QUEUE: &str = "default";
const HOUR_MS: u64 = 3_600_000;
const DAY_MS: u64 = 86_400_000;
/// The dispatcher looks at the clock at least this often (a suspended phone's timers stop).
const MAX_WAIT_MS: u64 = 30_000;
/// How long a job that needs the network waits while the app is offline.
const OFFLINE_WAIT_MS: u64 = 60_000;
/// History entries `list` returns, newest first.
const HISTORY_SHOWN: usize = 200;
/// History entries kept on disk at most, whatever their age.
const HISTORY_KEPT: usize = 10_000;

/// `jobs` in the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JobsConfig {
    /// Run background jobs and schedules (the queue is kept either way).
    pub enabled: bool,
    /// Jobs run at once, over all queues (capped by `resources.cpuPercent`).
    pub workers: usize,
    /// Attempts before a job goes to the dead letters (a job type can set its own).
    pub max_attempts: u32,
    /// The first retry waits about this long; each next one twice as long.
    pub backoff_base_secs: u64,
    /// The longest wait between attempts.
    pub backoff_max_secs: u64,
    /// Days the run history is kept.
    pub history_days: u32,
    /// Queue name → jobs run at once in that queue (overrides the job types' defaults).
    pub queues: BTreeMap<String, usize>,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self { enabled: false, workers: 2, max_attempts: 5, backoff_base_secs: 10, backoff_max_secs: 3600, history_days: 7, queues: BTreeMap::new() }
    }
}

impl JobsConfig {
    /// The same config with every number in a safe range (called from `Config::validated`).
    pub fn validated(mut self) -> Self {
        self.workers = self.workers.clamp(1, 32);
        self.max_attempts = self.max_attempts.clamp(1, 100);
        self.backoff_base_secs = self.backoff_base_secs.clamp(1, 86_400);
        self.backoff_max_secs = self.backoff_max_secs.clamp(self.backoff_base_secs, 7 * 86_400);
        self.history_days = self.history_days.clamp(1, 365);
        self.queues = self.queues.into_iter().map(|(q, n)| (q.trim().to_string(), n.clamp(1, 32))).filter(|(q, _)| !q.is_empty()).collect();
        self
    }
}

/// The wait before retry number `attempt` (1 after the first failure): `base * 2^(attempt-1)`
/// capped at `max`, then "equal jitter": half of it fixed and half scaled by `unit` (a random
/// number in 0..1), so retries of many jobs that failed together spread out.
///
/// ```
/// use desktop_core::queue::backoff;
/// use std::time::Duration;
/// let s = Duration::from_secs;
/// assert_eq!(backoff(1, s(10), s(3600), 0.0), s(5));
/// assert_eq!(backoff(3, s(10), s(3600), 1.0), s(40));
/// assert_eq!(backoff(30, s(10), s(3600), 1.0), s(3600));
/// ```
pub fn backoff(attempt: u32, base: Duration, max: Duration, unit: f64) -> Duration {
    let factor = 1u32 << attempt.saturating_sub(1).min(31);
    let exp = base.saturating_mul(factor).min(max);
    let unit = if unit.is_finite() { unit.clamp(0.0, 1.0) } else { 0.5 };
    exp / 2 + exp.mul_f64(unit) / 2
}

/// A random number in 0..1 from the system's secure generator (ring, already in the tree).
fn random_unit() -> f64 {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 8];
    if ring::rand::SystemRandom::new().fill(&mut b).is_err() {
        return 0.5;
    }
    (u64::from_le_bytes(b) >> 11) as f64 / (1u64 << 53) as f64
}

/// Milliseconds since the Unix epoch; tests use [`FakeClock`].
pub trait Clock: Send + Sync {
    /// Now, in milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

/// The computer's clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
    }
}

/// A clock that moves only when told to, for tests of delays, backoff and schedules.
#[derive(Debug, Default)]
pub struct FakeClock(AtomicU64);

impl FakeClock {
    /// A clock stopped at `ms`.
    pub fn new(ms: u64) -> Self {
        Self(AtomicU64::new(ms))
    }
    /// Move the clock to `ms`.
    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
    /// Move the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Why an attempt failed, and whether trying again could help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    /// What went wrong, shown in the queue and the history.
    pub message: String,
    /// Retry (with backoff) while attempts are left; `false` sends the job straight to the
    /// dead letters.
    pub retry: bool,
}

impl JobError {
    /// A failure worth retrying (a network error, a busy resource).
    pub fn retry(message: impl Into<String>) -> Self {
        Self { message: message.into(), retry: true }
    }
    /// A failure no retry fixes (a bad payload, a missing file).
    pub fn fatal(message: impl Into<String>) -> Self {
        Self { message: message.into(), retry: false }
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for JobError {
    fn from(message: String) -> Self {
        Self::retry(message)
    }
}

impl From<&str> for JobError {
    fn from(message: &str) -> Self {
        Self::retry(message)
    }
}

impl From<Error> for JobError {
    fn from(e: Error) -> Self {
        Self::retry(e.to_string())
    }
}

impl From<serde_json::Error> for JobError {
    fn from(e: serde_json::Error) -> Self {
        Self::fatal(e.to_string())
    }
}

/// What a running job knows about itself.
pub struct JobContext {
    /// The job's id.
    pub id: String,
    /// The job type.
    pub kind: String,
    /// 1 for the first attempt.
    pub attempt: u32,
    cancel: Arc<AtomicBool>,
    engine: Option<Weak<crate::Engine>>,
}

impl JobContext {
    /// Cancelled or timed out: stop at the next safe point (the result is ignored).
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// The open engine (search, ingest, backups, config); a retryable error when it is closed.
    pub fn engine(&self) -> std::result::Result<Arc<crate::Engine>, JobError> {
        self.engine.as_ref().and_then(Weak::upgrade).ok_or_else(|| JobError::retry("the engine is not open"))
    }
}

/// A kind of background work. The payload is typed: it is checked when a job is enqueued
/// and handed to `run` already parsed.
pub trait Job: Send + Sync + 'static {
    /// The payload, stored as JSON in the queue.
    type Payload: Serialize + DeserializeOwned;
    /// The job type's name, unique in the registry (snake_case).
    const NAME: &'static str;
    /// The queue it runs in; each queue has its own concurrency limit.
    fn queue(&self) -> &str {
        DEFAULT_QUEUE
    }
    /// How long one attempt may take.
    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
    /// Attempts before the dead letters (`None`: `jobs.maxAttempts`).
    fn max_attempts(&self) -> Option<u32> {
        None
    }
    /// It needs the network, so it waits while the app is offline (`net.offline`).
    fn uses_network(&self) -> bool {
        false
    }
    /// Do the work. The returned value is kept in the run history.
    fn run(&self, payload: Self::Payload, ctx: &JobContext) -> std::result::Result<Value, JobError>;
}

/// A job type with its payload type erased, so the registry can hold every kind.
trait Handler: Send + Sync {
    fn queue(&self) -> String;
    fn timeout(&self) -> Duration;
    fn max_attempts(&self) -> Option<u32>;
    fn uses_network(&self) -> bool;
    fn check(&self, payload: &Value) -> std::result::Result<(), String>;
    fn run(&self, payload: Value, ctx: &JobContext) -> std::result::Result<Value, JobError>;
}

struct Typed<J>(J);

impl<J: Job> Handler for Typed<J> {
    fn queue(&self) -> String {
        self.0.queue().to_string()
    }
    fn timeout(&self) -> Duration {
        self.0.timeout()
    }
    fn max_attempts(&self) -> Option<u32> {
        self.0.max_attempts()
    }
    fn uses_network(&self) -> bool {
        self.0.uses_network()
    }
    fn check(&self, payload: &Value) -> std::result::Result<(), String> {
        J::Payload::deserialize(payload).map(|_| ()).map_err(|e| e.to_string())
    }
    fn run(&self, payload: Value, ctx: &JobContext) -> std::result::Result<Value, JobError> {
        let p = J::Payload::deserialize(payload).map_err(|e| JobError::fatal(format!("payload: {e}")))?;
        self.0.run(p, ctx)
    }
}

/// The job types a queue can run, queue limits set in code, and default schedules.
#[derive(Clone, Default)]
pub struct Registry {
    handlers: BTreeMap<String, Arc<dyn Handler>>,
    limits: BTreeMap<String, usize>,
    schedules: Vec<Schedule>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a job type (a later one with the same name replaces it).
    pub fn register<J: Job>(&mut self, job: J) -> &mut Self {
        self.handlers.insert(J::NAME.to_string(), Arc::new(Typed(job)));
        self
    }

    /// Run at most `n` jobs of `queue` at once (`jobs.queues` in the settings wins).
    pub fn limit(&mut self, queue: &str, n: usize) -> &mut Self {
        self.limits.insert(queue.to_string(), n.max(1));
        self
    }

    /// A schedule the queue gets once, the first time it opens with this registry. The user
    /// can then pause, change or remove it; a removed one does not come back.
    pub fn schedule(&mut self, schedule: Schedule) -> &mut Self {
        self.schedules.push(schedule);
        self
    }

    /// The registered job type names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// Whether a job type of this name is registered.
    pub fn contains(&self, kind: &str) -> bool {
        self.handlers.contains_key(kind)
    }
}

/// Where a job is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Waits for its time (`runAt`) and a free worker.
    #[default]
    Queued,
    /// An attempt is running now.
    Running,
    /// Failed; waits for its next attempt.
    Retrying,
    /// Needs the network and the app is offline.
    Waiting,
    /// Out of attempts (or failed for good): in the dead letters until retried or cancelled.
    Dead,
}

/// A job in the queue (or in the dead letters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    /// Unique in the queue.
    pub id: String,
    /// The job type.
    pub kind: String,
    /// The queue it runs in (each has its own concurrency limit).
    pub queue: String,
    /// The job type's payload, as JSON.
    pub payload: Value,
    /// Where it is.
    pub state: State,
    /// Attempts started so far.
    pub attempts: u32,
    /// Attempts before it goes to the dead letters.
    pub max_attempts: u32,
    /// Not before this time (ms since the Unix epoch).
    pub run_at: u64,
    /// When it was enqueued (ms since the Unix epoch).
    pub created: u64,
    /// How long one attempt may take.
    pub timeout_secs: u64,
    /// Why the last attempt failed (or why it waits).
    #[serde(default)]
    pub last_error: Option<String>,
    /// The schedule that enqueued it.
    #[serde(default)]
    pub schedule: Option<String>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Done.
    Ok,
    /// `run` returned an error.
    Failed,
    /// It took longer than its timeout.
    TimedOut,
    /// Cancelled while it ran (or before).
    Cancelled,
    /// The process stopped while it ran.
    Interrupted,
}

/// One attempt, in the run history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// The job's id.
    pub job: String,
    /// The job type.
    pub kind: String,
    /// 1 for the first attempt.
    pub attempt: u32,
    /// When it started (ms since the Unix epoch).
    pub started: u64,
    /// When it ended.
    pub finished: u64,
    /// How it ended.
    pub outcome: Outcome,
    /// The error, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// What `run` returned, when it succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// When the next attempt is due, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
}

/// A job enqueued again and again: by a cron expression (five fields, minute first, in local
/// time; `@daily` and the like work too) or every `every_secs` seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Schedule {
    /// Unique among the schedules.
    pub id: String,
    /// The job type it enqueues.
    pub kind: String,
    /// The payload of each job.
    pub payload: Value,
    /// A cron expression, or
    pub cron: Option<String>,
    /// an interval in seconds.
    pub every_secs: Option<u64>,
    /// Paused schedules do not fire.
    pub paused: bool,
    /// When it fires next (ms since the Unix epoch; `None` while paused).
    pub next_run: Option<u64>,
    /// When it last fired.
    pub last_run: Option<u64>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self { id: String::new(), kind: String::new(), payload: Value::Object(Default::default()), cron: None, every_secs: None, paused: false, next_run: None, last_run: None }
    }
}

impl Schedule {
    /// `kind` with an empty payload at the times `expr` names.
    pub fn cron(id: &str, kind: &str, expr: &str) -> Self {
        Self { id: id.into(), kind: kind.into(), cron: Some(expr.into()), ..Self::default() }
    }

    /// `kind` with an empty payload every `secs` seconds.
    pub fn every(id: &str, kind: &str, secs: u64) -> Self {
        Self { id: id.into(), kind: kind.into(), every_secs: Some(secs), ..Self::default() }
    }

    /// The same schedule with this payload.
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    /// The same schedule, paused or not.
    pub fn paused(mut self, paused: bool) -> Self {
        self.paused = paused;
        self
    }

    /// The first time after `now_ms` it fires.
    pub fn next_after(&self, now_ms: u64) -> Result<u64> {
        match (&self.cron, self.every_secs) {
            (Some(expr), _) => next_cron(expr, now_ms),
            (None, Some(secs)) => Ok(now_ms.saturating_add(secs.max(1).saturating_mul(1000))),
            (None, None) => Err(Error::Source(format!("schedule {}: give a cron expression or an interval", self.id))),
        }
    }
}

/// The first time after `after_ms` that the cron expression `expr` names, in local time.
///
/// ```
/// // Every five minutes, from 00:01:00 UTC on 1 Jan 2026: 00:05 (whole hours keep whole
/// // minutes in every time zone that is whole quarter hours off UTC).
/// let t = 1_767_225_660_000;
/// assert_eq!(desktop_core::queue::next_cron("*/5 * * * *", t).unwrap(), t + 4 * 60_000);
/// assert!(desktop_core::queue::next_cron("not cron", t).is_err());
/// ```
pub fn next_cron(expr: &str, after_ms: u64) -> Result<u64> {
    use chrono::TimeZone;
    let cron: croner::Cron = expr.trim().parse().map_err(|e| Error::Source(format!("cron {expr:?}: {e}")))?;
    let after = chrono::Local.timestamp_millis_opt(after_ms as i64).single().ok_or_else(|| Error::Source("time out of range".into()))?;
    let next = cron.find_next_occurrence(&after, false).map_err(|e| Error::Source(format!("cron {expr:?}: {e}")))?;
    Ok(next.timestamp_millis().max(0) as u64)
}

/// When and how a job runs (each field optional).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Enqueue {
    /// Wait this long first.
    pub delay_secs: Option<u64>,
    /// Or not before this time (ms since the Unix epoch).
    pub run_at: Option<u64>,
    /// Attempts before the dead letters (default: the job type's, else `jobs.maxAttempts`).
    pub max_attempts: Option<u32>,
    /// How long one attempt may take (default: the job type's).
    pub timeout_secs: Option<u64>,
}

impl Enqueue {
    /// Run after `delay`.
    pub fn after(delay: Duration) -> Self {
        Self { delay_secs: Some(delay.as_secs()), ..Self::default() }
    }
}

/// A change to one schedule (`jobs_set_schedule`). Fields left out stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScheduleUpdate {
    /// The schedule to change (or create).
    pub id: String,
    /// Its job type (needed for a new one).
    pub kind: Option<String>,
    /// Its payload.
    pub payload: Option<Value>,
    /// Switch to this cron expression.
    pub cron: Option<String>,
    /// Or to this interval.
    pub every_secs: Option<u64>,
    /// Pause (`true`) or resume (`false`).
    pub paused: Option<bool>,
    /// Delete the schedule.
    pub remove: bool,
}

/// What `list` shows: waiting and running jobs, the dead letters and recent runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// `jobs.enabled`.
    pub enabled: bool,
    /// This process runs the queue (another process of the app may own it).
    pub owner: bool,
    /// The registered job types.
    pub types: Vec<String>,
    /// Waiting and running jobs.
    pub jobs: Vec<JobRecord>,
    /// The dead letters.
    pub dead: Vec<JobRecord>,
    /// Newest first.
    pub history: Vec<Run>,
    /// The queue's clock now (ms since the Unix epoch).
    pub now: u64,
}

/// What `Queue` needs from the outside world; tests swap in a fake clock and fixed jitter.
pub struct QueueOptions {
    /// The time.
    pub clock: Arc<dyn Clock>,
    /// A number in 0..1 for backoff jitter.
    pub jitter: fn() -> f64,
    /// Whether jobs that use the network may run now (default: the network policy is not offline).
    pub online: fn() -> bool,
    /// The engine handed to jobs ([`JobContext::engine`]); it can also be set later.
    pub engine: Option<Weak<crate::Engine>>,
}

impl Default for QueueOptions {
    fn default() -> Self {
        Self { clock: Arc::new(SystemClock), jitter: random_unit, online: || !crate::net::policy().offline, engine: None }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct QueueFile {
    jobs: Vec<JobRecord>,
    schedules: Vec<Schedule>,
    /// Registry schedules already added once (a removed one stays removed).
    seeded: Vec<String>,
}

struct Shared {
    file: QueueFile,
    config: JobsConfig,
    resources: ResourceConfig,
    engine: Option<Weak<crate::Engine>>,
    /// Running job id → its queue and cancel flag.
    running: HashMap<String, (String, Arc<AtomicBool>)>,
    seq: u64,
    last_prune: u64,
    /// Set by [`Queue::shutdown`]: start nothing, hand running jobs back.
    returning: bool,
}

struct Inner {
    dir: PathBuf,
    registry: Registry,
    clock: Arc<dyn Clock>,
    jitter: fn() -> f64,
    online: fn() -> bool,
    /// The lock that makes this process the queue's owner (held while the queue is open).
    owner: Option<std::fs::File>,
    shared: Mutex<Shared>,
    wake: Condvar,
}

/// A claimed attempt, ready to run.
struct Claim {
    record: JobRecord,
    handler: Arc<dyn Handler>,
    cancel: Arc<AtomicBool>,
    engine: Option<Weak<crate::Engine>>,
    started: u64,
    background: bool,
}

enum Failure {
    Error(JobError),
    TimedOut,
}

const RETURNED: &str = "returned to the queue: the app stopped while it ran";

/// Put a running job back to wait, as if this attempt never started.
fn give_back(j: &mut JobRecord, now: u64) {
    j.state = State::Queued;
    j.attempts = j.attempts.saturating_sub(1);
    j.run_at = now;
    j.last_error = Some(RETURNED.into());
}

/// The persistent queue. Cheap to clone (all clones are the same queue).
#[derive(Clone)]
pub struct Queue {
    inner: Arc<Inner>,
}

impl Queue {
    /// Open (or create) the queue in `<data_dir>/queue` with the system clock.
    pub fn open(data_dir: &Path, registry: Registry, config: JobsConfig) -> Result<Self> {
        Self::open_with(data_dir, registry, config, QueueOptions::default())
    }

    /// `open` with another clock, jitter, online check or engine.
    pub fn open_with(data_dir: &Path, registry: Registry, config: JobsConfig, opts: QueueOptions) -> Result<Self> {
        let dir = data_dir.join("queue");
        std::fs::create_dir_all(&dir)?;
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("owner.lock"))?;
        let owner = match lock.try_lock() {
            Ok(()) => Some(lock),
            Err(std::fs::TryLockError::WouldBlock) => None,
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        };
        let file = read_file(&dir)?;
        let shared = Shared { file, config: config.validated(), resources: ResourceConfig::default(), engine: opts.engine, running: HashMap::new(), seq: 0, last_prune: 0, returning: false };
        let q = Self { inner: Arc::new(Inner { dir, registry, clock: opts.clock, jitter: opts.jitter, online: opts.online, owner, shared: Mutex::new(shared), wake: Condvar::new() }) };
        if q.is_owner() {
            q.recover()?;
        } else {
            log::info!("background jobs: {} is open in another process of this app", q.inner.dir.display());
        }
        Ok(q)
    }

    /// The queue's folder (`<data>/queue`).
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// This process owns the queue: it may change and run it.
    pub fn is_owner(&self) -> bool {
        self.inner.owner.is_some()
    }

    /// Apply new settings (`jobs` and `resources`); a running `Runtime` picks them up at once.
    pub fn configure(&self, config: JobsConfig, resources: ResourceConfig) {
        let mut s = self.lock();
        s.config = config.validated();
        s.resources = resources.validated();
        self.inner.wake.notify_all();
    }

    /// The engine jobs get from [`JobContext::engine`].
    pub fn set_engine(&self, engine: Weak<crate::Engine>) {
        self.lock().engine = Some(engine);
    }

    /// The registered job type names.
    pub fn types(&self) -> Vec<String> {
        self.inner.registry.names()
    }

    /// Add a job of type `kind`. The payload is checked against the type's payload now, so a
    /// bad one is refused here instead of failing later.
    pub fn enqueue(&self, kind: &str, payload: Value, opts: Enqueue) -> Result<JobRecord> {
        let h = self.handler(kind)?;
        h.check(&payload).map_err(|e| Error::Source(format!("{kind}: bad payload: {e}")))?;
        let mut s = self.owned()?;
        let now = self.now();
        let rec = self.record(&mut s, kind, &h, payload, &opts, now);
        s.file.jobs.push(rec.clone());
        self.save(&s)?;
        self.inner.wake.notify_all();
        Ok(rec)
    }

    /// `enqueue` with a typed payload.
    pub fn enqueue_job<J: Job>(&self, payload: &J::Payload, opts: Enqueue) -> Result<JobRecord> {
        self.enqueue(J::NAME, serde_json::to_value(payload)?, opts)
    }

    /// Cancel a job: a waiting or dead one is removed, a running one is told to stop (its
    /// result is ignored). `false` when there is no such job.
    pub fn cancel(&self, id: &str) -> Result<bool> {
        let mut s = self.owned()?;
        if let Some((_, flag)) = s.running.get(id) {
            flag.store(true, Ordering::SeqCst);
            return Ok(true);
        }
        let Some(pos) = s.file.jobs.iter().position(|j| j.id == id) else { return Ok(false) };
        let j = s.file.jobs.remove(pos);
        let now = self.now();
        self.save(&s)?;
        self.log_run(&Run { job: j.id, kind: j.kind, attempt: j.attempts, started: now, finished: now, outcome: Outcome::Cancelled, message: None, output: None, retry_at: None });
        Ok(true)
    }

    /// Run a job again now: a dead one starts over with all its attempts, a retrying or
    /// waiting one skips its wait.
    pub fn retry(&self, id: &str) -> Result<JobRecord> {
        let mut s = self.owned()?;
        let now = self.now();
        let j = s.file.jobs.iter_mut().find(|j| j.id == id).ok_or_else(|| Error::Source(format!("no job {id}")))?;
        if j.state == State::Running {
            return Err(Error::Source(format!("job {id} is running")));
        }
        if j.state == State::Dead {
            j.attempts = 0;
        }
        j.state = State::Queued;
        j.run_at = now;
        let rec = j.clone();
        self.save(&s)?;
        self.inner.wake.notify_all();
        Ok(rec)
    }

    /// The jobs, the dead letters and the recent runs.
    pub fn list(&self) -> Snapshot {
        let (enabled, file) = self.view();
        let (dead, jobs) = file.jobs.into_iter().partition(|j| j.state == State::Dead);
        Snapshot { enabled, owner: self.is_owner(), types: self.types(), jobs, dead, history: self.history(HISTORY_SHOWN), now: self.now() }
    }

    /// The schedules.
    pub fn schedules(&self) -> Vec<Schedule> {
        self.view().1.schedules
    }

    /// Add, change, pause, resume or remove a schedule; returns them all.
    pub fn set_schedule(&self, u: ScheduleUpdate) -> Result<Vec<Schedule>> {
        let id = u.id.trim().to_string();
        if id.is_empty() {
            return Err(Error::Source("a schedule needs an id".into()));
        }
        let mut s = self.owned()?;
        let now = self.now();
        if u.remove {
            s.file.schedules.retain(|x| x.id != id);
        } else {
            let mut next = s.file.schedules.iter().find(|x| x.id == id).cloned().unwrap_or_else(|| Schedule { id: id.clone(), ..Schedule::default() });
            let timing = u.cron.is_some() || u.every_secs.is_some() || u.paused == Some(false);
            if let Some(k) = u.kind {
                next.kind = k;
            }
            if let Some(p) = u.payload {
                next.payload = p;
            }
            match (u.cron, u.every_secs) {
                (Some(c), _) => (next.cron, next.every_secs) = (Some(c.trim().to_string()), None),
                (None, Some(e)) => (next.cron, next.every_secs) = (None, Some(e.max(1))),
                (None, None) => {}
            }
            if let Some(p) = u.paused {
                next.paused = p;
            }
            let h = self.handler(&next.kind)?;
            h.check(&next.payload).map_err(|e| Error::Source(format!("{}: bad payload: {e}", next.kind)))?;
            let at = next.next_after(now)?;
            if next.paused {
                next.next_run = None;
            } else if timing || next.next_run.is_none() {
                next.next_run = Some(at);
            }
            match s.file.schedules.iter_mut().find(|x| x.id == id) {
                Some(x) => *x = next,
                None => s.file.schedules.push(next),
            }
        }
        self.save(&s)?;
        self.inner.wake.notify_all();
        Ok(s.file.schedules.clone())
    }

    /// Fire due schedules and run every due job on this thread, one after another, until
    /// none is left; returns the attempts run. The [`Runtime`] does the same on its own
    /// threads; this is for tests, examples and one-off catch-up.
    pub fn run_pending(&self) -> usize {
        let mut n = 0;
        loop {
            let claims = {
                let mut s = self.lock();
                self.tick(&mut s)
            };
            if claims.is_empty() {
                return n;
            }
            for c in claims {
                let r = execute(&c);
                self.finish(c, r);
                n += 1;
            }
        }
    }

    /// Stop cleanly (the daemon calls this when it stops): no new attempt starts, running
    /// jobs are told to stop ([`JobContext::is_cancelled`]) and each goes back to the queue
    /// as it was, the attempt not counted. Waits up to `grace` for them; jobs still running
    /// then are handed back too (whatever they finish with later is still recorded).
    /// Stop the [`Runtime`] first. Returns how many jobs went back.
    ///
    /// ```
    /// use desktop_core::queue::{JobsConfig, Queue, Registry};
    /// use std::time::Duration;
    ///
    /// let dir = std::env::temp_dir().join(format!("queue-shutdown-doc-{}", std::process::id()));
    /// let q = Queue::open(&dir, Registry::new(), JobsConfig::default())?;
    /// assert_eq!(q.shutdown(Duration::from_millis(10)), 0); // nothing was running
    /// # drop(q); let _ = std::fs::remove_dir_all(&dir);
    /// # Ok::<(), desktop_core::Error>(())
    /// ```
    pub fn shutdown(&self, grace: Duration) -> usize {
        let deadline = std::time::Instant::now() + grace;
        let mut s = self.lock();
        s.returning = true;
        let count = s.running.len();
        for (_, cancel) in s.running.values() {
            cancel.store(true, Ordering::SeqCst);
        }
        while !s.running.is_empty() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            s = self.inner.wake.wait_timeout(s, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
        let now = self.now();
        let stragglers: Vec<String> = s.running.keys().cloned().collect();
        for j in s.file.jobs.iter_mut().filter(|j| stragglers.contains(&j.id) && j.state == State::Running) {
            give_back(j, now);
        }
        if let Err(e) = self.save(&s) {
            log::warn!("background jobs: could not save the queue: {e}");
        }
        if !stragglers.is_empty() {
            log::warn!("background jobs: {} job(s) did not stop in time; they run again at the next start", stragglers.len());
        }
        count
    }

    // ---- internals

    fn now(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.inner.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The lock, for a change: only the owning process changes the queue.
    fn owned(&self) -> Result<MutexGuard<'_, Shared>> {
        if !self.is_owner() {
            return Err(Error::Source("background jobs are run by another process of this app; try again there".into()));
        }
        Ok(self.lock())
    }

    /// `enabled` and the queue: from memory when this process owns it, else from disk.
    fn view(&self) -> (bool, QueueFile) {
        let s = self.lock();
        if self.is_owner() {
            return (s.config.enabled, QueueFile { jobs: s.file.jobs.clone(), schedules: s.file.schedules.clone(), seeded: Vec::new() });
        }
        (s.config.enabled, read_file(&self.inner.dir).unwrap_or_default())
    }

    fn handler(&self, kind: &str) -> Result<Arc<dyn Handler>> {
        self.inner.registry.handlers.get(kind).cloned().ok_or_else(|| Error::Source(format!("no job type {kind:?} (known: {})", self.types().join(", "))))
    }

    fn record(&self, s: &mut Shared, kind: &str, h: &Arc<dyn Handler>, payload: Value, opts: &Enqueue, now: u64) -> JobRecord {
        s.seq += 1;
        let run_at = opts.run_at.unwrap_or_else(|| now.saturating_add(opts.delay_secs.unwrap_or(0).saturating_mul(1000)));
        JobRecord {
            id: format!("{now:x}-{:x}-{:x}", std::process::id(), s.seq),
            kind: kind.to_string(),
            queue: h.queue(),
            payload,
            state: State::Queued,
            attempts: 0,
            max_attempts: opts.max_attempts.or_else(|| h.max_attempts()).unwrap_or(s.config.max_attempts).clamp(1, 100),
            run_at,
            created: now,
            timeout_secs: opts.timeout_secs.unwrap_or_else(|| h.timeout().as_secs()).clamp(1, 7 * 86_400),
            last_error: None,
            schedule: None,
        }
    }

    fn save(&self, s: &Shared) -> Result<()> {
        crate::jobs::write_atomic(&self.inner.dir.join("queue.json"), &serde_json::to_vec_pretty(&s.file)?)?;
        Ok(())
    }

    /// Start-up of the owner: jobs that were running when the process stopped are retried
    /// (or dead when out of attempts), registry schedules are added once, the history pruned.
    fn recover(&self) -> Result<()> {
        let now = self.now();
        let mut s = self.lock();
        let mut runs = Vec::new();
        for j in s.file.jobs.iter_mut().filter(|j| j.state == State::Running) {
            let message = "interrupted: the app stopped while it ran".to_string();
            j.state = if j.attempts >= j.max_attempts { State::Dead } else { State::Retrying };
            j.run_at = now;
            j.last_error = Some(message.clone());
            runs.push(Run { job: j.id.clone(), kind: j.kind.clone(), attempt: j.attempts, started: now, finished: now, outcome: Outcome::Interrupted, message: Some(message), output: None, retry_at: None });
        }
        for sc in &self.inner.registry.schedules {
            if s.file.seeded.contains(&sc.id) {
                continue;
            }
            s.file.seeded.push(sc.id.clone());
            if !s.file.schedules.iter().any(|x| x.id == sc.id) {
                let mut sc = sc.clone();
                sc.next_run = if sc.paused { None } else { sc.next_after(now).ok() };
                s.file.schedules.push(sc);
            }
        }
        self.save(&s)?;
        drop(s);
        runs.iter().for_each(|r| self.log_run(r));
        let days = self.lock().config.history_days;
        self.prune_history(now, days);
        Ok(())
    }

    /// Fire due schedules, prune the history now and then, and claim the due jobs that
    /// fit the worker and queue limits.
    fn tick(&self, s: &mut Shared) -> Vec<Claim> {
        if !self.is_owner() {
            return Vec::new();
        }
        let now = self.now();
        let mut changed = self.fire_schedules(s, now);
        if now.saturating_sub(s.last_prune) >= HOUR_MS {
            s.last_prune = now;
            self.prune_history(now, s.config.history_days);
        }
        let claims = self.claim(s, now, &mut changed);
        if changed {
            if let Err(e) = self.save(s) {
                log::warn!("background jobs: could not save the queue: {e}");
            }
        }
        claims
    }

    fn fire_schedules(&self, s: &mut Shared, now: u64) -> bool {
        let mut changed = false;
        for i in 0..s.file.schedules.len() {
            let sc = s.file.schedules[i].clone();
            if sc.paused {
                continue;
            }
            match sc.next_run {
                None => {
                    s.file.schedules[i].next_run = sc.next_after(now).ok();
                    changed = true;
                }
                Some(t) if t <= now => {
                    // One job per schedule at a time: a slow job is not piled up behind.
                    let busy = s.file.jobs.iter().any(|j| j.schedule.as_deref() == Some(sc.id.as_str()) && j.state != State::Dead);
                    match self.inner.registry.handlers.get(&sc.kind).cloned() {
                        Some(h) if !busy => {
                            let mut rec = self.record(s, &sc.kind, &h, sc.payload.clone(), &Enqueue::default(), now);
                            rec.schedule = Some(sc.id.clone());
                            s.file.jobs.push(rec);
                        }
                        Some(_) => log::info!("background jobs: schedule {} skipped, its last job has not finished", sc.id),
                        None => log::warn!("background jobs: schedule {} names no job type in this build ({})", sc.id, sc.kind),
                    }
                    let x = &mut s.file.schedules[i];
                    x.last_run = Some(now);
                    x.next_run = sc.next_after(now).ok();
                    changed = true;
                }
                Some(_) => {}
            }
        }
        changed
    }

    fn claim(&self, s: &mut Shared, now: u64, changed: &mut bool) -> Vec<Claim> {
        if s.returning {
            return Vec::new();
        }
        let Shared { file, config, resources, engine, running, .. } = s;
        let cap = resources.workers(config.workers);
        let mut free = cap.saturating_sub(running.len());
        let mut per_queue: HashMap<String, usize> = HashMap::new();
        for (q, _) in running.values() {
            *per_queue.entry(q.clone()).or_default() += 1;
        }
        let mut due: Vec<usize> = (0..file.jobs.len()).filter(|&i| matches!(file.jobs[i].state, State::Queued | State::Retrying | State::Waiting) && file.jobs[i].run_at <= now).collect();
        due.sort_by_key(|&i| (file.jobs[i].run_at, file.jobs[i].created));
        let mut online = None;
        let mut claims = Vec::new();
        for i in due {
            if free == 0 {
                break;
            }
            let j = &mut file.jobs[i];
            let Some(h) = self.inner.registry.handlers.get(&j.kind).cloned() else {
                j.state = State::Dead;
                j.last_error = Some(format!("no job type {:?} in this build", j.kind));
                *changed = true;
                continue;
            };
            let limit = config.queues.get(&j.queue).or_else(|| self.inner.registry.limits.get(&j.queue)).copied().unwrap_or(cap).max(1);
            let in_queue = per_queue.entry(j.queue.clone()).or_default();
            if *in_queue >= limit {
                continue;
            }
            if h.uses_network() && !*online.get_or_insert_with(self.inner.online) {
                j.state = State::Waiting;
                j.last_error = Some("waiting: the app is offline (Settings → Network)".into());
                j.run_at = now + OFFLINE_WAIT_MS;
                *changed = true;
                continue;
            }
            *in_queue += 1;
            free -= 1;
            j.state = State::Running;
            j.attempts += 1;
            *changed = true;
            let cancel = Arc::new(AtomicBool::new(false));
            running.insert(j.id.clone(), (j.queue.clone(), cancel.clone()));
            claims.push(Claim { record: j.clone(), handler: h, cancel, engine: engine.clone(), started: now, background: resources.background_priority });
        }
        claims
    }

    /// Record how an attempt ended: done, retried after a backoff, or dead.
    fn finish(&self, claim: Claim, result: std::result::Result<Value, Failure>) {
        let now = self.now();
        let mut s = self.lock();
        let id = claim.record.id.clone();
        s.running.remove(&id);
        let cancelled = claim.cancel.load(Ordering::SeqCst) && !matches!(result, Err(Failure::TimedOut));
        let pos = s.file.jobs.iter().position(|j| j.id == id);
        let mut run = Run { job: id, kind: claim.record.kind.clone(), attempt: claim.record.attempts, started: claim.started, finished: now, outcome: Outcome::Ok, message: None, output: None, retry_at: None };
        match result {
            // Stopped by `shutdown`: back to the queue as it was; this attempt does not count.
            _ if cancelled && s.returning => {
                run.outcome = Outcome::Interrupted;
                run.message = Some(RETURNED.into());
                if let Some(j) = pos.map(|p| &mut s.file.jobs[p]).filter(|j| j.state == State::Running) {
                    give_back(j, now);
                }
            }
            _ if cancelled => {
                run.outcome = Outcome::Cancelled;
                if let Some(p) = pos {
                    s.file.jobs.remove(p);
                }
            }
            Ok(v) => {
                run.output = Some(v);
                if let Some(p) = pos {
                    s.file.jobs.remove(p);
                }
            }
            Err(f) => {
                let (outcome, message, retryable) = match f {
                    Failure::TimedOut => (Outcome::TimedOut, format!("timed out after {} s", claim.record.timeout_secs), true),
                    Failure::Error(e) => (Outcome::Failed, e.message, e.retry),
                };
                run.outcome = outcome;
                run.message = Some(message.clone());
                let (base, max) = (Duration::from_secs(s.config.backoff_base_secs), Duration::from_secs(s.config.backoff_max_secs));
                if let Some(j) = pos.map(|p| &mut s.file.jobs[p]) {
                    j.last_error = Some(message);
                    if retryable && j.attempts < j.max_attempts {
                        j.state = State::Retrying;
                        j.run_at = now + backoff(j.attempts, base, max, (self.inner.jitter)()).as_millis() as u64;
                        run.retry_at = Some(j.run_at);
                    } else {
                        j.state = State::Dead;
                        log::warn!("background job {} ({}) is dead after {} attempts: {}", j.id, j.kind, j.attempts, j.last_error.as_deref().unwrap_or(""));
                    }
                }
            }
        }
        if let Err(e) = self.save(&s) {
            log::warn!("background jobs: could not save the queue: {e}");
        }
        drop(s);
        self.log_run(&run);
        self.inner.wake.notify_all();
    }

    /// Milliseconds the dispatcher may sleep: until the next job or schedule is due.
    fn wait_ms(&self, s: &Shared) -> u64 {
        let now = self.now();
        let jobs = s.file.jobs.iter().filter(|j| matches!(j.state, State::Queued | State::Retrying | State::Waiting)).map(|j| j.run_at);
        let schedules = s.file.schedules.iter().filter(|x| !x.paused).filter_map(|x| x.next_run);
        jobs.chain(schedules).min().map_or(MAX_WAIT_MS, |t| t.saturating_sub(now).clamp(1, MAX_WAIT_MS))
    }

    fn history_file(&self) -> PathBuf {
        self.inner.dir.join("history.jsonl")
    }

    fn log_run(&self, run: &Run) {
        let line = match serde_json::to_string(run) {
            Ok(l) => l,
            Err(_) => return,
        };
        let written = std::fs::OpenOptions::new().create(true).append(true).open(self.history_file()).and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = written {
            log::warn!("background jobs: could not write the history: {e}");
        }
    }

    /// The last `n` runs, newest first.
    fn history(&self, n: usize) -> Vec<Run> {
        let text = std::fs::read_to_string(self.history_file()).unwrap_or_default();
        text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(n).collect()
    }

    /// Drop runs older than `days` (and beyond `HISTORY_KEPT`).
    fn prune_history(&self, now: u64, days: u32) {
        let Ok(text) = std::fs::read_to_string(self.history_file()) else { return };
        let cutoff = now.saturating_sub(u64::from(days) * DAY_MS);
        let lines: Vec<&str> = text.lines().collect();
        let mut keep: Vec<&str> = lines.iter().copied().filter(|l| serde_json::from_str::<Run>(l).is_ok_and(|r| r.finished >= cutoff)).collect();
        if keep.len() > HISTORY_KEPT {
            keep.drain(..keep.len() - HISTORY_KEPT);
        }
        if keep.len() == lines.len() {
            return;
        }
        let mut out = keep.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        if let Err(e) = crate::jobs::write_atomic(&self.history_file(), out.as_bytes()) {
            log::warn!("background jobs: could not prune the history: {e}");
        }
    }
}

fn read_file(dir: &Path) -> Result<QueueFile> {
    match std::fs::read(dir.join("queue.json")) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(QueueFile::default()),
        Err(e) => Err(e.into()),
    }
}

/// Run one attempt on its own thread and wait at most its timeout. A job that outlives its
/// timeout is told to stop (`is_cancelled`) and left to finish; its result is ignored.
fn execute(c: &Claim) -> std::result::Result<Value, Failure> {
    let (tx, rx) = mpsc::channel();
    let ctx = JobContext { id: c.record.id.clone(), kind: c.record.kind.clone(), attempt: c.record.attempts, cancel: c.cancel.clone(), engine: c.engine.clone() };
    let (handler, payload, background) = (c.handler.clone(), c.record.payload.clone(), c.background);
    let spawned = std::thread::Builder::new().name(format!("job-{}", c.record.kind)).spawn(move || {
        if background {
            crate::governor::background_thread();
        }
        let _ = tx.send(handler.run(payload, &ctx));
    });
    if let Err(e) = spawned {
        return Err(Failure::Error(JobError::retry(format!("could not start a thread: {e}"))));
    }
    match rx.recv_timeout(Duration::from_secs(c.record.timeout_secs)) {
        Ok(r) => r.map_err(Failure::Error),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            c.cancel.store(true, Ordering::SeqCst);
            Err(Failure::TimedOut)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Failure::Error(JobError::retry("the job panicked"))),
    }
}

/// Runs a queue in the background: one dispatcher thread that fires schedules and starts due
/// jobs, each on its own thread, within the limits. Stops (without waiting for running jobs,
/// which finish and are recorded) when dropped.
pub struct Runtime {
    queue: Queue,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Runtime {
    /// Start running `queue`. Fails when another process owns it.
    pub fn start(queue: Queue) -> Result<Self> {
        if !queue.is_owner() {
            return Err(Error::Source("background jobs are run by another process of this app".into()));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (q, halt) = (queue.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("jobs".into()).spawn(move || dispatch(&q, &halt))?;
        log::info!("background jobs: running ({})", queue.dir().display());
        Ok(Self { queue, stop, thread: Some(thread) })
    }

    /// The queue it runs.
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
}

fn dispatch(q: &Queue, stop: &AtomicBool) {
    let mut s = q.lock();
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let claims = q.tick(&mut s);
        if claims.is_empty() {
            let wait = Duration::from_millis(q.wait_ms(&s));
            s = q.inner.wake.wait_timeout(s, wait).unwrap_or_else(std::sync::PoisonError::into_inner).0;
            continue;
        }
        drop(s);
        for c in claims {
            let runner = q.clone();
            let kind = c.record.kind.clone();
            // `finish` runs on this thread too when the supervisor thread cannot start.
            let slot = Arc::new(Mutex::new(Some(c)));
            let taken = slot.clone();
            let spawned = std::thread::Builder::new().name(format!("jobs-{kind}")).spawn(move || {
                if let Some(c) = taken.lock().ok().and_then(|mut g| g.take()) {
                    let r = execute(&c);
                    runner.finish(c, r);
                }
            });
            if let Err(e) = spawned {
                if let Some(c) = slot.lock().ok().and_then(|mut g| g.take()) {
                    q.finish(c, Err(Failure::Error(JobError::retry(format!("could not start a thread: {e}")))));
                }
            }
        }
        s = q.lock();
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        {
            let _s = self.queue.lock();
            self.queue.inner.wake.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    const T0: u64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

    /// Fails `fail_first` times, then succeeds; counts its calls.
    struct Flaky {
        calls: Arc<AtomicUsize>,
        fail_first: usize,
        fatal: bool,
    }

    #[derive(Serialize, Deserialize)]
    struct N {
        n: u32,
    }

    impl Job for Flaky {
        type Payload = N;
        const NAME: &'static str = "flaky";
        fn queue(&self) -> &str {
            "net"
        }
        fn max_attempts(&self) -> Option<u32> {
            Some(3)
        }
        fn run(&self, p: N, _ctx: &JobContext) -> std::result::Result<Value, JobError> {
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            match i < self.fail_first {
                true if self.fatal => Err(JobError::fatal("broken for good")),
                true => Err(JobError::retry(format!("attempt {} failed", i + 1))),
                false => Ok(json!(p.n * 2)),
            }
        }
    }

    struct Slow;

    impl Job for Slow {
        type Payload = Value;
        const NAME: &'static str = "slow";
        fn run(&self, _p: Value, ctx: &JobContext) -> std::result::Result<Value, JobError> {
            while !ctx.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(json!("stopped"))
        }
    }

    struct Online;

    impl Job for Online {
        type Payload = Value;
        const NAME: &'static str = "online";
        fn uses_network(&self) -> bool {
            true
        }
        fn run(&self, _p: Value, _ctx: &JobContext) -> std::result::Result<Value, JobError> {
            Ok(Value::Null)
        }
    }

    fn setup(fail_first: usize, fatal: bool) -> (tempfile::TempDir, Arc<FakeClock>, Arc<AtomicUsize>, Queue) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(FakeClock::new(T0));
        let calls = Arc::new(AtomicUsize::new(0));
        let q = open(dir.path(), &clock, &calls, fail_first, fatal);
        (dir, clock, calls, q)
    }

    fn open(dir: &Path, clock: &Arc<FakeClock>, calls: &Arc<AtomicUsize>, fail_first: usize, fatal: bool) -> Queue {
        let mut r = Registry::new();
        r.register(Flaky { calls: calls.clone(), fail_first, fatal }).register(Slow).register(Online);
        r.schedule(Schedule::every("tick", "flaky", 60).with_payload(json!({"n": 1})).paused(true));
        let opts = QueueOptions { clock: clock.clone(), jitter: || 0.0, online: || false, engine: None };
        Queue::open_with(dir, r, JobsConfig::default(), opts).unwrap()
    }

    #[test]
    fn config_is_clamped() {
        let c = JobsConfig { workers: 0, max_attempts: 0, backoff_base_secs: 0, backoff_max_secs: 0, history_days: 0, queues: [(" ".into(), 3), ("x".into(), 0)].into(), ..JobsConfig::default() }.validated();
        assert_eq!((c.workers, c.max_attempts, c.backoff_base_secs, c.backoff_max_secs, c.history_days), (1, 1, 1, 1, 1));
        assert_eq!(c.queues, [("x".to_string(), 1)].into());
        assert!(!JobsConfig::default().enabled, "off by default");
    }

    #[test]
    fn backoff_doubles_with_jitter_and_a_cap() {
        let s = Duration::from_secs;
        assert_eq!(backoff(1, s(10), s(100), 0.0), s(5));
        assert_eq!(backoff(1, s(10), s(100), 1.0), s(10));
        assert_eq!(backoff(2, s(10), s(100), 0.5), Duration::from_millis(15_000));
        assert_eq!(backoff(4, s(10), s(100), 1.0), s(80));
        assert_eq!(backoff(5, s(10), s(100), 1.0), s(100), "capped");
        assert_eq!(backoff(u32::MAX, s(10), s(100), f64::NAN), Duration::from_millis(75_000));
        let u = random_unit();
        assert!((0.0..1.0).contains(&u));
    }

    #[test]
    fn delayed_jobs_wait_for_their_time() {
        let (_d, clock, calls, q) = setup(0, false);
        q.enqueue("flaky", json!({"n": 21}), Enqueue::after(Duration::from_secs(30))).unwrap();
        assert_eq!(q.run_pending(), 0);
        clock.advance(Duration::from_secs(30));
        assert_eq!(q.run_pending(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let l = q.list();
        assert!(l.jobs.is_empty() && l.dead.is_empty());
        assert_eq!((l.history[0].outcome, l.history[0].output.clone()), (Outcome::Ok, Some(json!(42))));
    }

    #[test]
    fn payloads_and_types_are_checked_on_enqueue() {
        let (_d, _c, _n, q) = setup(0, false);
        assert!(q.enqueue("flaky", json!({"n": "x"}), Enqueue::default()).is_err());
        assert!(q.enqueue("nope", json!({}), Enqueue::default()).is_err());
        assert_eq!(q.types(), ["flaky", "online", "slow"]);
    }

    #[test]
    fn failures_retry_with_backoff_then_go_to_the_dead_letters() {
        let (dir, clock, calls, q) = setup(usize::MAX, false);
        let job = q.enqueue("flaky", json!({"n": 1}), Enqueue::default()).unwrap();
        assert_eq!(job.max_attempts, 3, "the job type's own limit");
        assert_eq!(q.run_pending(), 1);
        let j = &q.list().jobs[0];
        assert_eq!((j.state, j.attempts, j.run_at), (State::Retrying, 1, T0 + 5_000), "base 10 s, jitter 0: 5 s");
        assert_eq!(q.list().history[0].retry_at, Some(T0 + 5_000));
        clock.advance(Duration::from_millis(4_999));
        assert_eq!(q.run_pending(), 0, "not yet");
        clock.advance(Duration::from_millis(1));
        assert_eq!(q.run_pending(), 1);
        assert_eq!(q.list().jobs[0].run_at, T0 + 5_000 + 10_000, "doubled");
        clock.advance(Duration::from_secs(10));
        assert_eq!(q.run_pending(), 1);
        let l = q.list();
        assert!(l.jobs.is_empty());
        assert_eq!((l.dead[0].state, l.dead[0].attempts), (State::Dead, 3));
        assert_eq!(l.dead[0].last_error.as_deref(), Some("attempt 3 failed"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // Dead letters survive a restart and can be retried by hand.
        drop(l);
        drop(q);
        let calls2 = Arc::new(AtomicUsize::new(0));
        let q = open(dir.path(), &clock, &calls2, 0, false);
        assert_eq!(q.list().dead.len(), 1);
        let again = q.retry(&job.id).unwrap();
        assert_eq!((again.state, again.attempts), (State::Queued, 0));
        assert_eq!(q.run_pending(), 1);
        assert!(q.list().dead.is_empty() && q.list().jobs.is_empty());
    }

    #[test]
    fn fatal_errors_skip_the_retries() {
        let (_d, _c, calls, q) = setup(usize::MAX, true);
        q.enqueue("flaky", json!({"n": 1}), Enqueue::default()).unwrap();
        q.run_pending();
        assert_eq!(q.list().dead.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_removes_waiting_and_dead_jobs() {
        let (_d, _c, _n, q) = setup(0, false);
        let j = q.enqueue("flaky", json!({"n": 1}), Enqueue::after(Duration::from_secs(60))).unwrap();
        assert!(q.cancel(&j.id).unwrap());
        assert!(!q.cancel(&j.id).unwrap());
        assert!(q.list().jobs.is_empty());
        assert_eq!(q.list().history[0].outcome, Outcome::Cancelled);
    }

    #[test]
    fn timeouts_count_as_failed_attempts() {
        let (_d, _c, _n, q) = setup(0, false);
        q.enqueue("slow", Value::Null, Enqueue { timeout_secs: Some(1), max_attempts: Some(1), ..Enqueue::default() }).unwrap();
        assert_eq!(q.run_pending(), 1);
        let l = q.list();
        assert_eq!(l.history[0].outcome, Outcome::TimedOut);
        assert_eq!(l.dead[0].last_error.as_deref(), Some("timed out after 1 s"));
    }

    #[test]
    fn network_jobs_wait_while_offline_without_using_attempts() {
        let (_d, clock, _n, q) = setup(0, false);
        q.enqueue("online", Value::Null, Enqueue::default()).unwrap();
        assert_eq!(q.run_pending(), 0);
        let j = &q.list().jobs[0];
        assert_eq!((j.state, j.attempts, j.run_at), (State::Waiting, 0, T0 + OFFLINE_WAIT_MS));
        clock.advance(Duration::from_secs(120));
        assert_eq!(q.run_pending(), 0, "still offline");
    }

    #[test]
    fn schedules_fire_on_time_and_can_be_paused() {
        let (dir, clock, calls, q) = setup(0, false);
        // The registry's schedule is added once, paused.
        let s = q.schedules();
        assert_eq!((s.len(), s[0].paused, s[0].next_run), (1, true, None));
        q.set_schedule(ScheduleUpdate { id: "tick".into(), paused: Some(false), ..Default::default() }).unwrap();
        assert_eq!(q.schedules()[0].next_run, Some(T0 + 60_000));
        assert_eq!(q.run_pending(), 0);
        clock.advance(Duration::from_secs(60));
        assert_eq!(q.run_pending(), 1);
        assert_eq!(q.schedules()[0].next_run, Some(T0 + 120_000));
        assert_eq!(q.schedules()[0].last_run, Some(T0 + 60_000));
        // Missed runs (the app was closed or suspended) fire once, not once per slot.
        clock.advance(Duration::from_secs(600));
        assert_eq!(q.run_pending(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // A cron schedule; bad ones are refused.
        let all = q.set_schedule(ScheduleUpdate { id: "five".into(), kind: Some("flaky".into()), payload: Some(json!({"n": 2})), cron: Some("*/5 * * * *".into()), ..Default::default() }).unwrap();
        assert_eq!(all.len(), 2);
        let now = clock.now_ms();
        assert_eq!(all[1].next_run, Some(next_cron("*/5 * * * *", now).unwrap()));
        assert!(q.set_schedule(ScheduleUpdate { id: "bad".into(), kind: Some("flaky".into()), payload: Some(json!({"n": 1})), cron: Some("61 * * * *".into()), ..Default::default() }).is_err());
        assert!(q.set_schedule(ScheduleUpdate { id: "bad".into(), kind: Some("flaky".into()), ..Default::default() }).is_err(), "no timing");
        q.set_schedule(ScheduleUpdate { id: "tick".into(), paused: Some(true), ..Default::default() }).unwrap();
        clock.advance(Duration::from_secs(3600));
        assert_eq!(q.run_pending(), 1, "only the cron schedule");
        // Removed registry schedules stay removed after a restart.
        q.set_schedule(ScheduleUpdate { id: "tick".into(), remove: true, ..Default::default() }).unwrap();
        drop(q);
        let q = open(dir.path(), &clock, &calls, 0, false);
        assert_eq!(q.schedules().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["five"]);
    }

    #[test]
    fn a_crash_mid_run_is_retried_and_one_process_owns_the_queue() {
        let (dir, clock, calls, q) = setup(0, false);
        let j = q.enqueue("flaky", json!({"n": 1}), Enqueue::default()).unwrap();
        // Claim it as the runtime would, then "crash" before it finishes.
        let claims = q.tick(&mut q.lock());
        assert_eq!(claims.len(), 1);
        let other = open(dir.path(), &clock, &calls, 0, false);
        assert!(!other.is_owner(), "the first process holds the lock");
        assert_eq!(other.list().jobs[0].state, State::Running, "others can read it");
        assert!(other.enqueue("flaky", json!({"n": 1}), Enqueue::default()).is_err());
        assert!(Runtime::start(other.clone()).is_err());
        drop(other);
        drop(claims);
        drop(q);
        let q = open(dir.path(), &clock, &calls, 0, false);
        assert!(q.is_owner());
        let r = &q.list().jobs[0];
        assert_eq!((r.id.as_str(), r.state, r.attempts), (j.id.as_str(), State::Retrying, 1));
        assert_eq!(q.list().history[0].outcome, Outcome::Interrupted);
        assert_eq!(q.run_pending(), 1);
    }

    #[test]
    fn queue_limits_and_history_pruning() {
        let (dir, clock, _n, q) = setup(0, false);
        q.configure(JobsConfig { workers: 4, queues: [("net".to_string(), 1)].into(), ..JobsConfig::default() }, ResourceConfig::default());
        for n in 0..3 {
            q.enqueue("flaky", json!({ "n": n }), Enqueue::default()).unwrap();
        }
        let claims = q.tick(&mut q.lock());
        assert_eq!(claims.len(), 1, "queue `net` runs one at a time");
        for c in claims {
            q.finish(c, Ok(Value::Null));
        }
        assert_eq!(q.run_pending(), 2);
        assert_eq!(q.list().history.len(), 3);
        clock.advance(Duration::from_secs(8 * 86_400));
        q.prune_history(clock.now_ms(), 7);
        assert!(q.list().history.is_empty());
        assert!(dir.path().join("queue/history.jsonl").exists());
    }

    #[test]
    fn the_runtime_runs_jobs_on_its_threads() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut r = Registry::new();
        r.register(Flaky { calls: calls.clone(), fail_first: 0, fatal: false }).register(Slow);
        let q = Queue::open(dir.path(), r, JobsConfig { enabled: true, ..JobsConfig::default() }).unwrap();
        let rt = Runtime::start(q.clone()).unwrap();
        let slow = q.enqueue("slow", Value::Null, Enqueue::default()).unwrap();
        q.enqueue("flaky", json!({"n": 1}), Enqueue::default()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while calls.load(Ordering::SeqCst) == 0 || q.list().jobs.iter().all(|j| j.state != State::Running) {
            assert!(std::time::Instant::now() < deadline, "the runtime did not run the jobs");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(q.cancel(&slow.id).unwrap(), "a running job is told to stop");
        while !q.list().jobs.is_empty() {
            assert!(std::time::Instant::now() < deadline, "the cancelled job did not stop");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(q.list().history.iter().any(|h| h.outcome == Outcome::Cancelled));
        drop(rt);
    }
    #[test]
    fn shutdown_hands_running_jobs_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::new();
        r.register(Slow);
        let q = Queue::open(dir.path(), r, JobsConfig { enabled: true, ..JobsConfig::default() }).unwrap();
        let rt = Runtime::start(q.clone()).unwrap();
        let slow = q.enqueue("slow", Value::Null, Enqueue::default()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while q.list().jobs.iter().all(|j| j.state != State::Running) {
            assert!(std::time::Instant::now() < deadline, "the runtime did not start the job");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(rt);
        assert_eq!(q.shutdown(Duration::from_secs(5)), 1);
        let jobs = q.list().jobs;
        assert_eq!(jobs.len(), 1, "the job is kept");
        assert_eq!((jobs[0].id.as_str(), jobs[0].state, jobs[0].attempts), (slow.id.as_str(), State::Queued, 0), "back as it was");
        assert!(q.list().history.iter().any(|h| h.outcome == Outcome::Interrupted));
        // Nothing new starts after a shutdown.
        assert_eq!(q.run_pending(), 0);
    }
}
