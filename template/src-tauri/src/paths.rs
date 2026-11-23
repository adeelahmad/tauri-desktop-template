//! The folders Tauri's `app_config_dir` / `app_data_dir` resolve to, without a window
//! (for `--mcp`, `--worker` and the single-instance check that runs before the window).

use std::path::PathBuf;

const IDENTIFIER: &str = env!("APP_IDENTIFIER");

pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

/// (config folder, data folder).
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let env_dir = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
    let (config, data) = if cfg!(target_os = "macos") {
        let d = home().join("Library").join("Application Support");
        (d.clone(), d)
    } else if cfg!(windows) {
        let d = env_dir("APPDATA").unwrap_or_else(|| home().join("AppData").join("Roaming"));
        (d.clone(), d)
    } else {
        (env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")), env_dir("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share")))
    };
    (config.join(IDENTIFIER), data.join(IDENTIFIER))
}
