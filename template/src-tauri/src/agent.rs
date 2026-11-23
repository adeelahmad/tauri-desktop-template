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

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

const ID: &str = env!("APP_IDENTIFIER");

#[derive(Serialize)]
pub struct AgentStatus {
    pub installed: bool,
    pub kind: &'static str,
    pub location: String,
}

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

    pub const KIND: &str = "launchd LaunchAgent";

    fn label() -> String {
        format!("{ID}.worker")
    }

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

    pub fn uninstall() -> Result<(), String> {
        let _ = Command::new("launchctl").args(["bootout", &format!("{}/{}", domain()?, label())]).output();
        match std::fs::remove_file(location()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    pub fn installed() -> bool {
        location().is_file()
    }
}

#[cfg(windows)]
mod os {
    use super::*;

    pub const KIND: &str = "Windows Run entry";
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    fn value() -> String {
        format!("{ID}.worker")
    }

    pub fn location() -> PathBuf {
        PathBuf::from(format!(r"{KEY}\{}", value()))
    }

    pub fn install() -> Result<(), String> {
        let exe = exe()?;
        run(Command::new("reg").args(["add", KEY, "/v", &value(), "/t", "REG_SZ", "/d", &format!("\"{exe}\" --worker"), "/f"]))?;
        // Start it now too, not only at the next login.
        Command::new(exe).arg("--worker").spawn().map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn uninstall() -> Result<(), String> {
        if installed() {
            run(Command::new("reg").args(["delete", KEY, "/v", &value(), "/f"]))?;
        }
        Ok(())
    }

    pub fn installed() -> bool {
        Command::new("reg").args(["query", KEY, "/v", &value()]).output().is_ok_and(|o| o.status.success())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod os {
    use super::*;

    pub const KIND: &str = "systemd user unit";

    fn unit() -> String {
        format!("{}-worker.service", ID.replace('.', "-"))
    }

    pub fn location() -> PathBuf {
        home().join(".config/systemd/user").join(unit())
    }

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

    pub fn uninstall() -> Result<(), String> {
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", &unit()]).output();
        match std::fs::remove_file(location()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    pub fn installed() -> bool {
        location().is_file()
    }
}

pub fn installed() -> bool {
    os::installed()
}

pub fn status() -> AgentStatus {
    AgentStatus { installed: os::installed(), kind: os::KIND, location: os::location().to_string_lossy().into_owned() }
}

#[tauri::command(async)]
pub fn agent_status() -> AgentStatus {
    status()
}

#[tauri::command(async)]
pub fn set_agent(enabled: bool) -> Result<AgentStatus, String> {
    if enabled {
        os::install()?;
    } else {
        os::uninstall()?;
    }
    Ok(status())
}
