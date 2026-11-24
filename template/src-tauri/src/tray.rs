//! The tray (menu-bar icon on macOS), global shortcuts and the app menu (feature
//! `tray-menu`, desktop only).
//!
//! All three run actions through one [`Registry`]: [`registry`] lists the app's actions
//! and [`AppHost`] carries them out. Settings (`config.tray`, `config.shortcuts`,
//! `config.menu`) are applied by [`apply`] at start and again whenever they are saved,
//! so the menus and shortcuts change at once. Problems (an unknown action, a bad or
//! clashing shortcut, a key another app holds) are logged and shown in Settings
//! (`tray_status`), never fatal.
//!
//! Built-in actions also send the `app-action` event to the window (`search` and
//! `settings` move the focus there); app actions send it too, then call the backend's
//! `action` method (see `service.rs`). Results arrive as `action-done`
//! `{action, ok, message}`.
//!
//! Add an action with `kitchen generate menu <id> --label "..."` or bind a default
//! shortcut with `kitchen generate shortcut <action> <keys>`; both write into [`own`].

use crate::error::CommandError;
use desktop_core::actions::{ActionError, Host, PlannedItem, Registry};
use desktop_core::Config;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const TRAY_ID: &str = "main";

/// The app's own actions, menu items and default shortcuts.
fn own(r: &mut Registry) -> Result<(), ActionError> {
    let _ = &r;
    // kitchen:actions (`kitchen generate menu` and `kitchen generate shortcut` add lines above this one)
    Ok(())
}

/// Every action the tray, shortcuts and menu may name: the built-ins and the app's own.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    if let Err(e) = own(&mut r) {
        log::warn!("app actions: {e}");
    }
    r
}

/// One global shortcut and whether it could be registered.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ShortcutState"))]
pub struct Bound {
    accelerator: String,
    action: String,
    error: Option<String>,
}

/// What Settings shows: the actions to pick from, problems, and the shortcuts' state.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "TrayStatus"))]
pub struct Status {
    actions: Vec<Value>,
    problems: Vec<String>,
    shortcuts: Vec<Bound>,
    paused: bool,
}

/// Managed state: the registry, and what the current settings turned on.
pub struct Tray {
    registry: Registry,
    paused: AtomicBool,
    close_to_tray: AtomicBool,
    left_click_opens: AtomicBool,
    /// Accelerators this module registered (others, e.g. from the UI, are left alone).
    registered: Mutex<Vec<String>>,
    status: Mutex<Status>,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            registry: registry(),
            paused: AtomicBool::new(false),
            close_to_tray: AtomicBool::new(false),
            left_click_opens: AtomicBool::new(true),
            registered: Mutex::new(Vec::new()),
            status: Mutex::new(Status::default()),
        }
    }
}

/// Run an action by id, from any entry point. Failures are logged and sent to the UI.
pub fn run(app: &AppHandle, action: &str) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    log::debug!("action {action}");
    if let Err(e) = tray.registry.dispatch(action, &AppHost(app.clone())) {
        log::warn!("{e}");
        done(app, action, Err(e.to_string()));
    }
}

/// Whether closing the window should hide it instead (the tray is on with `closeToTray`).
pub fn closes_to_tray(app: &AppHandle) -> bool {
    app.try_state::<Tray>().is_some_and(|t| t.close_to_tray.load(Ordering::Relaxed))
}

