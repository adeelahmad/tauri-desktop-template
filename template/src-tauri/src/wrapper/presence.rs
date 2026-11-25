//! How the web app shows itself: the window's remembered size and place, the Dock icon,
//! the menu bar (tray) icon and starting at login. These are the user's preferences
//! (`dockIcon`, `menuBarIcon`, `runAtStartup`, `rememberWindow` in `wrapper.userEditable`;
//! the developer's defaults are in `config/settings.json`), and each one reuses a part of
//! the template that already exists:
//!
//! | Preference | Built on | Applies |
//! |---|---|---|
//! | `rememberWindow` | `tauri-plugin-window-state` (`tauri_window_state`), plus [`screen::fit`] when a display was removed | on the next launch |
//! | `menuBarIcon` | the tray of `tray_menu` (`crate::tray`), with the wrapper's own menu: Show/Hide, Release Keys, Mute, Preferences, Quit | at once |
//! | `dockIcon` (macOS) | the tray's "menu bar only" switch (`set_dock_visibility`) | at once |
//! | `runAtStartup` | `tauri-plugin-autostart` (`tauri_autostart`), started with [`AUTOSTART_ARG`] | at once (the login item is written or removed) |
//!
//! A preference whose part is not in this build is not offered ([`available`]) and is
//! refused by `wrapper_set`. `kitchen new --web` turns all four parts on.
//!
//! The Dock icon and the menu bar icon are never both off: `WrapperConfig::check` refuses
//! it, so Preferences cannot save it and a hand-edited file falls back to the developer's
//! settings. Started at login with the Dock icon off, the app stays hidden in the menu
//! bar until the user picks Show.
//!
//! # Example
//!
//! ```
//! use desktop_core::wrapper::screen::{fit, Rect};
//!
//! // The window was left on a display that is gone: it comes back onto the one that is left.
//! let main = Rect { x: 0, y: 0, width: 1440, height: 900 };
//! let saved = Rect { x: -1200, y: 50, width: 800, height: 600 };
//! assert_eq!(fit(saved, &[main]).map(|r| r.x), Some(0));
//! ```

use desktop_core::wrapper::{screen, Editable, WrapperConfig};
#[cfg(feature = "tray-menu")]
use std::sync::OnceLock;
use tauri::{AppHandle, Runtime, WebviewWindow};

/// The argument the login item starts the app with, so it knows it was not opened by hand.
pub const AUTOSTART_ARG: &str = "--autostart";

/// The menu bar icon is in this build (`tray_menu`).
const HAS_TRAY: bool = {% if tray_menu %}cfg!(feature = "tray-menu"){% else %}false{% endif %};
/// Starting at login is in this build (`tauri_autostart`).
const HAS_AUTOSTART: bool = {% if tauri_plugins and tauri_autostart %}true{% else %}false{% endif %};
/// Remembering the window is in this build (`tauri_window_state`).
const HAS_WINDOW_STATE: bool = {% if tauri_plugins and tauri_window_state %}true{% else %}false{% endif %};

{% if tauri_plugins and tauri_window_state %}/// What the window-state plugin remembers for the wrapper's windows: size, position,
/// maximized and full screen (not visibility or decorations: the wrapper decides those).
pub const WINDOW_FLAGS: tauri_plugin_window_state::StateFlags = tauri_plugin_window_state::StateFlags::SIZE
    .union(tauri_plugin_window_state::StateFlags::POSITION)
    .union(tauri_plugin_window_state::StateFlags::MAXIMIZED)
    .union(tauri_plugin_window_state::StateFlags::FULLSCREEN);
{% endif %}
/// Whether `field` can be offered in this build, on this OS. The other editable fields
/// always can.
pub fn available(field: Editable) -> bool {
    match field {
        Editable::DockIcon => HAS_TRAY && cfg!(target_os = "macos"),
        Editable::MenuBarIcon => HAS_TRAY,
        Editable::RunAtStartup => HAS_AUTOSTART,
        Editable::RememberWindow => HAS_WINDOW_STATE,
        _ => true,
    }
}

