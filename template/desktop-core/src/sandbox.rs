//! Run a parser in a child process with a time limit and a memory limit (feature `pdf`).
//!
//! [`run`] starts a command, writes the input to its stdin, and collects its stdout. The
//! child is killed when it runs past [`Limits::timeout`], and its memory is capped at
//! [`Limits::memory_mb`]: with `setrlimit` (`RLIMIT_AS`, `RLIMIT_DATA`, plus a CPU-time
//! backstop and no core dumps) on Linux and macOS, and with a Job Object on Windows (which
//! also stops the child from starting processes of its own). macOS does not enforce
//! `RLIMIT_AS`, so there the time limit and the output cap are the hard bounds.
//!
//! The app uses it to parse PDFs as `<app> --parse` (see the `pdf` module): a parser that
//! crashes, loops or eats memory then takes down only the child.
//!
//! # Example
//!
//! ```
//! use desktop_core::sandbox::{run, Limits};
//! use std::time::Duration;
//!
//! # #[cfg(unix)] {
//! let limits = Limits { timeout: Duration::from_secs(10), memory_mb: 512 };
//! let out = run(std::process::Command::new("cat"), b"hello", &limits, 1024)?;
//! assert_eq!(out, b"hello");
//! # }
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::{Error, Result};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long, and with how much memory, a child may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Wall-clock time before the child is killed.
    pub timeout: Duration,
    /// Address space (Linux) or committed memory (Windows) in MiB; 0 means no memory limit.
    pub memory_mb: u64,
}

/// The child's stderr kept for the error message (the end of it).
const STDERR_KEEP: usize = 4096;

fn failed(e: impl std::fmt::Display) -> Error {
    Error::Source(format!("parser process: {e}"))
}

