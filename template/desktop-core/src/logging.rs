//! Structured logs: one JSON object per line in `<data>/logs/<process>.log`, rotated by
//! size, through the `log` facade (so Tauri's and the libraries' own messages land in the
//! same file). Debug builds also print to stderr. The home folder is written as `~`, so a
//! log can be shared without the user name in it.
//!
//! The level comes from Settings (`log.level`) and can be overridden with `APP_LOG`, as a
//! level (`debug`) or per target (`warn,desktop_core=debug`).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogConfig {
    /// error, warn, info, debug or trace.
    pub level: String,
    /// Write the log file (off = stderr in debug builds only).
    pub file: bool,
    /// Rotate when the file reaches this size.
    pub max_mb: u32,
    /// Rotated files to keep.
    pub keep: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self { level: "info".into(), file: true, max_mb: 10, keep: 5 }
    }
}

impl LogConfig {
    pub fn validated(mut self) -> Self {
        if parse_level(&self.level).is_none() {
            self.level = "info".into();
        }
        self.max_mb = self.max_mb.clamp(1, 1024);
        self.keep = self.keep.min(50);
        self
    }
}

fn parse_level(s: &str) -> Option<log::LevelFilter> {
    match s.trim().to_ascii_lowercase().as_str() {
        "off" => Some(log::LevelFilter::Off),
        "error" => Some(log::LevelFilter::Error),
        "warn" | "warning" => Some(log::LevelFilter::Warn),
        "info" => Some(log::LevelFilter::Info),
        "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace),
        _ => None,
    }
}

/// `warn,desktop_core=debug` as a default level and per-target levels.
fn parse_filter(spec: &str, fallback: log::LevelFilter) -> (log::LevelFilter, Vec<(String, log::LevelFilter)>) {
    let mut default = fallback;
    let mut targets = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('=') {
            Some((t, l)) => {
                if let Some(l) = parse_level(l) {
                    targets.push((t.trim().replace('-', "_"), l));
                }
            }
            None => {
                if let Some(l) = parse_level(part) {
                    default = l;
                }
            }
        }
    }
    targets.sort_by_key(|(t, _)| std::cmp::Reverse(t.len())); // most specific first
    (default, targets)
}

/// RFC 3339 in UTC with milliseconds, e.g. 2026-11-23T05:40:12.345Z.
pub fn rfc3339(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, d.subsec_millis())
}

/// Replace the home folder with `~`.
pub fn scrub(text: &str) -> String {
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    if home.len() > 1 {
        text.replace(&home, "~")
    } else {
        text.to_string()
    }
}

struct Sink {
    path: PathBuf,
    file: Option<std::fs::File>,
    size: u64,
}

struct Logger {
    filter: RwLock<(log::LevelFilter, Vec<(String, log::LevelFilter)>)>,
    config: RwLock<LogConfig>,
    sink: Mutex<Option<Sink>>,
    path: PathBuf,
    env_override: Option<String>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

impl Logger {
    fn level_for(&self, target: &str) -> log::LevelFilter {
        let f = self.filter.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let target = target.replace('-', "_");
        f.1.iter().find(|(t, _)| target == *t || target.starts_with(&format!("{t}::"))).map_or(f.0, |(_, l)| *l)
    }

