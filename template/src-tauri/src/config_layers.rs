//! Layered build-time config: how the files in `config/` become one JSON value.
//!
//! `build.rs` includes this file (`#[path = "src/config_layers.rs"]`) to do the merge at
//! build time; the app includes it too, so the same code is unit tested with
//! `cargo test`. It uses only `std` and `serde_json`.
//!
//! The layers, each one overriding the one before it (objects merge key by key; any other
//! value, arrays included, replaces the old one):
//!
//! ```text
//! config/settings.json                      shared by every build
//! config/environments/<env>.json            development | production | test (KITCHEN_ENV)
//! config/platforms/<platform>.json          mac | linux | windows (the target OS)
//! config/local.json                         this machine only (gitignored)
//!   -- the four above are merged by build.rs and compiled in --
//! APP_<SECTION>__<KEY>=value                at run time, e.g. APP_SEARCH__DEFAULT_TOP_K=5
//! ```
//!
//! A missing file is an empty layer. The result also gets two keys of its own, `env` and
//! `platform`, naming the layers that were used.
//!
//! Environment variables ([`apply_env`]) use `APP_` and `__` between levels. A key matches
//! the existing one ignoring case and `_` (`DEFAULT_TOP_K` is `defaultTopK`), and the value
//! is read as JSON when it parses (`5`, `true`, `[1,2]`), else as a string. Only variables
//! whose first level is already a top-level key apply, so the app's other `APP_*`
//! variables (`APP_PROFILE`, `APP_LOG`, ...) are left alone.
//!
//! `requiredInProduction` lists dot paths that must have a value in a production build
//! ([`missing_required`]); the app refuses to start without them.
//!
//! ```
//! use app_lib::config_layers;
//! use serde_json::json;
//!
//! let mut base = json!({"search": {"defaultTopK": 10, "mode": "vector"}, "tags": ["a"]});
//! config_layers::deep_merge(&mut base, json!({"search": {"defaultTopK": 5}, "tags": ["b"]}));
//! assert_eq!(base, json!({"search": {"defaultTopK": 5, "mode": "vector"}, "tags": ["b"]}));
//! ```
// Not every item is used by both build.rs and the app.
#![allow(dead_code)]

use serde_json::Value;
use std::path::Path;

/// Merge `over` into `base`: objects key by key (recursively), anything else replaces.
pub fn deep_merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => deep_merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

/// The environment for a build: `KITCHEN_ENV` (or `RUN_MODE`, its alias) when set and not
/// empty, else `production` for the release profile and `development` for any other.
pub fn env_name(kitchen_env: Option<&str>, cargo_profile: &str) -> String {
    match kitchen_env.map(str::trim) {
        Some(e) if !e.is_empty() => e.to_string(),
        _ if cargo_profile == "release" => "production".into(),
        _ => "development".into(),
    }
}

/// The prefix of run-time overrides.
pub const ENV_PREFIX: &str = "APP_";

fn squash(key: &str) -> String {
    key.chars().filter(|c| *c != '_').flat_map(char::to_lowercase).collect()
}

/// Apply `APP_A__B=value` overrides from `vars` (normally `std::env::vars()`) to `config`.
/// Returns the dot paths that were set.
pub fn apply_env(config: &mut Value, vars: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    let mut applied = Vec::new();
    for (name, raw) in vars {
        let Some(rest) = name.strip_prefix(ENV_PREFIX) else { continue };
        let parts: Vec<&str> = rest.split("__").collect();
        if parts.iter().any(|p| p.is_empty()) {
            continue;
        }
        let top = config.as_object().and_then(|o| o.keys().find(|k| squash(k) == squash(parts[0])).cloned());
        let Some(top) = top else { continue };
        let mut slot = &mut config[&top];
        let mut path = vec![top];
        for part in &parts[1..] {
            if !slot.is_object() {
                *slot = Value::Object(Default::default());
            }
            let key =
                slot.as_object().and_then(|o| o.keys().find(|k| squash(k) == squash(part)).cloned()).unwrap_or_else(|| part.to_lowercase());
            path.push(key.clone());
            slot = &mut slot[&key];
        }
        *slot = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
        applied.push(path.join("."));
    }
    applied
}

/// The `requiredInProduction` paths that have no value (missing, `null` or `""`).
pub fn missing_required(config: &Value) -> Vec<String> {
    let required = config.get("requiredInProduction").and_then(Value::as_array).cloned().unwrap_or_default();
    required
        .iter()
        .filter_map(Value::as_str)
        .filter(|path| {
            matches!(lookup(config, path), None | Some(Value::Null)) || lookup(config, path) == Some(&Value::String(String::new()))
        })
        .map(String::from)
        .collect()
}

/// The platform layer for a Rust `target_os`: `macos` is `mac`; others keep their name
/// (`linux`, `windows`, and `ios` / `android` if you add those files).
pub fn platform_name(target_os: &str) -> String {
    match target_os {
        "macos" => "mac".into(),
        other => other.into(),
    }
}

/// The files that make up `env` on `platform`, in merge order.
pub fn layer_paths(dir: &Path, env: &str, platform: &str) -> Vec<std::path::PathBuf> {
    vec![
        dir.join("settings.json"),
        dir.join("environments").join(format!("{env}.json")),
        dir.join("platforms").join(format!("{platform}.json")),
        dir.join("local.json"),
    ]
}

