//! The app's folders, the same in every process (window, `--mcp`, `--worker`), and the
//! profiles that split them.
//!
//! A profile is a separate set of settings, documents, index, keys and logs (for example
//! "Work" and "Personal"). The `default` profile uses the app folders themselves, so an app
//! that never creates a second profile looks exactly as before. Others live in
//! `<config>/profiles/<id>/` and `<data>/profiles/<id>/`. `profiles.json` in the config
//! folder lists them and names the active one; `APP_PROFILE` picks one for a single run
//! (the window passes it to the worker, and the MCP entry carries it).
//!
//! # Example
//!
//! `profiles.json`, as [`Profiles`] reads and writes it:
//!
//! ```
//! let text = r#"{"active": "work", "profiles": [{"id": "default", "name": "Default"}, {"id": "work", "name": "Work"}]}"#;
//! let v: serde_json::Value = serde_json::from_str(text)?;
//! assert_eq!(v["profiles"][1]["id"], v["active"]);
//! // Profile "work" keeps its files in <config>/profiles/work and <data>/profiles/work.
//! # Ok::<(), serde_json::Error>(())
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

const IDENTIFIER: &str = env!("APP_IDENTIFIER");

/// The user's home folder (`HOME`, or `USERPROFILE` on Windows).
pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

/// (config folder, data folder) of the profile this process runs.
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let (config, data) = base_dirs();
    match profile() {
        DEFAULT => (config, data),
        id => (config.join("profiles").join(id), data.join("profiles").join(id)),
    }
}

/// iOS and Android: the app's private folder, from Tauri's path resolver at start.
#[cfg(mobile)]
static MOBILE_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// iOS and Android: set the app's private folder (once, before anything reads a path).
/// There is no home folder to build paths from there, and only this folder is writable.
#[cfg(mobile)]
pub fn set_mobile_root(dir: PathBuf) {
    let _ = MOBILE_ROOT.set(dir);
}

/// The app's own folders, before any profile.
pub fn base_dirs() -> (PathBuf, PathBuf) {
    #[cfg(test)]
    if let Some(root) = tests::ROOT.with(|r| r.borrow().clone()) {
        return (root.join("config"), root.join("data"));
    }
    #[cfg(mobile)]
    if let Some(root) = MOBILE_ROOT.get() {
        return (root.join("config"), root.join("data"));
    }
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

/// The id of the profile that uses the app folders themselves.
pub const DEFAULT: &str = "default";

/// One profile: a folder id and the name the user sees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Folder name (see [`valid_id`]).
    pub id: String,
    /// The name shown in the profile menu.
    pub name: String,
}

/// The contents of `profiles.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profiles {
    /// The profile the next start opens.
    pub active: String,
    /// Every profile, the default first.
    pub profiles: Vec<Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self { active: DEFAULT.into(), profiles: vec![Profile { id: DEFAULT.into(), name: "Default".into() }] }
    }
}

/// Profile ids are folder names: lower-case letters, digits and `-`, at most 32.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn registry_file() -> PathBuf {
    base_dirs().0.join("profiles.json")
}

/// The saved profile list (the default profile is always there).
pub fn profiles() -> Profiles {
    let mut p: Profiles = std::fs::read(registry_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    p.profiles.retain(|x| valid_id(&x.id));
    if !p.profiles.iter().any(|x| x.id == DEFAULT) {
        p.profiles.insert(0, Profiles::default().profiles.remove(0));
    }
    if !p.profiles.iter().any(|x| x.id == p.active) {
        p.active = DEFAULT.into();
    }
    p
}

fn save(p: &Profiles) -> Result<(), String> {
    let file = registry_file();
    std::fs::create_dir_all(file.parent().ok_or("no config folder")?).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

/// The profile this process runs: `APP_PROFILE` if it names a known profile, else the
/// active one. Fixed for the life of the process.
pub fn profile() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let saved = profiles();
        std::env::var("APP_PROFILE")
            .ok()
            .filter(|id| saved.profiles.iter().any(|p| &p.id == id))
            .unwrap_or(saved.active)
    })
}

/// Add a profile named `name` and return it (its id comes from the name).
pub fn create(name: &str) -> Result<Profile, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err("give the profile a name of 1 to 60 characters".into());
    }
    let mut p = profiles();
    let base: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let base = base.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let base = if base.is_empty() { "profile".to_string() } else { base.chars().take(28).collect() };
    let mut id = base.clone();
    let mut n = 2;
    while p.profiles.iter().any(|x| x.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let profile = Profile { id, name: name.to_string() };
    p.profiles.push(profile.clone());
    save(&p)?;
    Ok(profile)
}

/// Make `id` the profile the next start opens.
pub fn set_active(id: &str) -> Result<(), String> {
    let mut p = profiles();
    if !p.profiles.iter().any(|x| x.id == id) {
        return Err(format!("no profile {id}"));
    }
    p.active = id.to_string();
    save(&p)
}

/// Rename a profile (its folders keep their id).
pub fn rename(id: &str, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err("give the profile a name of 1 to 60 characters".into());
    }
    let mut p = profiles();
    let entry = p.profiles.iter_mut().find(|x| x.id == id).ok_or_else(|| format!("no profile {id}"))?;
    entry.name = name.to_string();
    save(&p)
}

/// Remove a profile from the list. Its folders stay on disk unless `delete_data`; the
/// default profile, the running one and the active one cannot be removed.
pub fn remove(id: &str, delete_data: bool) -> Result<(), String> {
    let mut p = profiles();
    if id == DEFAULT || id == profile() || id == p.active {
        return Err("switch to another profile first; the default profile always stays".into());
    }
    if !p.profiles.iter().any(|x| x.id == id) {
        return Err(format!("no profile {id}"));
    }
    p.profiles.retain(|x| x.id != id);
    save(&p)?;
    if delete_data && valid_id(id) {
        let (config, data) = base_dirs();
        for dir in [config.join("profiles").join(id), data.join("profiles").join(id)] {
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Folders for this test thread instead of the real app folders.
        pub(super) static ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    }

    #[test]
    fn profile_ids_and_registry() {
        let tmp = std::env::temp_dir().join(format!("profiles-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        ROOT.with(|r| *r.borrow_mut() = Some(tmp.clone()));
        assert!(valid_id("work-2") && !valid_id("Work") && !valid_id("../x") && !valid_id(""));
        assert_eq!(profiles(), Profiles::default());
        let work = create("Work & Clients").unwrap();
        assert_eq!(work.id, "work-clients");
        assert_eq!(create("work clients").unwrap().id, "work-clients-2");
        assert!(create("  ").is_err());
        set_active(&work.id).unwrap();
        assert_eq!(profiles().active, "work-clients");
        rename(&work.id, "Work").unwrap();
        assert!(remove("default", false).is_err());
        assert!(remove(&work.id, false).is_err(), "the active profile stays");
        let (config, data) = base_dirs();
        let other = data.join("profiles").join("work-clients-2");
        std::fs::create_dir_all(&other).unwrap();
        remove("work-clients-2", true).unwrap();
        assert!(!other.exists());
        assert_eq!(profiles().profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Default", "Work"]);
        assert!(config.join("profiles.json").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
