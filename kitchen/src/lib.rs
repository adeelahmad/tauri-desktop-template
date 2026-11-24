//! # Kitchen
//!
//! The command line for the Kitchen desktop template: create an app, turn features on and
//! off after the fact, generate code in the app's own style, check the machine, and offer
//! all of it to AI agents as a local MCP server.
//!
//! ```text
//! kitchen new notes --enable email,ai      # a new app with two extra features
//! cd notes
//! kitchen features                         # what is on, what is off
//! kitchen enable tts                       # turn one on later; your edits are kept
//! kitchen generate module invoices --config
//! kitchen generate setting invoices.due_days int 30 --label "Days until due"
//! kitchen generate command export_csv
//! kitchen generate page Reports
//! kitchen hooks install                    # fmt + clippy (and prettier/eslint) before each commit
//! kitchen doctor                           # what this machine is missing, and the fix
//! kitchen service install                  # run the app's daemon as a user service
//! kitchen mcp                              # the same, as MCP tools over stdio
//! ```
//!
//! ## How `enable` keeps your changes
//!
//! An app records the answers it was generated with in `kitchen.toml`. Changing an answer
//! renders the template twice, with the old answers (*base*) and the new ones (*next*),
//! and applies the difference to your files as a three-way merge ([`merge`]): files you
//! never touched are replaced, files you changed are merged (with `git merge-file`), and
//! a clash is marked like a git conflict and listed. Nothing is changed on `--dry-run`.
//!
//! ## Use it as a library
//!
//! Every command is a function that returns a [`Report`] (and prints nothing), so the
//! CLI, the MCP server and your own tools share one implementation:
//!
//! ```no_run
//! use kitchen::{project::Project, Options};
//!
//! let project = Project::find(std::path::Path::new("."))?;
//! let report = project.set_features(&["email"], &[], &Options { dry_run: true })?;
//! println!("{report}");
//! # Ok::<(), kitchen::Error>(())
//! ```
//!
//! Modules: [`manifest`] (`kitchen.toml`), [`template`] (fetch and render the template),
//! [`merge`] (three-way apply), [`project`] (the commands on an app), [`generate`] (code
//! generators), [`doctor`] (machine checks), [`hooks`] (the app's git hooks), [`service`]
//! (the app's daemon as a service), [`mcp`] (the MCP server), [`cli`] (arguments).

pub mod cli;
pub mod doctor;
pub mod generate;
pub mod hooks;
pub mod manifest;
pub mod mcp;
pub mod merge;
pub mod project;
pub mod service;
pub mod template;

use std::fmt;

/// Everything that can go wrong, as a message a person can act on.
#[derive(Debug)]
pub enum Error {
    /// A file or process could not be read, written or started.
    Io(String, std::io::Error),
    /// The request does not make sense here (bad name, unknown feature, no project ...).
    Usage(String),
    /// A tool this needs (git, cargo-generate) failed or is missing.
    Tool(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(what, e) => write!(f, "{what}: {e}"),
            Error::Usage(m) | Error::Tool(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

/// `Result` with [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Attach what was being done to an I/O error.
pub(crate) fn io<T>(what: impl fmt::Display, r: std::io::Result<T>) -> Result<T> {
    r.map_err(|e| Error::Io(what.to_string(), e))
}

/// Options shared by the commands that change files.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Show what would change and change nothing.
    pub dry_run: bool,
}

/// What a command did (or, on a dry run, would do): lines for people, and JSON for tools.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Report {
    /// One-line outcome.
    pub summary: String,
    /// One entry per file or check, e.g. `update src-tauri/Cargo.toml`.
    pub lines: Vec<String>,
    /// Things the person should do next.
    pub next: Vec<String>,
}

impl Report {
    /// A report with just a summary.
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ..Self::default()
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.summary)?;
        for l in &self.lines {
            writeln!(f, "  {l}")?;
        }
        if !self.next.is_empty() {
            writeln!(f, "\nNext:")?;
            for n in &self.next {
                writeln!(f, "  {n}")?;
            }
        }
        Ok(())
    }
}

/// A scratch folder removed when dropped (no `tempfile` dependency).
pub(crate) struct TempDir(std::path::PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("kitchen-{tag}-{}-{nanos}", std::process::id()));
        io(
            format!("create {}", p.display()),
            std::fs::create_dir_all(&p),
        )?;
        Ok(Self(p))
    }
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