/// Read and merge the layers in `dir` (the app's `config/` folder) for `env` on `platform`.
/// Returns the merged value, or a message naming the file that is not a JSON object.
pub fn load(dir: &Path, env: &str, platform: &str) -> Result<Value, String> {
    let mut out = Value::Object(Default::default());
    for path in layer_paths(dir, env, platform) {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let layer: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if !layer.is_object() {
            return Err(format!("{}: the top level must be a JSON object", path.display()));
        }
        deep_merge(&mut out, layer);
    }
    out["env"] = Value::String(env.into());
    out["platform"] = Value::String(platform.into());
    Ok(out)
}

/// The value at a dot path (`"search.defaultTopK"`); an empty path is the root.
pub fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    path.split('.').try_fold(root, |v, key| match v {
        Value::Array(a) => a.get(key.parse::<usize>().ok()?),
        _ => v.get(key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The template's own `config/` folder (src-tauri/../config).
    fn template_config() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../config")
    }

    #[test]
    fn objects_merge_and_the_rest_replaces() {
        let mut v = json!({"a": {"b": 1, "c": [1, 2]}, "d": "x"});
        deep_merge(&mut v, json!({"a": {"c": [3], "e": null}, "d": {"f": true}}));
        assert_eq!(v, json!({"a": {"b": 1, "c": [3], "e": null}, "d": {"f": true}}));
    }

    #[test]
    fn env_follows_kitchen_env_then_the_profile() {
        assert_eq!(env_name(Some("test"), "debug"), "test");
        assert_eq!(env_name(Some(" "), "release"), "production");
        assert_eq!(env_name(None, "release"), "production");
        assert_eq!(env_name(None, "debug"), "development");
        assert_eq!(platform_name("macos"), "mac");
        assert_eq!(platform_name("windows"), "windows");
    }

    #[test]
    fn development_on_linux() {
        let c = load(&template_config(), "development", "linux").unwrap();
        assert_eq!(c["env"], "development");
        assert_eq!(c["platform"], "linux");
        assert_eq!(c["ui"]["showEnvBadge"], true);
        assert_eq!(c["ui"]["shortcutModifier"], "Ctrl");
        assert_eq!(c["search"]["defaultTopK"], 10);
        assert!(c["app"]["name"].is_string(), "settings.json is the base layer");
    }

    #[test]
    fn production_on_linux() {
        let c = load(&template_config(), "production", "linux").unwrap();
        assert_eq!(c["env"], "production");
        assert_eq!(c["ui"]["showEnvBadge"], false);
        assert_eq!(c["search"]["defaultTopK"], 10);
    }

    #[test]
    fn test_env_on_linux() {
        let c = load(&template_config(), "test", "linux").unwrap();
        assert_eq!(c["env"], "test");
        assert_eq!(c["search"]["defaultTopK"], 5);
        assert_eq!(c["ui"]["showEnvBadge"], true);
    }

    #[test]
    fn platform_overrides_environment() {
        let c = load(&template_config(), "production", "mac").unwrap();
        assert_eq!(c["platform"], "mac");
        assert_eq!(c["ui"]["shortcutModifier"], "Cmd");
    }

    #[test]
    fn missing_layers_are_empty_and_bad_ones_are_named() {
        let dir = std::env::temp_dir().join(format!("config-layers-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("environments")).unwrap();
        let c = load(&dir, "staging", "linux").unwrap();
        assert_eq!(c, json!({"env": "staging", "platform": "linux"}));
        std::fs::write(dir.join("environments/staging.json"), "[1]").unwrap();
        let e = load(&dir, "staging", "linux").unwrap_err();
        assert!(e.contains("staging.json") && e.contains("object"), "{e}");
        std::fs::write(dir.join("settings.json"), "{ nope").unwrap();
        assert!(load(&dir, "staging", "linux").unwrap_err().contains("settings.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_vars_override_existing_sections() {
        let mut c = json!({"search": {"defaultTopK": 10}, "mail": {"host": ""}});
        let vars = [
            ("APP_SEARCH__DEFAULT_TOP_K", "5"),
            ("APP_MAIL__HOST", "smtp.example.com"),
            ("APP_MAIL__TLS__ON", "true"),
            ("APP_PROFILE", "work"),
            ("APP_SEARCH__", "x"),
            ("OTHER", "1"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let applied = apply_env(&mut c, vars);
        assert_eq!(applied, ["search.defaultTopK", "mail.host", "mail.tls.on"]);
        assert_eq!(c, json!({"search": {"defaultTopK": 5}, "mail": {"host": "smtp.example.com", "tls": {"on": true}}}));
    }

    #[test]
    fn required_in_production() {
        let c = json!({"requiredInProduction": ["mail.host", "api.url", "api.key"], "mail": {"host": ""}, "api": {"url": "https://x", "key": null}});
        assert_eq!(missing_required(&c), ["mail.host", "api.key"]);
        assert!(missing_required(&json!({})).is_empty());
    }

    #[test]
    fn local_layer_comes_last() {
        let dir = std::env::temp_dir().join(format!("config-local-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("platforms")).unwrap();
        std::fs::write(dir.join("platforms/linux.json"), r#"{"a": 1, "b": 1}"#).unwrap();
        std::fs::write(dir.join("local.json"), r#"{"b": 2}"#).unwrap();
        assert_eq!(load(&dir, "development", "linux").unwrap()["b"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dot_paths() {
        let v = json!({"a": {"b": [10, {"c": "x"}]}});
        assert_eq!(lookup(&v, "a.b.0"), Some(&json!(10)));
        assert_eq!(lookup(&v, "a.b.1.c"), Some(&json!("x")));
        assert_eq!(lookup(&v, "a.nope"), None);
        assert_eq!(lookup(&v, ""), Some(&v));
    }
}