/// Whether Preferences shows `field`: [`available`] in this build, and the capture options
/// (`startCaptured`, `releaseShortcut`) only while key capture is on.
pub fn offered(field: Editable, cfg: &WrapperConfig) -> bool {
    available(field) && (cfg.capture_keys || !matches!(field, Editable::StartCaptured | Editable::ReleaseShortcut))
}

/// When a change to `field` takes effect: `"now"` or `"nextLaunch"` (for Preferences).
pub fn applies(field: Editable) -> &'static str {
    match field {
        Editable::RememberWindow => "nextLaunch",
        _ => "now",
    }
}

/// The Dock icon is off in effect (macOS, with the menu bar icon there to reach the app).
pub fn dock_hidden(cfg: &WrapperConfig) -> bool {
    available(Editable::DockIcon) && !cfg.dock_icon && cfg.menu_bar_icon
}

/// Whether this launch opens with the window hidden: started at login while the Dock icon
/// is off (the menu bar icon shows it).
pub fn starts_hidden(cfg: &WrapperConfig, args: &[String]) -> bool {
    dock_hidden(cfg) && cfg.run_at_startup && args.iter().any(|a| a == AUTOSTART_ARG)
}

/// The tray settings for the wrapper's menu bar icon: on with `menuBarIcon`; Show/Hide,
/// Release Keys (only with key capture), Mute, Preferences and Quit; no Dock icon and
/// "close hides" with the Dock icon off.
#[cfg(feature = "tray-menu")]
pub fn tray_settings(cfg: &WrapperConfig) -> desktop_core::actions::TrayConfig {
    use desktop_core::actions::{MenuEntry, TrayConfig, SEPARATOR};
    let mut ids = vec!["show_hide"];
    if cfg.capture_keys {
        ids.push(super::A_RELEASE);
    }
    ids.extend([super::A_MUTE, SEPARATOR, super::A_PREFS, "quit"]);
    let items = ids
        .iter()
        .enumerate()
        .map(|(i, a)| if *a == SEPARATOR { MenuEntry { id: format!("sep{i}"), ..MenuEntry::new(a) } } else { MenuEntry::new(a) })
        .collect();
    let menu_bar_only = dock_hidden(cfg);
    TrayConfig { enabled: HAS_TRAY && cfg.menu_bar_icon, items, show_in_menu_bar_only: menu_bar_only, close_to_tray: menu_bar_only, left_click_opens: false }
}

/// Rebuilds the tray from the saved settings (set by `lib.rs`, which knows the runtime).
#[cfg(feature = "tray-menu")]
static TRAY_REFRESH: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Called when `menuBarIcon` or `dockIcon` change: `refresh` rebuilds the tray.
#[cfg(feature = "tray-menu")]
pub fn on_tray_change(refresh: impl Fn() + Send + Sync + 'static) {
    let _ = TRAY_REFRESH.set(Box::new(refresh));
}

/// Make the login item match `want` (write it or remove it). Errors are returned for
/// Preferences and logged at start.
pub fn set_run_at_startup<R: Runtime>(app: &AppHandle<R>, want: bool) -> Result<(), String> {
{% if tauri_plugins and tauri_autostart %}    use tauri_plugin_autostart::ManagerExt;
    let m = app.autostart();
    if m.is_enabled().unwrap_or(!want) == want {
        return Ok(());
    }
    let r = if want { m.enable() } else { m.disable() };
    r.map_err(|e| format!("run at startup: {e}"))
{% else %}    let _ = (app, want);
    Ok(())
{% endif %}}

/// At start, before the window opens: the login item as the settings say.
pub fn setup<R: Runtime>(app: &AppHandle<R>, cfg: &WrapperConfig) {
    if HAS_AUTOSTART {
        if let Err(e) = set_run_at_startup(app, cfg.run_at_startup) {
            log::warn!("{e}");
        }
    }
}

