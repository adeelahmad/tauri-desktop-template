//! Register a Python MCP server in Claude Desktop, with no terminal for the user.
//!
//! The app ships `uv` as a sidecar. `register_claude` copies it to a stable folder (so the
//! entry keeps working when the app moves), adds `mcpServers.<APP_MCP_NAME>` to
//! `claude_desktop_config.json` (backing up an existing file first) and runs the server's
//! package once so Python and its wheels are cached before the first chat.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tauri::Manager;

pub const SERVER: &str = env!("APP_MCP_NAME");
/// What `uv tool run --from` installs: a PyPI name or `name @ https://.../archive.zip`.
/// `APP_MCP_PACKAGE` at run time overrides it (handy for testing a branch).
pub fn package() -> String {
    std::env::var("APP_MCP_PACKAGE").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| env!("APP_MCP_PACKAGE").into())
}
/// A brand-new CPython often has no wheels yet for ONNX and friends; pin a known one.
pub const PYTHON: &str = "3.13";

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// Claude Desktop's config folder.
pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support").join("Claude")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming")).join("Claude")
    } else {
        home().join(".config").join("Claude")
    }
}

pub fn config_file() -> PathBuf {
    config_dir().join("claude_desktop_config.json")
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

/// The uv shipped with the app (Tauri puts sidecars next to the main binary), else one on PATH.
pub fn bundled_uv() -> Option<PathBuf> {
    let here = std::env::current_exe().ok()?.parent()?.join(exe("uv"));
    if here.is_file() {
        return Some(here);
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe("uv"))).find(|c| c.is_file()))
}

pub fn uv_version() -> Option<String> {
    let out = std::process::Command::new(bundled_uv()?).arg("--version").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn install_uv(data_dir: &Path) -> Result<PathBuf, String> {
    let src = bundled_uv().ok_or("This build does not include uv; reinstall the app from the release download.")?;
    let dst = data_dir.join("bin").join(exe("uv"));
    if dst == src {
        return Ok(dst);
    }
    std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = dst.with_extension("new");
    std::fs::copy(&src, &tmp).map_err(|e| format!("could not copy uv: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    Ok(dst)
}

pub fn server_args(extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["tool", "run", "--python", PYTHON, "--from"].iter().map(|s| s.to_string()).collect();
    args.push(package());
    args.push(SERVER.into());
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

pub fn entry(uv: &Path) -> Value {
    json!({"command": uv.to_string_lossy(), "args": server_args(&[])})
}

#[derive(Debug, Serialize)]
pub struct Registration {
    pub file: String,
    pub status: &'static str,
    pub backup: Option<String>,
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    secs.to_string()
}

/// Add or replace `mcpServers.<SERVER>` in `file`. A file that is not plain JSON is left
/// untouched (error); any other existing file is backed up first.
pub fn register(file: &Path, value: Value) -> Result<Registration, String> {
    let mut data: Map<String, Value> = match std::fs::read_to_string(file) {
        Ok(text) if text.trim().is_empty() => Map::new(),
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => {
                return Err(format!(
                    "{} is not plain JSON, so it was left untouched. Add this under \"mcpServers\" by hand:\n\"{SERVER}\": {}",
                    file.display(),
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                ))
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.to_string()),
    };
    let servers = data.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("\"mcpServers\" in {} is not an object; fix it by hand.", file.display()));
    };
    let status = match servers.get(SERVER) {
        Some(old) if *old == value => return Ok(Registration { file: file.display().to_string(), status: "unchanged", backup: None }),
        Some(_) => "replaced",
        None => "added",
    };
    servers.insert(SERVER.into(), value);
    let mut backup = None;
    if file.is_file() {
        let b = PathBuf::from(format!("{}.{SERVER}-backup-{}", file.display(), timestamp()));
        std::fs::copy(file, &b).map_err(|e| e.to_string())?;
        backup = Some(b.display().to_string());
    } else {
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, serde_json::to_string_pretty(&Value::Object(data)).map_err(|e| e.to_string())? + "\n").map_err(|e| e.to_string())?;
    Ok(Registration { file: file.display().to_string(), status, backup })
}

/// Register, then warm the package cache in the background (`warmup-done` event, true/false).
#[tauri::command(async)]
pub fn register_claude(app: tauri::AppHandle) -> Result<Registration, String> {
    use tauri::Emitter;
    let uv = install_uv(&app.path().app_data_dir().map_err(|e| e.to_string())?)?;
    let reg = register(&config_file(), entry(&uv))?;
    std::thread::spawn(move || {
        let ok = desktop_core::net::apply_env(&mut std::process::Command::new(&uv))
            .args(server_args(&["--help"]))
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success());
        let _ = app.emit("warmup-done", ok);
    });
    Ok(reg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_adds_replaces_and_backs_up() {
        let tmp = tempdir();
        let file = tmp.join("Claude").join("claude_desktop_config.json");
        let a = entry(Path::new("/x/uv"));
        let r = register(&file, a.clone()).unwrap();
        assert_eq!((r.status, r.backup.is_none()), ("added", true));
        assert_eq!(register(&file, a.clone()).unwrap().status, "unchanged");
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        v["mcpServers"]["other"] = json!({"command": "o"});
        v["theme"] = json!("dark");
        std::fs::write(&file, v.to_string()).unwrap();
        let r = register(&file, entry(Path::new("/y/uv"))).unwrap();
        assert_eq!(r.status, "replaced");
        assert!(Path::new(r.backup.as_ref().unwrap()).is_file());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!((v["mcpServers"]["other"]["command"].as_str(), v["theme"].as_str()), (Some("o"), Some("dark")));
        std::fs::write(&file, "{ // comment\n}").unwrap();
        assert!(register(&file, a).unwrap_err().contains("left untouched"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("claude-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }
}
