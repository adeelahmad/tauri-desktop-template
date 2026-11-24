//! Background agent: start the worker process at login, without the window.
//!
//! - macOS: a launchd LaunchAgent, `~/Library/LaunchAgents/<id>.worker.plist`
//!   (RunAtLoad, KeepAlive: launchd restarts it if it stops), loaded with `launchctl bootstrap`.
//! - Windows: a per-user Run entry (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`),
//!   which needs no admin rights. (A Task Scheduler ONLOGON task is the alternative when
//!   you need restart-on-failure, but creating one usually needs elevation.)
//! - Linux: a systemd user unit, `~/.config/systemd/user/<id>-worker.service`.
//!
//! The agent runs `<this app> --worker`; the UI connects to that worker if it is up and
//! starts one itself otherwise.
//!
//! Commands: `agent_status` and `set_agent` (feature `background-agent`, which needs
//! `ipc-worker`).
//!
//! # Example
//!
//! What the UI gets back from `agent_status`:
//!
//! ```
//! let status = serde_json::json!({"installed": false, "kind": "systemd user unit", "location": "~/.config/systemd/user/com-example-app-worker.service"});
//! assert_eq!(status["installed"], false);
//! ```

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

const ID: &str = env!("APP_IDENTIFIER");

/// Whether the agent is installed, and where (for Settings).
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentStatus {
    /// The agent is set up to start the worker at login.
    pub installed: bool,
    /// What kind of agent this OS uses, such as `launchd LaunchAgent`.
    pub kind: &'static str,
    /// The plist, registry key or unit file.
    pub location: String,
}

#[cfg(not(windows))]
fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

fn exe() -> Result<String, String> {
    Ok(std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned())
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?}: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::*;

    /// Shown in Settings.
    pub const KIND: &str = "launchd LaunchAgent";

    fn label() -> String {
        format!("{ID}.worker")
    }

    /// The LaunchAgent plist.
    pub fn location() -> PathBuf {
        home().join("Library/LaunchAgents").join(format!("{}.plist", label()))
    }

    fn domain() -> Result<String, String> {
        let out = Command::new("id").arg("-u").output().map_err(|e| e.to_string())?;
        Ok(format!("gui/{}", String::from_utf8_lossy(&out.stdout).trim()))
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// Write the plist and load it.
    pub fn install() -> Result<(), String> {
        let log = home().join("Library/Logs").join(format!("{}.log", label()));
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>--worker</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
            label = label(),
            exe = escape(&exe()?),
            log = escape(&log.to_string_lossy()),
        );
        let file = location();
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&file, plist).map_err(|e| e.to_string())?;
        let _ = Command::new("launchctl").args(["bootout", &format!("{}/{}", domain()?, label())]).output();
        run(Command::new("launchctl").args(["bootstrap", &domain()?]).arg(&file))
    }

    /// Unload the agent and delete the plist.
    pub fn uninstall() -> Result<(), String> {
        let _ = Command::new("launchctl").args(["bootout", &format!("{}/{}", domain()?, label())]).output();
        match std::fs::remove_file(location()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    /// Whether the plist exists.
    pub fn installed() -> bool {
        location().is_file()
    }
}

#[cfg(windows)]
mod os {
    use super::*;

    /// Shown in Settings.
    pub const KIND: &str = "Windows Run entry";
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    /// `reg.exe` with no console window: the windowed app has none to share, so each
    /// call from Settings would flash one.
    fn reg() -> Command {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("reg");
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    }

    fn value() -> String {
        format!("{ID}.worker")
    }

    /// The registry value that holds the entry.
    pub fn location() -> PathBuf {
        PathBuf::from(format!(r"{KEY}\{}", value()))
    }

    /// Add the Run entry and start the worker now.
    pub fn install() -> Result<(), String> {
        let exe = exe()?;
        run(reg().args(["add", KEY, "/v", &value(), "/t", "REG_SZ", "/d", &format!("\"{exe}\" --worker"), "/f"]))?;
        // Start it now too, not only at the next login.
        Command::new(exe).arg("--worker").spawn().map(|_| ()).map_err(|e| e.to_string())
    }

    /// Remove the Run entry.
    pub fn uninstall() -> Result<(), String> {
        if installed() {
            run(reg().args(["delete", KEY, "/v", &value(), "/f"]))?;
        }
        Ok(())
    }

    /// Whether the Run entry exists.
    pub fn installed() -> bool {
        reg().args(["query", KEY, "/v", &value()]).output().is_ok_and(|o| o.status.success())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod os {
    use super::*;

    /// Shown in Settings.
    pub const KIND: &str = "systemd user unit";

    fn unit() -> String {
        format!("{}-worker.service", ID.replace('.', "-"))
    }

    /// The unit file.
    pub fn location() -> PathBuf {
        home().join(".config/systemd/user").join(unit())
    }

    /// Write the unit, then enable and start it.
    pub fn install() -> Result<(), String> {
        let text = format!(
            "[Unit]\nDescription={ID} background worker\n\n[Service]\nExecStart=\"{}\" --worker\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            exe()?
        );
        let file = location();
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run(Command::new("systemctl").args(["--user", "enable", "--now", &unit()]))
    }

    /// Disable the unit and delete it.
    pub fn uninstall() -> Result<(), String> {
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", &unit()]).output();
        match std::fs::remove_file(location()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    /// Whether the unit file exists.
    pub fn installed() -> bool {
        location().is_file()
    }
}

/// Whether the agent is installed.
pub fn installed() -> bool {
    os::installed()
}

/// The agent's state, for Settings.
pub fn status() -> AgentStatus {
    AgentStatus { installed: os::installed(), kind: os::KIND, location: os::location().to_string_lossy().into_owned() }
}

/// Tauri command: the agent's state.
#[tauri::command(async)]
pub fn agent_status() -> AgentStatus {
    status()
}

/// Tauri command: install or remove the agent, then return its state.
#[tauri::command(async)]
pub fn set_agent(enabled: bool) -> Result<AgentStatus, String> {
    if enabled {
        os::install()?;
    } else {
        os::uninstall()?;
    }
    Ok(status())
}
