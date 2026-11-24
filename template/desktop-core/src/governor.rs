//! Resource limits for background work (`resources` in the config): how much of the CPU
//! ingest may use, and whether its threads run at background priority so the window and
//! the rest of the computer stay responsive. Plain OS calls, no crates:
//! macOS QoS "utility", Windows background mode (lower CPU and disk priority), Linux nice 10.
//!
//! Optional budgets (all off by default):
//! - `memory_budget_mb` caps ingest workers and embedding batches by the memory that is
//!   both free and inside the budget (`free_memory_mb`: `/proc/meminfo` on Linux,
//!   `sysctl`/`vm_stat` on macOS; elsewhere unknown, so half the budget is assumed). Over
//!   budget, work slows down (one worker, small batches) and files too large to fit are
//!   skipped with a reason, instead of the process running out of memory;
//! - `gpu_budget` caps the llama.cpp layers on the GPU at `gpu_layers` (0 = CPU only);
//! - `onnx_provider` is the preferred ONNX Runtime execution provider; one this build or
//!   platform lacks falls back to the CPU (`OnnxProvider::resolve`).
//!
//! ```
//! use desktop_core::governor::ResourceConfig;
//! let r = ResourceConfig { memory_budget_mb: 600, ..ResourceConfig::default() };
//! // 4 workers and batches of 64 asked for, 1000 MB free: the 600 MB budget allows 2 workers.
//! let plan = r.plan(4, 64, Some(1000));
//! assert_eq!(plan.workers, 2);
//! assert!(plan.degraded);
//! // No budget: nothing changes.
//! assert_eq!(ResourceConfig::default().plan(4, 64, Some(10)).workers, 4);
//! ```

use serde::{Deserialize, Serialize};

/// Memory one ingest worker is assumed to need besides its batch (reading, chunking, the
/// model's working set), in MB.
pub const WORKER_MB: u64 = 256;
/// Memory one text in an embedding batch is assumed to need, in MB.
pub const BATCH_ITEM_MB: u64 = 4;
/// A file is assumed to take this many times its size in memory while it is ingested.
pub const FILE_FACTOR: u64 = 4;

/// The ONNX Runtime execution provider to prefer (embeddings, speech).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum OnnxProvider {
    /// The CPU (always there).
    #[default]
    Cpu,
    /// The best one this build and platform have.
    Auto,
    /// Apple Core ML (macOS).
    Coreml,
    /// DirectML (Windows).
    Directml,
    /// NVIDIA CUDA (Linux, Windows).
    Cuda,
}

impl OnnxProvider {
    /// Whether the platform could run it at all.
    pub fn on_this_platform(self) -> bool {
        match self {
            Self::Cpu | Self::Auto => true,
            Self::Coreml => cfg!(target_os = "macos"),
            Self::Directml => cfg!(windows),
            Self::Cuda => cfg!(any(target_os = "linux", windows)),
        }
    }

    /// The provider to use for preference `self` when this build has `built` (the CPU is
    /// always there): the preference when built and on this platform, the first of `built`
    /// that is for `Auto`, else the CPU.
    pub fn resolve(self, built: &[OnnxProvider]) -> OnnxProvider {
        let usable = |p: &OnnxProvider| *p != Self::Auto && p.on_this_platform() && (*p == Self::Cpu || built.contains(p));
        match self {
            Self::Auto => built.iter().copied().find(|p| usable(p) && *p != Self::Cpu).unwrap_or(Self::Cpu),
            p if usable(&p) => p,
            _ => Self::Cpu,
        }
    }
}

/// What `ResourceConfig::plan` allows for one ingest run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Ingest workers to run.
    pub workers: usize,
    /// Texts per embedding call.
    pub embed_batch: usize,
    /// Memory the run may use in MB (`None`: no budget).
    pub memory_mb: Option<u64>,
    /// Fewer workers or a smaller batch than asked for, to stay inside the budget.
    pub degraded: bool,
}

impl Plan {
    /// Why a file of `bytes` would not fit this plan's memory, if it would not.
    pub fn too_large(&self, bytes: u64) -> Option<String> {
        let need = (bytes >> 20).saturating_mul(FILE_FACTOR);
        let have = self.memory_mb? / self.workers.max(1) as u64;
        (need > have).then(|| format!("needs about {need} MB, more than the {have} MB per worker the memory budget leaves"))
    }
}