/// Run `cmd` with `input` on stdin and return its stdout, which must be at most `max_output`
/// bytes. A non-zero exit, a crash, the time limit or a larger output is an error (with the
/// end of the child's stderr when it wrote any).
pub fn run(mut cmd: Command, input: &[u8], limits: &Limits, max_output: usize) -> Result<Vec<u8>> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // glibc keeps a large virtual arena per thread, which counts against RLIMIT_AS.
    cmd.env("MALLOC_ARENA_MAX", "2");
    #[cfg(unix)]
    unix::limit(&mut cmd, limits);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(failed)?;
    #[cfg(windows)]
    let _job = windows::limit(&child, limits).map_err(failed)?;

    let mut stdin = child.stdin.take();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        if let Some(s) = stdin.as_mut() {
            // A child that exits early closes the pipe: that is its answer, not our error.
            let _ = s.write_all(&input);
        }
        drop(stdin);
    });
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.take(max_output as u64 + 1).read_to_end(&mut out);
        }
        out
    });
    let mut stderr = child.stderr.take();
    let err_reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
                if out.len() > 2 * STDERR_KEEP {
                    out.drain(..out.len() - STDERR_KEEP);
                }
            }
        }
        out
    });

    let deadline = Instant::now() + limits.timeout;
    let status = loop {
        match child.try_wait().map_err(failed)? {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                // Its whole process group (not yet reaped, so the id is still its own): anything
                // it started goes too, so no leftover holds the pipes open.
                #[cfg(unix)]
                unix::kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let _ = writer.join();
    let out = reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    let err = String::from_utf8_lossy(&err[err.len().saturating_sub(STDERR_KEEP)..]).trim().to_string();
    let detail = if err.is_empty() { String::new() } else { format!(": {err}") };
    let Some(status) = status else {
        return Err(failed(format!("stopped after the {} s time limit", limits.timeout.as_secs_f32())));
    };
    if !status.success() {
        return Err(failed(format!("{status} (it crashed, ran out of memory or refused the input){detail}")));
    }
    if out.len() > max_output {
        return Err(failed(format!("wrote more than {max_output} bytes")));
    }
    Ok(out)
}

#[cfg(unix)]
mod unix {
    use super::Limits;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    /// Kill the child's process group (it leads its own, see [`limit`]).
    pub fn kill_group(pid: u32) {
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // SAFETY: kill has no memory effects; a negative pid names the process group.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }

    pub fn limit(cmd: &mut Command, limits: &Limits) {
        // Its own process group, so a timeout can stop everything it started.
        cmd.process_group(0);
        let bytes = limits.memory_mb.saturating_mul(1024 * 1024);
        // CPU time a little past the wall clock: a backstop if the parent dies.
        let cpu = limits.timeout.as_secs().saturating_add(5);
        // SAFETY: the closure runs in the forked child before exec and only calls
        // setrlimit, which is async-signal-safe; it touches no locks or allocations.
        unsafe {
            cmd.pre_exec(move || {
                let set = |what, value: u64| {
                    let r = libc::rlimit { rlim_cur: value as libc::rlim_t, rlim_max: value as libc::rlim_t };
                    libc::setrlimit(what, &r);
                };
                if bytes > 0 {
                    set(libc::RLIMIT_AS, bytes);
                    set(libc::RLIMIT_DATA, bytes);
                }
                set(libc::RLIMIT_CPU, cpu);
                set(libc::RLIMIT_CORE, 0);
                Ok(())
            });
        }
    }
}

#[cfg(windows)]
mod windows {
    //! A Job Object through four kernel32 calls (no extra crate).
    use super::Limits;
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    // JOBOBJECT_EXTENDED_LIMIT_INFORMATION is 144 bytes on 64-bit Windows.
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(std::mem::size_of::<ExtendedLimits>() == 144);

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const LIMIT_ACTIVE_PROCESS: u32 = 0x0008;
    const LIMIT_PROCESS_MEMORY: u32 = 0x0100;
    const LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0400;
    const LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, len: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// The job; closing it (on drop) kills the child if it still runs.
    pub struct Job(Handle);

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn limit(child: &std::process::Child, limits: &Limits) -> std::io::Result<Job> {
        // SAFETY: plain Win32 calls with valid pointers to live, correctly laid out structs.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = Job(job);
            let mut info = ExtendedLimits::default();
            info.basic.limit_flags = LIMIT_ACTIVE_PROCESS | LIMIT_DIE_ON_UNHANDLED_EXCEPTION | LIMIT_KILL_ON_JOB_CLOSE;
            info.basic.active_process_limit = 1;
            if limits.memory_mb > 0 {
                info.basic.limit_flags |= LIMIT_PROCESS_MEMORY;
                info.process_memory_limit = usize::try_from(limits.memory_mb.saturating_mul(1024 * 1024)).unwrap_or(usize::MAX);
            }
            let size = std::mem::size_of::<ExtendedLimits>() as u32;
            if SetInformationJobObject(job.0, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, (&info as *const ExtendedLimits).cast(), size) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if AssignProcessToJobObject(job.0, child.as_raw_handle()) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    const LIMITS: Limits = Limits { timeout: Duration::from_secs(20), memory_mb: 512 };

    #[test]
    fn passes_input_and_output() {
        assert_eq!(run(sh("cat"), b"abc", &LIMITS, 10).unwrap(), b"abc");
    }

    #[test]
    fn a_hang_is_killed_at_the_time_limit() {
        let started = Instant::now();
        let limits = Limits { timeout: Duration::from_millis(300), ..LIMITS };
        let e = run(sh("sleep 30"), b"", &limits, 10).unwrap_err().to_string();
        assert!(e.contains("time limit"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_crash_is_an_error_with_its_stderr() {
        let e = run(sh("echo boom >&2; exit 3"), b"", &LIMITS, 10).unwrap_err().to_string();
        assert!(e.contains("boom"), "{e}");
    }

    #[test]
    fn output_is_capped() {
        assert!(run(sh("head -c 5000 /dev/zero"), b"", &LIMITS, 100).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn memory_is_limited() {
        let out = run(sh("ulimit -v"), b"", &LIMITS, 100).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), (512 * 1024).to_string());
    }
}