/// Apply what changed in Preferences, where the OS allows it now.
pub fn changed<R: Runtime>(app: &AppHandle<R>, old: &WrapperConfig, new: &WrapperConfig) -> Result<(), String> {
    #[cfg(feature = "tray-menu")]
    if old.dock_icon != new.dock_icon || old.menu_bar_icon != new.menu_bar_icon || old.capture_keys != new.capture_keys {
        if let Some(refresh) = TRAY_REFRESH.get() {
            refresh();
        }
    }
    if old.run_at_startup != new.run_at_startup {
        set_run_at_startup(app, new.run_at_startup)?;
    }
    let _ = (app, old, new);
    Ok(())
}

/// The site's window was built hidden: restore its size and place (when remembered), keep
/// it on screen, then show it unless this launch starts hidden.
pub fn restore_and_show<R: Runtime>(w: &WebviewWindow<R>, cfg: &WrapperConfig, hidden: bool) -> tauri::Result<()> {
{% if tauri_plugins and tauri_window_state %}    use tauri_plugin_window_state::{StateFlags, WindowExt};
    if cfg.remember_window {
        if let Err(e) = w.restore_state(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED) {
            log::warn!("could not restore the window: {e}");
        }
        keep_on_screen(w);
    }
{% else %}    let _ = cfg;
{% endif %}    if hidden {
        log::info!("started at login: the window stays hidden until Show");
        return Ok(());
    }
    w.show()?;
    // Full screen needs a visible window.
{% if tauri_plugins and tauri_window_state %}    if cfg.remember_window {
        let _ = w.restore_state(tauri_plugin_window_state::StateFlags::FULLSCREEN);
    }
{% endif %}    w.set_focus()
}

/// Write the window's size and place to disk now, when the user keeps `rememberWindow`
/// on. The window-state plugin saves only on `RunEvent::Exit`, which a quit from the menu
/// bar or the Chromium engine can skip, so the wrapper also saves when the window loses
/// focus, is closed, and shortly after it stops moving or resizing ([`save_soon`]).
pub fn save_window<R: Runtime>(app: &AppHandle<R>, cfg: &WrapperConfig) {
{% if tauri_plugins and tauri_window_state %}    use tauri_plugin_window_state::AppHandleExt;
    if !cfg.remember_window {
        return;
    }
    match app.save_window_state(WINDOW_FLAGS) {
        Ok(()) => log::debug!("window size and place saved"),
        Err(e) => log::warn!("could not save the window: {e}"),
    }
{% else %}    let _ = (app, cfg);
{% endif %}}

/// [`save_window`] once the window has been still for [`SAVE_DELAY`]: a drag or a resize
/// sends many events, and only the last one is written.
pub fn save_soon<R: Runtime>(app: &AppHandle<R>, save: impl FnOnce(&AppHandle<R>) + Send + 'static) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    let mine = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SAVE_DELAY);
        if GENERATION.load(Ordering::SeqCst) == mine {
            save(&app);
        }
    });
}

/// How long the window must be still before [`save_soon`] writes it.
pub const SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(800);