/// How much of the computer background work may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceConfig {
    /// Share of the CPU cores background work may use (10 to 100).
    pub cpu_percent: u32,
    /// Run ingest threads at background priority.
    pub background_priority: bool,
    /// Continue an ingest that a crash or quit interrupted, at the next start.
    pub resume_interrupted: bool,
    /// Memory background work may use, in MB; 0 = no budget (the default).
    pub memory_budget_mb: u64,
    /// Cap the local model's GPU layers at `gpu_layers` (off: the AI settings decide).
    pub gpu_budget: bool,
    /// GPU layers allowed while `gpu_budget` is on; 0 = CPU only.
    pub gpu_layers: u32,
    /// Preferred ONNX Runtime execution provider; falls back to the CPU.
    pub onnx_provider: OnnxProvider,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            cpu_percent: 100,
            background_priority: true,
            resume_interrupted: true,
            memory_budget_mb: 0,
            gpu_budget: false,
            gpu_layers: 0,
            onnx_provider: OnnxProvider::Cpu,
        }
    }
}

impl ResourceConfig {
    /// The same config with every value in a safe range (called from `Config::validated`).
    pub fn validated(mut self) -> Self {
        self.cpu_percent = self.cpu_percent.clamp(10, 100);
        // Below 128 MB nothing useful runs; a budget that small means "as little as possible".
        self.memory_budget_mb = match self.memory_budget_mb {
            0 => 0,
            mb => mb.clamp(128, 1 << 30),
        };
        self.gpu_layers = self.gpu_layers.min(1000);
        self
    }

    /// The memory work may use now: the budget, or less when less is free.
    /// `None` when there is no budget.
    pub fn memory_mb(&self, free_mb: Option<u64>) -> Option<u64> {
        (self.memory_budget_mb > 0).then(|| self.memory_budget_mb.min(free_mb.unwrap_or(self.memory_budget_mb / 2)))
    }

    /// Workers and embedding batch for an ingest run that asks for `workers` (already
    /// capped by `workers`) and `embed_batch`, with `free_mb` free (`free_memory_mb()`).
    pub fn plan(&self, workers: usize, embed_batch: usize, free_mb: Option<u64>) -> Plan {
        let (workers, embed_batch) = (workers.max(1), embed_batch.max(1));
        let Some(mem) = self.memory_mb(free_mb) else {
            return Plan { workers, embed_batch, memory_mb: None, degraded: false };
        };
        let w = workers.min((mem / WORKER_MB).max(1) as usize);
        let per_worker = mem / w as u64;
        let b = embed_batch.min((per_worker.saturating_sub(WORKER_MB / 2) / BATCH_ITEM_MB).max(1) as usize);
        let plan = Plan { workers: w, embed_batch: b, memory_mb: Some(mem), degraded: w < workers || b < embed_batch };
        if plan.degraded {
            log::info!("memory budget: {w} of {workers} workers, batches of {b} of {embed_batch} ({mem} MB available)");
        }
        plan
    }

    /// GPU layers for the local model that asks for `requested`.
    pub fn gpu_layers(&self, requested: u32) -> u32 {
        if self.gpu_budget {
            requested.min(self.gpu_layers)
        } else {
            requested
        }
    }

    /// Why a model file of `bytes` may not be loaded under the memory budget, if so.
    pub fn model_too_large(&self, bytes: u64, free_mb: Option<u64>) -> Option<String> {
        let mem = self.memory_mb(free_mb)?;
        let need = bytes >> 20;
        (need > mem).then(|| format!("the model needs about {need} MB, more than the {mem} MB the memory budget allows (Settings → Performance)"))
    }

    /// The threads to use for `requested` workers under the CPU share.
    pub fn workers(&self, requested: usize) -> usize {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let cap = (cores * self.cpu_percent as usize).div_ceil(100).max(1);
        requested.clamp(1, cap)
    }
}

/// Memory free for new work, in MB (`None` when this platform gives no cheap answer).
pub fn free_memory_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| meminfo_available_mb(&t))
    }
    #[cfg(target_os = "macos")]
    {
        let run = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let page = run("/usr/sbin/sysctl", &["-n", "hw.pagesize"]).and_then(|p| p.trim().parse::<u64>().ok());
        run("/usr/bin/vm_stat", &[]).and_then(|t| vm_stat_available_mb(&t, page))
    }
    // Windows and the rest: unknown (no extra crates); `memory_mb` then assumes half the budget.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}