/// Build the tray, the app menu and the shortcuts from `cfg`, replacing what was there.
pub fn apply(app: &AppHandle, cfg: &Config) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    let plan = tray.registry.plan(&cfg.tray, &cfg.menu, &cfg.shortcuts);
    let mut problems = plan.problems.clone();
    let paused = tray.paused.load(Ordering::Relaxed);
    tray.close_to_tray.store(cfg.tray.enabled && cfg.tray.close_to_tray, Ordering::Relaxed);
    tray.left_click_opens.store(cfg.tray.left_click_opens, Ordering::Relaxed);
    if let Err(e) = build_tray(app, cfg, &plan.tray, &tray.registry, paused) {
        problems.push(format!("Tray: {e}"));
    }
    if let Err(e) = build_menu(app, &cfg.menu.title, &plan.menu, &tray.registry, paused) {
        problems.push(format!("Menu: {e}"));
    }
    #[cfg(target_os = "macos")]
    if let Err(e) = app.set_dock_visibility(!(cfg.tray.enabled && cfg.tray.show_in_menu_bar_only)) {
        problems.push(format!("Dock icon: {e}"));
    }
    let shortcuts = bind(app, &tray, &plan.shortcuts);
    for p in &problems {
        log::warn!("{p}");
    }
    let actions = tray.registry.list().into_iter().map(|(id, label)| json!({"id": id, "label": label})).collect();
    let status = Status { actions, problems, shortcuts, paused };
    let _ = app.emit("tray-status", &status);
    if let Ok(mut s) = tray.status.lock() {
        *s = status;
    };
}

/// The current problems and shortcut state.
#[tauri::command]
pub fn tray_status(app: AppHandle) -> Status {
    app.try_state::<Tray>().and_then(|t| t.status.lock().ok().map(|s| s.clone())).unwrap_or_default()
}

/// Run an action from the UI (the same path as the tray, the menu and the shortcuts).
#[tauri::command]
pub fn run_action(app: AppHandle, action: String) -> Result<(), CommandError> {
    let tray = app.try_state::<Tray>().ok_or("not ready")?;
    tray.registry.resolve(&action).map_err(|e| e.to_string())?;
    run(&app, &action);
    Ok(())
}

/// A label, with the pause item saying what it will do now.
fn label_of(registry: &Registry, action: &str, label: &str, paused: bool) -> String {
    if paused && action == "pause_resume" && label == registry.label(action) {
        "Resume background work".into()
    } else {
        label.to_string()
    }
}

/// The items of a menu, as Tauri menu items. Ids are `action#item` ([`desktop_core::actions::menu_id`]).
fn items(app: &AppHandle, plan: &[PlannedItem], registry: &Registry, paused: bool) -> tauri::Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    plan.iter()
        .map(|i| -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
            Ok(match i {
                PlannedItem::Separator => Box::new(PredefinedMenuItem::separator(app)?),
                PlannedItem::Item { menu_id, label, accelerator, enabled } => {
                    let action = desktop_core::actions::action_of(menu_id).unwrap_or_default();
                    let accel = accelerator.as_ref().map(|a| a.to_string());
                    Box::new(MenuItem::with_id(app, menu_id, label_of(registry, action, label, paused), *enabled, accel.as_deref())?)
                }
            })
        })
        .collect()
}

