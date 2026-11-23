//! Crash-safe ingest: every multi-file ingest is written to a journal before it starts
//! and each finished file is appended as it completes, so a crash, a power cut or a quit
//! mid-way leaves a record of what is left. The next start resumes it (when
//! `ingest.resumeInterrupted` is on). Standard library only.
//!
//! Layout: `<data>/jobs/<id>.json` (the file list, written atomically and synced) and
//! `<data>/jobs/<id>.done` (one index per finished file, appended and synced). Both go
//! when the job ends. A store write for one file is replace-by-source, so a file cut off
//! half-way is simply ingested again.

use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobFile {
    pub id: String,
    pub created: String,
    pub paths: Vec<PathBuf>,
}

/// A job that did not finish: what is left of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub id: String,
    pub created: String,
    pub total: usize,
    pub remaining: Vec<PathBuf>,
}

pub struct Journal {
    dir: PathBuf,
}

/// Write `bytes` to `path` through a synced temporary file and a rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}tmp", path.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

impl Journal {
    pub fn new(data_dir: &Path) -> Self {
        Self { dir: data_dir.join("jobs") }
    }

    /// Record a job before it starts.
    pub fn begin(&self, paths: &[PathBuf]) -> Result<Job> {
        std::fs::create_dir_all(&self.dir)?;
        let now = std::time::SystemTime::now();
        let id = format!("{}-{}", now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(), std::process::id());
        let job = JobFile { id: id.clone(), created: crate::logging::rfc3339(now), paths: paths.to_vec() };
        write_atomic(&self.dir.join(format!("{id}.json")), &serde_json::to_vec(&job)?)?;
        let done = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(format!("{id}.done")))?;
        Ok(Job { dir: self.dir.clone(), id, done: Mutex::new(Some(done)) })
    }

    /// Pick up a pending job to continue it (its finished files stay finished).
    pub fn reopen(&self, id: &str) -> Result<Job> {
        let done = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(format!("{id}.done")))?;
        Ok(Job { dir: self.dir.clone(), id: id.into(), done: Mutex::new(Some(done)) })
    }

    /// Jobs that did not finish, oldest first. Unreadable records are dropped.
    pub fn pending(&self) -> Vec<Pending> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return out };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(job) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<JobFile>(&b).ok()) else {
                let _ = std::fs::remove_file(&p);
                continue;
            };
            let done: HashSet<usize> = std::fs::read_to_string(p.with_extension("done")).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect();
            let remaining: Vec<PathBuf> = job.paths.iter().enumerate().filter(|(i, _)| !done.contains(i)).map(|(_, p)| p.clone()).collect();
            if remaining.is_empty() {
                self.discard(&job.id);
                continue;
            }
            out.push(Pending { id: job.id, created: job.created, total: job.paths.len(), remaining });
        }
        out.sort_by(|a, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
        out
    }

    /// The original list of a pending job (indices for `Job::done`).
    pub fn paths(&self, id: &str) -> Option<Vec<PathBuf>> {
        std::fs::read(self.dir.join(format!("{id}.json"))).ok().and_then(|b| serde_json::from_slice::<JobFile>(&b).ok()).map(|j| j.paths)
    }

    pub fn discard(&self, id: &str) {
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.json")));
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.done")));
    }
}

/// A job in progress.
pub struct Job {
    dir: PathBuf,
    pub id: String,
    done: Mutex<Option<std::fs::File>>,
}

impl Job {
    /// File `index` of the job's list is finished (synced, so it survives a crash).
    pub fn done(&self, index: usize) {
        if let Ok(mut g) = self.done.lock() {
            if let Some(f) = g.as_mut() {
                let _ = writeln!(f, "{index}").and_then(|_| f.sync_data());
            }
        }
    }

    /// The whole list is done: forget the job.
    pub fn finish(self) {
        if let Ok(mut g) = self.done.lock() {
            *g = None;
        }
        Journal { dir: self.dir.clone() }.discard(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interrupted_job_lists_what_is_left() {
        let dir = std::env::temp_dir().join(format!("jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let j = Journal::new(&dir);
        let paths: Vec<PathBuf> = ["a.md", "b.md", "c.md"].iter().map(PathBuf::from).collect();
        let job = j.begin(&paths).unwrap();
        job.done(0);
        job.done(2);
        drop(job); // the "crash"
        let p = j.pending();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].total, p[0].remaining.clone()), (3, vec![PathBuf::from("b.md")]));
        let again = j.reopen(&p[0].id).unwrap();
        again.done(1);
        again.finish();
        assert!(j.pending().is_empty());
        assert!(std::fs::read_dir(dir.join("jobs")).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