    fn rotate(&self, sink: &mut Sink) {
        let keep = self.config.read().map(|c| c.keep).unwrap_or(5);
        sink.file = None;
        let numbered = |n: u32| sink.path.with_extension(format!("{n}.log"));
        let _ = std::fs::remove_file(numbered(keep.max(1)));
        for n in (1..keep.max(1)).rev() {
            let _ = std::fs::rename(numbered(n), numbered(n + 1));
        }
        if keep == 0 {
            let _ = std::fs::remove_file(&sink.path);
        } else {
            let _ = std::fs::rename(&sink.path, numbered(1));
        }
        sink.size = 0;
    }
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= self.level_for(m.target())
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let now = rfc3339(SystemTime::now());
        let line = serde_json::json!({
            "t": now,
            "level": record.level().as_str().to_ascii_lowercase(),
            "target": record.target(),
            "msg": scrub(&record.args().to_string()),
            "thread": std::thread::current().name().unwrap_or(""),
        })
        .to_string();
        if cfg!(debug_assertions) {
            eprintln!("{} {:5} {}: {}", &now[11..23], record.level(), record.target(), record.args());
        }
        let Ok(mut guard) = self.sink.lock() else { return };
        let Some(sink) = guard.as_mut() else { return };
        let max = self.config.read().map(|c| c.max_mb as u64).unwrap_or(10) << 20;
        if sink.size + line.len() as u64 + 1 > max {
            self.rotate(sink);
        }
        if sink.file.is_none() {
            sink.file = std::fs::OpenOptions::new().create(true).append(true).open(&sink.path).ok();
            sink.size = std::fs::metadata(&sink.path).map(|m| m.len()).unwrap_or(0);
        }
        if let Some(f) = sink.file.as_mut() {
            if writeln!(f, "{line}").is_ok() {
                sink.size += line.len() as u64 + 1;
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut g) = self.sink.lock() {
            if let Some(f) = g.as_mut().and_then(|s| s.file.as_mut()) {
                let _ = f.flush();
            }
        }
    }
}

/// Start logging for this process into `<dir>/<process>.log` (once; later calls apply
/// `config` only). Returns the log file.
pub fn init(dir: &Path, process: &str, config: LogConfig) -> PathBuf {
    let path = dir.join(format!("{process}.log"));
    let config = config.validated();
    if LOGGER.get().is_some() {
        configure(config);
        return LOGGER.get().map_or(path, |l| l.path.clone());
    }
    let env_override = std::env::var("APP_LOG").ok().filter(|s| !s.trim().is_empty());
    let base = parse_level(&config.level).unwrap_or(log::LevelFilter::Info);
    let filter = parse_filter(env_override.as_deref().unwrap_or(""), base);
    let sink = config.file.then(|| {
        let _ = std::fs::create_dir_all(dir);
        Sink { path: path.clone(), file: None, size: 0 }
    });
    let logger = Logger { filter: RwLock::new(filter), config: RwLock::new(config), sink: Mutex::new(sink), path: path.clone(), env_override };
    if LOGGER.set(logger).is_ok() {
        if let Some(l) = LOGGER.get() {
            if log::set_logger(l).is_ok() {
                log::set_max_level(log::LevelFilter::Trace);
            }
        }
    }
    path
}

/// Apply new settings (the environment override still wins).
pub fn configure(config: LogConfig) {
    let Some(l) = LOGGER.get() else { return };
    let config = config.validated();
    let base = parse_level(&config.level).unwrap_or(log::LevelFilter::Info);
    if let Ok(mut f) = l.filter.write() {
        *f = parse_filter(l.env_override.as_deref().unwrap_or(""), base);
    }
    if let Ok(mut s) = l.sink.lock() {
        match (config.file, s.is_some()) {
            (false, true) => *s = None,
            (true, false) => {
                if let Some(dir) = l.path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                *s = Some(Sink { path: l.path.clone(), file: None, size: 0 });
            }
            _ => {}
        }
    }
    if let Ok(mut c) = l.config.write() {
        *c = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn levels_and_filters() {
        assert_eq!(parse_level("WARN"), Some(log::LevelFilter::Warn));
        assert_eq!(parse_level("loud"), None);
        let (d, t) = parse_filter("warn, desktop-core=debug, desktop_core::stt=trace, bad=nope", log::LevelFilter::Info);
        assert_eq!(d, log::LevelFilter::Warn);
        assert_eq!(t[0], ("desktop_core::stt".into(), log::LevelFilter::Trace));
        assert_eq!(t.len(), 2);
        assert_eq!(LogConfig { level: "x".into(), max_mb: 0, ..LogConfig::default() }.validated(), LogConfig { max_mb: 1, ..LogConfig::default() });
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(UNIX_EPOCH + Duration::from_millis(1_795_412_412_345)), "2026-11-23T05:40:12.345Z");
        assert_eq!(rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn writes_json_lines_and_rotates() {
        let dir = std::env::temp_dir().join(format!("log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = init(&dir, "test", LogConfig { level: "debug".into(), max_mb: 1, keep: 2, file: true });
        let filler = "x".repeat(200_000);
        for i in 0..12 {
            log::info!(target: "desktop_core::test", "line {i} {filler}");
        }
        log::trace!(target: "desktop_core::test", "not written at debug");
        log::logger().flush();
        let text = std::fs::read_to_string(&file).unwrap();
        let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(last["level"], "info");
        assert!(last["msg"].as_str().unwrap().starts_with("line 11"));
        assert!(dir.join("test.1.log").exists() && dir.join("test.2.log").exists() && !dir.join("test.3.log").exists());
        assert!(!text.contains("not written"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
