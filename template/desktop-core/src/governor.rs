//! Resource limits for background work (`resources` in the config): how much of the CPU
//! ingest may use, and whether its threads run at background priority so the window and
//! the rest of the computer stay responsive. Plain OS calls, no crates:
//! macOS QoS "utility", Windows background mode (lower CPU and disk priority), Linux nice 10.
//!
//! # Example
//!
//! ```
//! use desktop_core::governor::{background_thread, ResourceConfig};
//!
//! let half = ResourceConfig { cpu_percent: 50, ..ResourceConfig::default() }.validated();
//! let workers = half.workers(64);
//! assert!((1..=64).contains(&workers));
//! std::thread::spawn(|| {
//!     background_thread(); // best effort; the work below runs either way
//! })
//! .join()
//! .unwrap();
//! ```

use serde::{Deserialize, Serialize};

/// How much of the computer background work may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceConfig {
    /// Share of the CPU cores background work may use (10 to 100).
    pub cpu_percent: u32,
    /// Run ingest threads at background priority.
    pub background_priority: bool,
    /// Continue an ingest that a crash or quit interrupted, at the next start.
    pub resume_interrupted: bool,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self { cpu_percent: 100, background_priority: true, resume_interrupted: true }
    }
}

impl ResourceConfig {
    /// The share clamped to 10..=100 percent.
    pub fn validated(mut self) -> Self {
        self.cpu_percent = self.cpu_percent.clamp(10, 100);
        self
    }

    /// The threads to use for `requested` workers under the CPU share.
    pub fn workers(&self, requested: usize) -> usize {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let cap = (cores * self.cpu_percent as usize).div_ceil(100).max(1);
        requested.clamp(1, cap)
    }
}

/// Lower the calling thread's priority (best effort; returns whether it worked).
pub fn background_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        const QOS_CLASS_UTILITY: u32 = 0x11;
        // SAFETY: only changes the calling thread's scheduling class.
        unsafe { pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0) == 0 }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        const THREAD_MODE_BACKGROUND_BEGIN: i32 = 0x0001_0000;
        // SAFETY: the pseudo handle always refers to the calling thread.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) != 0 }
    }
    #[cfg(target_os = "linux")]
    {
        extern "C" {
            fn gettid() -> i32;
            fn setpriority(which: i32, who: u32, prio: i32) -> i32;
        }
        const PRIO_PROCESS: i32 = 0;
        // SAFETY: on Linux a thread id names just that thread for setpriority.
        unsafe { setpriority(PRIO_PROCESS, gettid() as u32, 10) == 0 }
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_priority() {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let full = ResourceConfig::default();
        assert_eq!(full.workers(10_000), cores);
        assert_eq!(full.workers(0), 1);
        let tenth = ResourceConfig { cpu_percent: 1, ..full.clone() }.validated();
        assert_eq!(tenth.cpu_percent, 10);
        assert_eq!(tenth.workers(10_000), cores.div_ceil(10).max(1));
        let worked = std::thread::spawn(background_thread).join().unwrap();
        assert!(worked || cfg!(not(any(target_os = "macos", windows, target_os = "linux"))));
    }
}
