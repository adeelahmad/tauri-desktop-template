//! Build-time app config: the files in `config/`, merged for this build and compiled in.
//!
//! This is *your* configuration as the app's developer (support URL, defaults, per-OS
//! shortcuts, staging versus production endpoints). It is fixed when the app is built.
//! It is not the user's settings: those are `<config>/settings.json`, edited in the
//! Settings screen and read as `desktop_core::Config` at run time.
//!
//! `build.rs` merges `config/settings.json`, then `config/environments/<env>.json`, then
//! `config/platforms/<platform>.json` ([`crate::config_layers`]). The environment is
//! `KITCHEN_ENV` (`kitchen start` sets `development`, `kitchen test` sets `test`), else
//! `production` for release builds and `development` for debug builds. The platform is the
//! target OS: `mac`, `linux` or `windows`. Last comes `config/local.json` (gitignored, for
//! this machine). The merged value also says which: the keys `env` and `platform`. At run
//! time, `APP_<SECTION>__<KEY>=value` overrides a key (`APP_SEARCH__DEFAULT_TOP_K=5`).
//!
//! **Production safety.** Paths listed in `requiredInProduction` must have a value in a
//! production build, or the app refuses to start with an error naming them ([`check`]).
//! Development and test only warn.
//!
//! The UI gets the same value as `config` in `app_info` (and `ui-react/src/config.js`
//! reads it). It is compiled into the binary, so never put secrets in `config/`.
//!
//! # Example
//!
//! ```
//! use app_lib::app_config;
//!
//! let env: String = app_config::get("env").unwrap();
//! assert!(["development", "production", "test"].contains(&env.as_str()));
//! let top_k = app_config::get_or("search.defaultTopK", 10usize);
//! assert!(top_k > 0);
//! assert!(app_config::get::<u32>("no.such.key").is_none());
//! assert!(app_config::CONFIG.is_object());
//! ```

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::LazyLock;

/// The merged config of this build (written by `build.rs`), with this process's
/// `APP_<SECTION>__<KEY>` environment overrides applied.
pub static CONFIG: LazyLock<Value> = LazyLock::new(|| {
    let mut config: Value =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/app_config.json"))).expect("build.rs writes valid JSON");
    crate::config_layers::apply_env(&mut config, std::env::vars());
    config
});

/// Production safety: in a `production` build, an error naming every
/// `requiredInProduction` path that has no value (set it in `config/` or as
/// `APP_<SECTION>__<KEY>`). Other environments only log a warning and return `Ok`.
/// The window, the worker and `--mcp` call this at start and refuse to run on an error.
pub fn check() -> Result<(), String> {
    let missing = crate::config_layers::missing_required(&CONFIG);
    if missing.is_empty() {
        return Ok(());
    }
    let vars: Vec<String> = missing.iter().map(|p| format!("APP_{}", p.split('.').map(screaming).collect::<Vec<_>>().join("__"))).collect();
    let msg = format!("missing required config: {} (set it in config/ or as {})", missing.join(", "), vars.join(", "));
    if env() == "production" {
        Err(msg)
    } else {
        log::warn!("{msg}; allowed outside production");
        Ok(())
    }
}

/// `defaultTopK` as `DEFAULT_TOP_K`.
fn screaming(key: &str) -> String {
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_uppercase());
    }
    out
}

/// The value at a dot path (`"search.defaultTopK"`, `"list.0"`) as `T`; `None` when the
/// key is missing or has another type.
pub fn get<T: DeserializeOwned>(path: &str) -> Option<T> {
    crate::config_layers::lookup(&CONFIG, path).and_then(|v| T::deserialize(v).ok())
}

/// [`get`], or `default` when the key is missing or has another type.
pub fn get_or<T: DeserializeOwned>(path: &str, default: T) -> T {
    get(path).unwrap_or(default)
}

/// The environment this build was made for (`development`, `production`, `test`, ...).
pub fn env() -> &'static str {
    CONFIG["env"].as_str().unwrap_or("development")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_embeds_the_merged_layers() {
        // `cargo test` builds with KITCHEN_ENV unset (development) or `test` (kitchen test).
        assert!(matches!(env(), "development" | "test"), "{}", env());
        assert!(get::<String>("app.name").is_some());
        assert!(get::<String>("platform").is_some());
        assert_eq!(get::<bool>("ui.showEnvBadge"), Some(true));
        assert_eq!(get_or("search.missing", 7), 7);
        assert_eq!(get::<bool>("app.name"), None, "wrong type is None");
        assert!(check().is_ok(), "the samples require nothing");
    }

    #[test]
    fn env_var_names() {
        assert_eq!(screaming("defaultTopK"), "DEFAULT_TOP_K");
        assert_eq!(screaming("host"), "HOST");
    }
}