/// `MemAvailable` from `/proc/meminfo` text, in MB.
pub fn meminfo_available_mb(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Free, inactive, speculative and purgeable pages from `vm_stat` output, in MB. The page
/// size comes from `page_size` (`sysctl hw.pagesize`) or the output's own header.
pub fn vm_stat_available_mb(vm_stat: &str, page_size: Option<u64>) -> Option<u64> {
    let header = vm_stat.split("page size of ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok());
    let page = page_size.or(header)?;
    let pages = |name: &str| {
        vm_stat.lines().find(|l| l.trim_start().starts_with(name)).and_then(|l| l.rsplit(':').next()).and_then(|v| v.trim().trim_end_matches('.').parse::<u64>().ok())
    };
    let free = pages("Pages free")?;
    let more: u64 = ["Pages inactive", "Pages speculative", "Pages purgeable"].iter().filter_map(|n| pages(n)).sum();
    Some((free + more).saturating_mul(page) >> 20)
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

    #[test]
    fn memory_budget_plans() {
        let off = ResourceConfig::default();
        assert_eq!(off.plan(8, 64, Some(1)), Plan { workers: 8, embed_batch: 64, memory_mb: None, degraded: false }, "off by default");
        let r = ResourceConfig { memory_budget_mb: 2048, ..off.clone() }.validated();
        let roomy = r.plan(4, 64, Some(100_000));
        assert_eq!((roomy.workers, roomy.embed_batch, roomy.memory_mb, roomy.degraded), (4, 64, Some(2048), false));
        // Little free memory: fewer workers and smaller batches, never zero.
        let tight = r.plan(4, 64, Some(300));
        assert_eq!((tight.workers, tight.embed_batch, tight.degraded), (1, 43, true));
        let starved = r.plan(4, 64, Some(0));
        assert_eq!((starved.workers, starved.embed_batch), (1, 1));
        // Free memory unknown: half the budget.
        assert_eq!(r.memory_mb(None), Some(1024));
        assert_eq!(ResourceConfig { memory_budget_mb: 5, ..off.clone() }.validated().memory_budget_mb, 128);
        // A file four times larger than a worker's share is skipped, with a reason.
        assert!(tight.too_large(10 << 20).is_none());
        assert!(tight.too_large(200 << 20).unwrap().contains("memory budget"));
        assert!(off.plan(1, 1, None).too_large(u64::MAX).is_none());
        assert!(r.model_too_large(4 << 30, Some(100_000)).is_some());
        assert!(off.model_too_large(4 << 30, Some(1)).is_none());
    }

    #[test]
    fn gpu_and_onnx_preferences() {
        let r = ResourceConfig::default();
        assert_eq!(r.gpu_layers(99), 99, "off: the AI settings decide");
        let capped = ResourceConfig { gpu_budget: true, gpu_layers: 20, ..r.clone() };
        assert_eq!((capped.gpu_layers(99), capped.gpu_layers(10)), (20, 10));
        assert_eq!(ResourceConfig { gpu_budget: true, ..r }.gpu_layers(99), 0, "CPU only");
        assert_eq!(OnnxProvider::Coreml.resolve(&[]), OnnxProvider::Cpu, "not built in");
        assert_eq!(OnnxProvider::Auto.resolve(&[OnnxProvider::Cpu]), OnnxProvider::Cpu);
        let native = if cfg!(target_os = "macos") { OnnxProvider::Coreml } else if cfg!(windows) { OnnxProvider::Directml } else { OnnxProvider::Cuda };
        assert_eq!(native.resolve(&[native]), native);
        assert_eq!(OnnxProvider::Auto.resolve(&[native]), native);
        let foreign = if cfg!(target_os = "macos") { OnnxProvider::Directml } else { OnnxProvider::Coreml };
        assert_eq!(foreign.resolve(&[foreign]), OnnxProvider::Cpu, "wrong platform");
    }

    #[test]
    fn reads_free_memory() {
        let meminfo = "MemTotal:       16314508 kB\nMemFree:         1000000 kB\nMemAvailable:    8388608 kB\n";
        assert_eq!(meminfo_available_mb(meminfo), Some(8192));
        let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               65536.\nPages active:                            100.\nPages inactive:                           65536.\nPages speculative:                            0.\n";
        assert_eq!(vm_stat_available_mb(vm, None), Some(2048));
        assert_eq!(vm_stat_available_mb(vm, Some(4096)), Some(512));
        assert_eq!(vm_stat_available_mb("nothing", None), None);
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(free_memory_mb().is_some_and(|mb| mb > 0));
        }
    }
}
