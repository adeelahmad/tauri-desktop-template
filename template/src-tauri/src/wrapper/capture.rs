//! Key capture: while it is on and the site's window has focus, system keys go to the page
//! instead of the operating system, until the release shortcut (or the window losing focus).
//!
//! | OS | How keys are taken | How they reach the page |
//! |---|---|---|
//! | macOS | a session-level `CGEventTap` (needs Accessibility and Input Monitoring) | synthetic `keydown`/`keyup` events |
//! | Windows | a `WH_KEYBOARD_LL` hook on its own thread | synthetic `keydown`/`keyup` events |
//! | Linux | a GDK keyboard grab: `XGrabKeyboard` on X11, the keyboard-shortcuts-inhibit protocol on Wayland where the compositor has it | the web view's own key events |
//!
//! Not capturable: Ctrl+Alt+Del on Windows (the secure attention sequence never reaches
//! apps), the power button and Touch ID, and on Wayland whatever the compositor keeps for
//! itself when it does not support the inhibit protocol.
//!
//! The platform hooks call [`key`] for every key; it returns whether the key was taken.

use super::platform;
use desktop_core::keymap::{KeyEvent, ModTracker, Shortcut};
use desktop_core::wrapper::{page, LABEL};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use tauri::{AppHandle, Manager, Runtime};

/// The site's window as capture sees it, whatever the runtime (the state is a `static`, so
/// it cannot be generic).
trait Target: Send + Sync {
    /// Run a script in the site's window, if it is open.
    fn eval(&self, js: String);
    /// Start taking keys ([`platform::capture_start`]).
    fn start(&self) -> Result<(), String>;
    /// Let the keys go ([`platform::capture_stop`]).
    fn stop(&self);
}

struct App<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Target for App<R> {
    fn eval(&self, js: String) {
        if let Some(w) = self.0.get_webview_window(LABEL) {
            let _ = w.eval(js);
        }
    }

    fn start(&self) -> Result<(), String> {
        platform::capture_start(&self.0)
    }

    fn stop(&self) {
        platform::capture_stop(&self.0);
    }
}

/// The capture state the platform hooks read (they run outside Tauri's event loop).
struct State {
    app: Box<dyn Target>,
    enabled: AtomicBool,
    active: AtomicBool,
    keys: Mutex<Keys>,
}

struct Keys {
    release: Shortcut,
    mods: ModTracker,
    down: Vec<&'static str>,
}

static STATE: OnceLock<State> = OnceLock::new();

fn lock(s: &State) -> std::sync::MutexGuard<'_, Keys> {
    s.keys.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Set up capture (once, at start).
pub fn init<R: Runtime>(app: &AppHandle<R>, enabled: bool, release: Shortcut) {
    let _ = STATE.set(State {
        app: Box::new(App(app.clone())),
        enabled: AtomicBool::new(enabled),
        active: AtomicBool::new(false),
        keys: Mutex::new(Keys { release, mods: ModTracker::default(), down: Vec::new() }),
    });
}

/// New settings from Preferences.
pub fn configure(enabled: bool, release: Shortcut) {
    if let Some(s) = STATE.get() {
        s.enabled.store(enabled, Ordering::SeqCst);
        lock(s).release = release;
        if !enabled {
            stop();
        }
    }
}

/// Whether keys are being captured now.
pub fn is_active() -> bool {
    STATE.get().is_some_and(|s| s.active.load(Ordering::SeqCst))
}

/// Whether capture is switched on in Preferences.
pub fn is_enabled() -> bool {
    STATE.get().is_some_and(|s| s.enabled.load(Ordering::SeqCst))
}

/// The release shortcut, for menus and the overlay.
pub fn release_text() -> String {
    STATE.get().map(|s| lock(s).release.to_string()).unwrap_or_default()
}

fn eval(js: String) {
    if let Some(s) = STATE.get() {
        s.app.eval(js);
    }
}

/// Start capturing (the window must have focus). Returns why it could not.
pub fn start() -> Result<(), String> {
    let s = STATE.get().ok_or("key capture is not set up")?;
    if !s.enabled.load(Ordering::SeqCst) {
        return Err("key capture is switched off in Preferences".into());
    }
    if s.active.load(Ordering::SeqCst) {
        return Ok(());
    }
    s.app.start()?;
    s.active.store(true, Ordering::SeqCst);
    let release = lock(s).release.clone();
    eval(page::overlay_script(true, &format!("Keys captured. Press {release} to release.")));
    if platform::KEYS_ARRIVE_IN_PAGE {
        eval(page::watch_script(Some(&release)));
    }
    log::info!("key capture on");
    Ok(())
}

/// Stop capturing: the page gets `keyup` for every key still down, so nothing sticks.
pub fn stop() {
    let Some(s) = STATE.get() else { return };
    if !s.active.swap(false, Ordering::SeqCst) {
        return;
    }
    s.app.stop();
    let mut k = lock(s);
    if !platform::KEYS_ARRIVE_IN_PAGE {
        let held: Vec<&'static str> = std::mem::take(&mut k.down);
        for code in held.into_iter().rev() {
            let mods = k.mods.update(code, false);
            eval(KeyEvent { down: false, code, mods, repeat: false }.script());
        }
    }
    k.mods.clear();
    k.down.clear();
    drop(k);
    eval(page::overlay_script(false, ""));
    eval(page::watch_script(None));
    log::info!("key capture off");
}

/// Toggle (the menu item).
pub fn toggle() -> Result<(), String> {
    if is_active() {
        stop();
        Ok(())
    } else {
        start()
    }
}

/// A key from a platform hook. Returns `true` when it was taken (the OS must not see it).
/// (Linux has no hook: the keys reach the page themselves.)
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub fn key(code: &'static str, down: bool) -> bool {
    let Some(s) = STATE.get() else { return false };
    if !s.active.load(Ordering::SeqCst) {
        return false;
    }
    let mut k = lock(s);
    let repeat = down && k.down.contains(&code);
    if down && !repeat {
        k.down.push(code);
    } else if !down {
        k.down.retain(|c| *c != code);
    }
    let mods = k.mods.update(code, down);
    if down && !repeat && k.release.matches(code, mods) {
        k.down.retain(|c| *c != code);
        drop(k);
        // Not from inside the hook: stopping touches the hook itself.
        std::thread::spawn(stop);
        return true;
    }
    drop(k);
    eval(KeyEvent { down, code, mods, repeat }.script());
    true
}