fn build_tray(app: &AppHandle, cfg: &Config, plan: &[PlannedItem], registry: &Registry, paused: bool) -> tauri::Result<()> {
    if !cfg.tray.enabled {
        app.remove_tray_by_id(TRAY_ID);
        return Ok(());
    }
    let boxed = items(app, plan, registry, paused)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = boxed.iter().map(|b| b.as_ref()).collect();
    let menu = Menu::with_items(app, &refs)?;
    let left_menu = !cfg.tray.left_click_opens;
    if let Some(t) = app.tray_by_id(TRAY_ID) {
        t.set_menu(Some(menu))?;
        t.set_show_menu_on_left_click(left_menu)?;
        return Ok(());
    }
    let mut b = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(app.package_info().name.clone())
        .menu(&menu)
        .show_menu_on_left_click(left_menu)
        // macOS draws a template image in the menu bar's own colours (light and dark).
        .icon_as_template(true)
        .on_tray_icon_event(|t, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let app = t.app_handle();
                if app.try_state::<Tray>().is_some_and(|s| s.left_click_opens.load(Ordering::Relaxed)) {
                    run(app, "show");
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

/// The platform's standard menus (on macOS: App, Edit, Window, so copy and paste keep
/// working) plus the app-defined menu when it has items. Windows and Linux get a menu bar
/// only when there is an app-defined menu.
fn build_menu(app: &AppHandle, title: &str, plan: &[PlannedItem], registry: &Registry, paused: bool) -> tauri::Result<()> {
    let mac = cfg!(target_os = "macos");
    if plan.is_empty() && !mac {
        app.remove_menu()?;
        return Ok(());
    }
    let menu = if mac { Menu::default(app)? } else { Menu::new(app)? };
    if !plan.is_empty() {
        let boxed = items(app, plan, registry, paused)?;
        let refs: Vec<&dyn IsMenuItem<Wry>> = boxed.iter().map(|b| b.as_ref()).collect();
        let sub = Submenu::with_items(app, title, true, &refs)?;
        // Before Window and Help on macOS, as apps usually place their own menus.
        let at = if mac { menu.items()?.len().saturating_sub(2) } else { 0 };
        menu.insert(&sub, at)?;
    }
    app.set_menu(menu)?;
    Ok(())
}

/// Replace this module's global shortcuts. Each failure (usually: another app has those
/// keys) is reported for that shortcut only.
fn bind(app: &AppHandle, tray: &Tray, plan: &[(desktop_core::actions::Accelerator, String)]) -> Vec<Bound> {
    let gs = app.global_shortcut();
    let mut registered = tray.registered.lock().unwrap_or_else(|e| e.into_inner());
    for old in registered.drain(..) {
        let _ = gs.unregister(old.as_str());
    }
    plan.iter()
        .map(|(accel, action)| {
            let keys = accel.to_string();
            let id = action.clone();
            let r = gs.on_shortcut(keys.as_str(), move |app, _, e| {
                if e.state == ShortcutState::Pressed {
                    run(app, &id);
                }
            });
            let error = match r {
                Ok(()) => {
                    registered.push(keys.clone());
                    None
                }
                Err(e) => {
                    log::warn!("shortcut {keys} for {action}: {e}");
                    Some(format!("{e} (another app may be using these keys)"))
                }
            };
            Bound { accelerator: keys, action: action.clone(), error }
        })
        .collect()
}

/// Send `action-done` to the window.
fn done(app: &AppHandle, action: &str, r: Result<String, String>) {
    let (ok, message) = match r {
        Ok(m) => (true, m),
        Err(m) => (false, m),
    };
    let _ = app.emit("action-done", json!({"action": action, "ok": ok, "message": message}));
}

/// Call the backend off the menu's thread and report the result.
fn in_background(app: &AppHandle, action: &'static str, work: impl FnOnce(&AppHandle) -> Result<String, String> + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || {
        let r = work(&app);
        if let Err(e) = &r {
            log::warn!("{action}: {e}");
        }
        done(&app, action, r);
    });
}

fn call(app: &AppHandle, method: &str, params: Value) -> Result<Value, String> {
    let state = app.try_state::<crate::AppState>().ok_or("not ready")?;
    Ok(state.service.call(method, params, &mut |_| {})?)
}

/// The text on the clipboard, through the system's own tool (no extra crates).
fn clipboard_text() -> Result<String, String> {
    let tries: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else if cfg!(windows) {
        &[("powershell", &["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard -Raw"])]
    } else {
        &[("wl-paste", &["--no-newline"]), ("xclip", &["-selection", "clipboard", "-o"]), ("xsel", &["--clipboard", "--output"])]
    };
    let mut last = String::from("no clipboard tool found (install wl-clipboard or xclip)");
    for (cmd, args) in tries {
        let mut c = std::process::Command::new(cmd);
        c.args(*args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // no console window
        }
        match c.output() {
            Ok(o) if o.status.success() => return Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
            Ok(o) => last = format!("{cmd}: {}", String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => last = format!("{cmd}: {e}"),
        }
    }
    Err(last)
}

/// The drop folder: the configured one, else `<Documents>/<App> Inbox` (the same place
/// folder watching uses), else `<data>/inbox`.
fn drop_folder(app: &AppHandle, cfg: &Config) -> std::path::PathBuf {
    if let Some(p) = &cfg.watch.drop_folder_path {
        return p.clone();
    }
    let docs = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|h| std::path::PathBuf::from(h).join("Documents"));
    match docs.filter(|d| d.is_dir()) {
        Some(d) => d.join(format!("{} Inbox", app.package_info().name)),
        None => crate::paths::app_dirs().1.join("inbox"),
    }
}

/// The real thing: windows, the backend and the UI.
pub struct AppHost(pub AppHandle);

impl AppHost {
    fn window(&self) -> Result<tauri::WebviewWindow, String> {
        self.0.get_webview_window("main").ok_or_else(|| "no main window".into())
    }

    /// Tell the window an action ran (it may move the focus).
    fn tell(&self, action: &str) {
        let _ = self.0.emit("app-action", json!({"action": action}));
    }
}

impl Host for AppHost {
    fn show_hide(&self) -> Result<(), String> {
        let w = self.window()?;
        let front = w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
        if front {
            self.hide()
        } else {
            self.show()
        }
    }

    fn show(&self) -> Result<(), String> {
        let w = self.window()?;
        let _ = w.unminimize();
        w.show().map_err(|e| e.to_string())?;
        w.set_focus().map_err(|e| e.to_string())
    }

    fn hide(&self) -> Result<(), String> {
        self.window()?.hide().map_err(|e| e.to_string())
    }

    fn focus_search(&self) -> Result<(), String> {
        self.show()?;
        self.tell("search");
        Ok(())
    }

    fn ingest_clipboard(&self) -> Result<(), String> {
        in_background(&self.0, "ingest_clipboard", |app| {
            let text = clipboard_text()?;
            if text.trim().is_empty() {
                return Err("The clipboard has no text.".into());
            }
            let source = format!("clipboard {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            let chunks = call(app, "ingest_text", json!({"source": source, "text": text}))?;
            Ok(format!("Added the clipboard text ({chunks} chunks)."))
        });
        Ok(())
    }

    fn ingest_drop_folder(&self) -> Result<(), String> {
        in_background(&self.0, "ingest_drop_folder", |app| {
            let folder = drop_folder(app, &crate::load_settings(app).config);
            if !folder.is_dir() {
                return Err(format!("The drop folder {} does not exist yet.", folder.display()));
            }
            let r = call(app, "ingest_folder", json!({"path": folder, "recursive": true}))?;
            Ok(format!("Imported the drop folder: {} files, {} chunks.", r["files"], r["chunks"]))
        });
        Ok(())
    }

    /// Pause: stop the running ingest (the rest resumes later) and folder watching, for
    /// this session. Resume: the saved settings again, and finish what was cut short.
    /// Saving settings while paused turns watching back on as configured.
    fn pause_resume(&self) -> Result<(), String> {
        let tray = self.0.try_state::<Tray>().ok_or("not ready")?;
        let paused = !tray.paused.fetch_xor(true, Ordering::Relaxed);
        let _ = self.0.emit("background-paused", paused);
        in_background(&self.0, "pause_resume", move |app| {
            let mut cfg = crate::load_settings(app).config;
            let r = if paused {
                call(app, "cancel_ingest", Value::Null)?;
                cfg.watch.enabled = false;
                call(app, "configure", json!({"config": cfg})).map(|_| "Background work paused.".to_string())
            } else {
                call(app, "configure", json!({"config": cfg}))?;
                call(app, "resume_interrupted", Value::Null).map(|_| "Background work resumed.".to_string())
            };
            apply(app, &cfg_saved(app));
            r
        });
        Ok(())
    }

    fn open_settings(&self) -> Result<(), String> {
        self.show()?;
        self.tell("settings");
        Ok(())
    }

    fn quit(&self) -> Result<(), String> {
        self.0.exit(0);
        Ok(())
    }

    fn custom(&self, id: &str) -> Result<(), String> {
        self.tell(id);
        let id = id.to_string();
        let app = self.0.clone();
        std::thread::spawn(move || {
            let r = call(&app, "action", json!({"id": id})).map(|v| if v.is_null() { String::new() } else { v.to_string() });
            done(&app, &id, r);
        });
        Ok(())
    }
}

fn cfg_saved(app: &AppHandle) -> Config {
    crate::load_settings(app).config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_own_actions_register() {
        let mut r = Registry::new();
        own(&mut r).expect("an action in `own` is invalid");
        let p = r.plan(&Default::default(), &Default::default(), &[]);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
    }
}