/// Move (and shrink) the window onto a display when it is off screen, e.g. its display was
/// removed since it was saved ([`screen::fit`], on the displays' work areas).
{% unless tauri_plugins and tauri_window_state %}#[allow(dead_code)] // used with tauri_window_state
{% endunless %}fn keep_on_screen<R: Runtime>(w: &WebviewWindow<R>) {
    if w.is_maximized().unwrap_or(false) || w.is_fullscreen().unwrap_or(false) {
        return;
    }
    let (Ok(pos), Ok(outer), Ok(inner), Ok(monitors)) = (w.outer_position(), w.outer_size(), w.inner_size(), w.available_monitors()) else {
        return;
    };
    let displays: Vec<screen::Rect> = monitors
        .iter()
        .map(|m| {
            let a = m.work_area();
            screen::Rect { x: a.position.x, y: a.position.y, width: a.size.width, height: a.size.height }
        })
        .collect();
    let window = screen::Rect { x: pos.x, y: pos.y, width: outer.width, height: outer.height };
    let Some(to) = screen::fit(window, &displays) else { return };
    log::info!("the window was off screen; moved back");
    if (to.width, to.height) != (outer.width, outer.height) {
        let frame = (outer.width.saturating_sub(inner.width), outer.height.saturating_sub(inner.height));
        let _ = w.set_size(tauri::PhysicalSize::new(to.width.saturating_sub(frame.0).max(1), to.height.saturating_sub(frame.1).max(1)));
    }
    let _ = w.set_position(tauri::PhysicalPosition::new(to.x, to.y));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WrapperConfig {
        WrapperConfig { url: "https://h.example/".into(), ..WrapperConfig::default() }
    }

    #[test]
    fn what_is_offered_matches_the_build() {
        assert!(available(Editable::Zoom));
        assert_eq!(available(Editable::MenuBarIcon), HAS_TRAY);
        assert_eq!(available(Editable::DockIcon), HAS_TRAY && cfg!(target_os = "macos"));
        assert_eq!(available(Editable::RunAtStartup), HAS_AUTOSTART);
        assert_eq!(available(Editable::RememberWindow), HAS_WINDOW_STATE);
        assert_eq!(applies(Editable::RememberWindow), "nextLaunch");
        assert_eq!(applies(Editable::DockIcon), "now");
    }

    #[test]
    fn capture_options_only_with_capture() {
        let off = cfg();
        assert!(!offered(Editable::StartCaptured, &off) && !offered(Editable::ReleaseShortcut, &off));
        assert!(offered(Editable::Zoom, &off));
        let on = WrapperConfig { capture_keys: true, ..cfg() };
        assert!(offered(Editable::StartCaptured, &on) && offered(Editable::ReleaseShortcut, &on));
    }

    #[test]
    fn hidden_only_when_started_at_login_without_a_dock_icon() {
        let login = vec!["app".to_string(), AUTOSTART_ARG.to_string()];
        let by_hand = vec!["app".to_string()];
        let menu_bar_only = WrapperConfig { dock_icon: false, menu_bar_icon: true, run_at_startup: true, ..cfg() };
        assert_eq!(starts_hidden(&menu_bar_only, &login), available(Editable::DockIcon));
        assert!(!starts_hidden(&menu_bar_only, &by_hand));
        assert!(!starts_hidden(&WrapperConfig { run_at_startup: true, ..cfg() }, &login), "the Dock icon is on");
        // Never hidden without a menu bar icon to bring it back, whatever the file says.
        assert!(!starts_hidden(&WrapperConfig { dock_icon: false, run_at_startup: true, ..cfg() }, &login));
    }

    #[cfg(feature = "tray-menu")]
    #[test]
    fn the_tray_menu_is_the_wrappers() {
        let t = tray_settings(&cfg());
        assert!(!t.enabled && !t.show_in_menu_bar_only && !t.close_to_tray);
        let ids: Vec<_> = t.items.iter().map(|i| i.action.as_str()).collect();
        assert_eq!(ids, ["show_hide", super::super::A_MUTE, "separator", super::super::A_PREFS, "quit"]);
        let t = tray_settings(&WrapperConfig { capture_keys: true, menu_bar_icon: true, dock_icon: false, ..cfg() });
        assert!(t.enabled);
        assert_eq!(t.show_in_menu_bar_only, cfg!(target_os = "macos"));
        assert_eq!(t.items[1].action, super::super::A_RELEASE);
        let r = crate::tray::registry();
        let plan = r.plan(&t, &Default::default(), &[]);
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    }
}
