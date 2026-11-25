//! The app shell (feature `app-shell`): themes without a flash, a custom titlebar and
//! Settings in a window of its own.
//!
//! - **Themes and fonts.** [`plugin`] gives every web view a script that runs before the
//!   page loads and sets the saved theme, light/dark/system mode and fonts on `<html>`
//!   (see `desktop_core::appearance`), so the first paint already has the right colours.
//!   When the settings are saved, [`applied`] sends the `appearance` event and every open
//!   window switches at once; [`get_appearance`] gives the saved values to a page that
//!   (re)loads later.
//! - **Custom titlebar.** `tauri.conf.json` turns the native decorations off on Windows and
//!   Linux, where the UI draws a drag region and minimise / maximise / close buttons;
//!   `tauri.macos.conf.json` keeps the macOS traffic lights over the content
//!   (`titleBarStyle: Overlay`) and the UI leaves room for them. `capabilities/shell.json`
//!   allows exactly those window calls for the main window.
//! - **Settings window.** [`open_settings`] opens (or brings to the front) a second window,
//!   `settings`, showing `settings.html`. The UI opens it with Cmd/Ctrl+, or its gear button,
//!   and on macOS the app menu gets a "Settings…" item ([`add_menu_item`]). That window has
//!   its own capability (`capabilities/settings.json`) with events only: no plugins, no
//!   window control, no file access.
//!
//! The tray / app menu feature (`tray-menu`), when present, owns the menu bar and replaces
//! it when its settings are applied; its `settings` action reaches the UI as an
//! `app-action` event, and the UI answers by calling `open_settings`, so both work together.
//!
//! # Example
//!
//! [`plugin`] registers the script `desktop_core::appearance` builds from the saved settings:
//!
//! ```
//! use desktop_core::appearance::AppearanceConfig;
//!
//! let saved = AppearanceConfig { theme: "nord".into(), mode: "dark".into(), ..Default::default() };
//! let js = saved.init_script("windows", 0);
//! assert!(js.contains(r#""theme":"nord""#));
//! assert!(js.contains(r#""mode":"dark""#));
//! ```

// iOS and Android use the themes only: no titlebar, menus or second window there.
#![cfg_attr(mobile, allow(dead_code))]

use desktop_core::appearance::AppearanceConfig;
use tauri::{AppHandle, Emitter, Manager};

/// The label of the Settings window.
pub const SETTINGS_WINDOW: &str = "settings";

/// The id of the macOS "Settings…" menu item.
pub const SETTINGS_MENU_ID: &str = "app-shell-settings";

/// The page the Settings window shows (next to `index.html` in the frontend).
pub const SETTINGS_PAGE: &str = "settings.html";

/// The Tauri plugin that applies `appearance` before each page loads.
pub fn plugin(appearance: &AppearanceConfig) -> tauri::plugin::TauriPlugin<tauri::DynRuntime> {
    tauri::plugin::Builder::new("app-shell").js_init_script(appearance.init_script(std::env::consts::OS, 0)).build()
}

/// Tell every window about new appearance settings (after `set_config`).
pub fn applied(app: &AppHandle, appearance: &AppearanceConfig) {
    if let Err(e) = app.emit("appearance", appearance.resolved()) {
        log::warn!("could not send the new appearance to the windows: {e}");
    }
}

/// The saved appearance, resolved for the UI (the same JSON as the `appearance` event).
/// The UI applies it once at start: on iOS and Android, and after a development reload,
/// the script from [`plugin`] may be older than the settings.
#[tauri::command]
pub fn get_appearance(app: AppHandle) -> serde_json::Value {
    crate::load_settings(&app).config.appearance.resolved()
}

/// Open the Settings window, or bring it to the front when it is open.
#[cfg(desktop)]
#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), crate::error::CommandError> {
    Ok(show_settings(&app).map_err(|e| e.to_string())?)
}

/// What [`open_settings`] does, for Rust callers (menus, tray actions).
#[cfg(desktop)]
pub fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(SETTINGS_WINDOW) {
        w.unminimize()?;
        w.show()?;
        return w.set_focus();
    }
    // The newest saved appearance: this window may open long after the app started.
    let appearance = crate::load_settings(app).config.appearance;
    let title = format!("{} Settings", app.package_info().name);
    tauri::WebviewWindowBuilder::new(app, SETTINGS_WINDOW, tauri::WebviewUrl::App(SETTINGS_PAGE.into()))
        .title(title)
        .inner_size(760.0, 680.0)
        .min_inner_size(480.0, 400.0)
        .initialization_script(appearance.init_script(std::env::consts::OS, 1))
{%- if web_wrapper %}
        // Web apps turn `freezePrototype` off for the site's sake; keep it for this page.
        .initialization_script("Object.freeze(Object.prototype);")
{%- endif %}
        .build()?
        .set_focus()
}

/// Opens Settings for the menu item [`add_menu_item`] adds (other menu events pass by).
#[cfg(desktop)]
pub fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    if event.id() == SETTINGS_MENU_ID {
        if let Err(e) = show_settings(app) {
            log::warn!("could not open the Settings window: {e}");
        }
    }
}

/// macOS: add "Settings…" (Cmd+,) to the application menu, after "About". Other systems
/// have no app menu; there the titlebar's gear button and Ctrl+, open Settings.
#[cfg(desktop)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn add_menu_item(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
    let menu = match app.menu() {
        Some(menu) => menu,
        None => Menu::default(app)?,
    };
    if menu.get(SETTINGS_MENU_ID).is_some() {
        return Ok(());
    }
    if let Some(MenuItemKind::Submenu(first)) = menu.items()?.into_iter().next() {
        let item = MenuItem::with_id(app, SETTINGS_MENU_ID, "Settings…", true, Some("CmdOrCtrl+,"))?;
        let separator = PredefinedMenuItem::separator(app)?;
        let items: [&dyn IsMenuItem<tauri::DynRuntime>; 2] = [&separator, &item];
        // After "About" (the first item), as macOS apps place it.
        first.insert_items(&items, first.items()?.len().min(1))?;
    }
    app.set_menu(menu)?;
    Ok(())
}

/// Closing the main window closes Settings too, so the app quits as before.
#[cfg(desktop)]
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
        if let Some(w) = window.app_handle().get_webview_window(SETTINGS_WINDOW) {
            let _ = w.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_init_script_has_the_saved_theme() {
        let a = AppearanceConfig { theme: "nord".into(), mode: "dark".into(), ..Default::default() };
        let js = a.init_script(std::env::consts::OS, 0);
        assert!(js.contains(r#""theme":"nord""#) && js.contains(r#""mode":"dark""#));
    }

    #[test]
    fn the_settings_page_is_one_of_the_apps_own() {
        let url = tauri::Url::parse("tauri://localhost/").unwrap().join(SETTINGS_PAGE).unwrap();
        assert!(crate::guard::allowed(&url));
    }
}
