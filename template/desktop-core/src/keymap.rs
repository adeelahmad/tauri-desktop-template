//! Keys for the web app wrapper's key capture (feature `web-wrapper`): platform key codes
//! turned into DOM `KeyboardEvent` values, the release shortcut, and modifier tracking.
//!
//! Captured keys never reach the operating system: the wrapper takes them from a macOS
//! event tap or a Windows low-level hook, maps them here to the DOM's `code` (the
//! physical key, layout independent: `KeyA`, `MetaLeft`, `AudioVolumeUp`), `key` and the
//! legacy `keyCode`, and hands them to the page as `keydown`/`keyup` events. Remote KVM
//! pages such as PiKVM read `code`, so a Mac's Cmd+Tab reaches the remote machine as
//! `MetaLeft` + `Tab`.
//!
//! Pure logic, no platform code: every table is unit tested on every OS.
//!
//! # Example
//!
//! ```
//! use desktop_core::keymap::{mac_code, win_code, Mods, Shortcut};
//!
//! assert_eq!(mac_code(0x30), Some("Tab"));
//! assert_eq!(mac_code(0x37), Some("MetaLeft")); // Cmd
//! assert_eq!(win_code(0x5b, 0x5b, true), Some("MetaLeft")); // the Windows key
//!
//! let release: Shortcut = "ctrl+alt+shift+k".parse().unwrap();
//! assert_eq!(release.to_string(), "Ctrl+Alt+Shift+K");
//! let mods = Mods { ctrl: true, alt: true, shift: true, meta: false };
//! assert!(release.matches("KeyK", mods));
//! assert!(!release.matches("KeyK", Mods { shift: false, ..mods }));
//! ```

use std::fmt;
use std::str::FromStr;

/// The default release shortcut: four keys a remote session almost never needs at once.
pub const DEFAULT_RELEASE: &str = "Ctrl+Alt+Shift+K";

/// Modifier keys held during an event (left and right count the same).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    /// Control.
    pub ctrl: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Cmd / Windows / Super.
    pub meta: bool,
}

impl Mods {
    /// The bit mask the page script reads: 1 ctrl, 2 alt, 4 shift, 8 meta.
    pub fn bits(self) -> u8 {
        u8::from(self.ctrl) | u8::from(self.alt) << 1 | u8::from(self.shift) << 2 | u8::from(self.meta) << 3
    }
}

/// One key event for the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    /// `keydown` (true) or `keyup`.
    pub down: bool,
    /// The DOM `code`, e.g. `KeyA`.
    pub code: &'static str,
    /// Modifiers held, this key included once it is down.
    pub mods: Mods,
    /// Held down long enough to repeat.
    pub repeat: bool,
}

impl KeyEvent {
    /// The JavaScript that dispatches this event in the page (through the wrapper's page
    /// script, `window.__kitchenWrap`).
    ///
    /// ```
    /// use desktop_core::keymap::{KeyEvent, Mods};
    /// let e = KeyEvent { down: true, code: "KeyA", mods: Mods { shift: true, ..Mods::default() }, repeat: false };
    /// assert_eq!(e.script(), r#"window.__kitchenWrap&&window.__kitchenWrap.key("keydown","A","KeyA",65,4,false)"#);
    /// ```
    pub fn script(&self) -> String {
        let key = dom_key(self.code, self.mods.shift);
        let key = serde_json::to_string(&key).unwrap_or_else(|_| "\"\"".into());
        format!(
            r#"window.__kitchenWrap&&window.__kitchenWrap.key("{}",{key},"{}",{},{},{})"#,
            if self.down { "keydown" } else { "keyup" },
            self.code,
            key_code(self.code),
            self.mods.bits(),
            self.repeat
        )
    }
}

/// Which modifier a DOM code is, if any.
pub fn modifier_of(code: &str) -> Option<fn(&mut Mods) -> &mut bool> {
    match code {
        "ControlLeft" | "ControlRight" => Some(|m| &mut m.ctrl),
        "AltLeft" | "AltRight" => Some(|m| &mut m.alt),
        "ShiftLeft" | "ShiftRight" => Some(|m| &mut m.shift),
        "MetaLeft" | "MetaRight" => Some(|m| &mut m.meta),
        _ => None,
    }
}

/// Tracks which modifiers are down from the key events themselves (each side on its own,
/// so releasing one Shift while the other is held keeps `shift`).
#[derive(Debug, Default, Clone)]
pub struct ModTracker {
    held: Vec<&'static str>,
}

impl ModTracker {
    /// Record `code` going down or up; returns the modifiers now held.
    pub fn update(&mut self, code: &'static str, down: bool) -> Mods {
        if modifier_of(code).is_some() {
            self.held.retain(|c| *c != code);
            if down {
                self.held.push(code);
            }
        }
        self.mods()
    }

    /// The modifiers held now.
    pub fn mods(&self) -> Mods {
        let mut m = Mods::default();
        for c in &self.held {
            if let Some(f) = modifier_of(c) {
                *f(&mut m) = true;
            }
        }
        m
    }

    /// Forget everything (capture released).
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// The modifiers still down, as codes (to send their `keyup` on release).
    pub fn held(&self) -> &[&'static str] {
        &self.held
    }
}

/// A release shortcut: modifiers plus one key, by DOM code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// Modifiers that must be held (exactly these).
    pub mods: Mods,
    /// The key, as a DOM code (`KeyK`, `F12`, `Pause`).
    pub code: String,
}

impl Shortcut {
    /// Whether `code` pressed with `mods` is this shortcut.
    pub fn matches(&self, code: &str, mods: Mods) -> bool {
        self.code == code && self.mods == mods
    }
}

impl Default for Shortcut {
    fn default() -> Self {
        DEFAULT_RELEASE.parse().expect("the default shortcut parses")
    }
}

impl FromStr for Shortcut {
    type Err = String;

    /// `Ctrl+Alt+Shift+K`, case-insensitive; `Cmd`, `Win`, `Super` and `Meta` are the same
    /// key, as are `Alt` and `Option`. Without a modifier only F13–F24, `Pause` and
    /// `ScrollLock` are accepted (keys a remote session rarely needs).
    fn from_str(s: &str) -> Result<Self, String> {
        let mut mods = Mods::default();
        let mut code = None;
        for part in s.split('+').map(str::trim) {
            let flag = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut mods.ctrl,
                "alt" | "option" | "opt" => &mut mods.alt,
                "shift" => &mut mods.shift,
                "cmd" | "command" | "meta" | "super" | "win" | "windows" => &mut mods.meta,
                _ => {
                    if code.is_some() {
                        return Err(format!("{s:?}: only one key besides the modifiers"));
                    }
                    code = Some(code_from_name(part).ok_or_else(|| format!("{s:?}: {part:?} is not a key"))?);
                    continue;
                }
            };
            *flag = true;
        }
        let code = code.ok_or_else(|| format!("{s:?}: name a key, e.g. Ctrl+Alt+Shift+K"))?;
        let lone_ok =
            code == "Pause" || code == "ScrollLock" || code.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| n >= 13);
        if mods == Mods::default() && !lone_ok {
            return Err(format!("{s:?}: add a modifier (Ctrl, Alt, Shift or Cmd), or use F13–F24, Pause or ScrollLock"));
        }
        if modifier_of(&code).is_some() {
            return Err(format!("{s:?}: the key itself cannot be a modifier"));
        }
        Ok(Self { mods, code })
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.mods.ctrl, "Ctrl"), (self.mods.alt, "Alt"), (self.mods.shift, "Shift"), (self.mods.meta, "Cmd")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        let c = self.code.as_str();
        let name = c.strip_prefix("Key").or_else(|| c.strip_prefix("Digit")).unwrap_or(c);
        f.write_str(name)
    }
}

/// The DOM code for a key name: `K`, `5`, `F12`, `Space`, `Esc`, `Left`, or a DOM code
/// itself (`KeyK`). Also reads GDK key names (`space`, `Page_Up`), case-insensitive.
pub fn code_from_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.len() == 1 {
        let c = n.chars().next()?.to_ascii_uppercase();
        return match c {
            'A'..='Z' => Some(format!("Key{c}")),
            '0'..='9' => Some(format!("Digit{c}")),
            _ => None,
        };
    }
    let lower = n.to_ascii_lowercase().replace('_', "");
    if let Some(num) = lower.strip_prefix('f').and_then(|d| d.parse::<u8>().ok()) {
        return (1..=24).contains(&num).then(|| format!("F{num}"));
    }
    let named = match lower.as_str() {
        "space" => "Space",
        "enter" | "return" => "Enter",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" | "prior" => "PageUp",
        "pagedown" | "next" => "PageDown",
        "up" | "arrowup" => "ArrowUp",
        "down" | "arrowdown" => "ArrowDown",
        "left" | "arrowleft" => "ArrowLeft",
        "right" | "arrowright" => "ArrowRight",
        "pause" | "break" => "Pause",
        "scrolllock" | "scrolllck" => "ScrollLock",
        "printscreen" | "print" => "PrintScreen",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    // A DOM code given as is.
    ALL_CODES.iter().find(|c| c.eq_ignore_ascii_case(n)).map(|c| c.to_string())
}

/// Every code the tables below produce.
#[rustfmt::skip]
const ALL_CODES: &[&str] = &[
    "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL", "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV", "KeyW", "KeyX",
    "KeyY", "KeyZ", "Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9", "Minus", "Equal", "BracketLeft", "BracketRight", "Backslash", "Semicolon",
    "Quote", "Backquote", "Comma", "Period", "Slash", "IntlBackslash", "Enter", "Tab", "Space", "Backspace", "Escape", "CapsLock", "ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight",
    "AltLeft", "AltRight", "MetaLeft", "MetaRight", "ContextMenu", "Fn", "Insert", "Delete", "Home", "End", "PageUp", "PageDown", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "NumLock",
    "Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4", "Numpad5", "Numpad6", "Numpad7", "Numpad8", "Numpad9", "NumpadDecimal", "NumpadMultiply", "NumpadAdd", "NumpadSubtract",
    "NumpadDivide", "NumpadEnter", "NumpadEqual", "PrintScreen", "ScrollLock", "Pause", "AudioVolumeMute", "AudioVolumeUp", "AudioVolumeDown", "MediaPlayPause", "MediaTrackNext",
    "MediaTrackPrevious", "MediaStop", "IntlYen", "IntlRo", "Lang1", "Lang2", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17", "F18",
    "F19", "F20", "F21", "F22", "F23", "F24",
];

/// macOS virtual key code (`kVK_*`, Carbon `Events.h`) to DOM code.
pub fn mac_code(keycode: u16) -> Option<&'static str> {
    const LETTERS: [(u16, &str); 26] = [
        (0x00, "KeyA"),
        (0x0B, "KeyB"),
        (0x08, "KeyC"),
        (0x02, "KeyD"),
        (0x0E, "KeyE"),
        (0x03, "KeyF"),
        (0x05, "KeyG"),
        (0x04, "KeyH"),
        (0x22, "KeyI"),
        (0x26, "KeyJ"),
        (0x28, "KeyK"),
        (0x25, "KeyL"),
        (0x2E, "KeyM"),
        (0x2D, "KeyN"),
        (0x1F, "KeyO"),
        (0x23, "KeyP"),
        (0x0C, "KeyQ"),
        (0x0F, "KeyR"),
        (0x01, "KeyS"),
        (0x11, "KeyT"),
        (0x20, "KeyU"),
        (0x09, "KeyV"),
        (0x0D, "KeyW"),
        (0x07, "KeyX"),
        (0x10, "KeyY"),
        (0x06, "KeyZ"),
    ];
    if let Some((_, c)) = LETTERS.iter().find(|(k, _)| *k == keycode) {
        return Some(c);
    }
    Some(match keycode {
        0x12 => "Digit1",
        0x13 => "Digit2",
        0x14 => "Digit3",
        0x15 => "Digit4",
        0x17 => "Digit5",
        0x16 => "Digit6",
        0x1A => "Digit7",
        0x1C => "Digit8",
        0x19 => "Digit9",
        0x1D => "Digit0",
        0x1B => "Minus",
        0x18 => "Equal",
        0x21 => "BracketLeft",
        0x1E => "BracketRight",
        0x2A => "Backslash",
        0x29 => "Semicolon",
        0x27 => "Quote",
        0x32 => "Backquote",
        0x2B => "Comma",
        0x2F => "Period",
        0x2C => "Slash",
        0x0A => "IntlBackslash",
        0x24 => "Enter",
        0x30 => "Tab",
        0x31 => "Space",
        0x33 => "Backspace",
        0x35 => "Escape",
        0x39 => "CapsLock",
        0x38 => "ShiftLeft",
        0x3C => "ShiftRight",
        0x3B => "ControlLeft",
        0x3E => "ControlRight",
        0x3A => "AltLeft",
        0x3D => "AltRight",
        0x37 => "MetaLeft",
        0x36 => "MetaRight",
        0x3F => "Fn",
        0x6E => "ContextMenu",
        0x72 => "Insert",
        0x75 => "Delete",
        0x73 => "Home",
        0x77 => "End",
        0x74 => "PageUp",
        0x79 => "PageDown",
        0x7E => "ArrowUp",
        0x7D => "ArrowDown",
        0x7B => "ArrowLeft",
        0x7C => "ArrowRight",
        0x47 => "NumLock",
        0x52 => "Numpad0",
        0x53 => "Numpad1",
        0x54 => "Numpad2",
        0x55 => "Numpad3",
        0x56 => "Numpad4",
        0x57 => "Numpad5",
        0x58 => "Numpad6",
        0x59 => "Numpad7",
        0x5B => "Numpad8",
        0x5C => "Numpad9",
        0x41 => "NumpadDecimal",
        0x43 => "NumpadMultiply",
        0x45 => "NumpadAdd",
        0x4E => "NumpadSubtract",
        0x4B => "NumpadDivide",
        0x4C => "NumpadEnter",
        0x51 => "NumpadEqual",
        0x4A => "AudioVolumeMute",
        0x48 => "AudioVolumeUp",
        0x49 => "AudioVolumeDown",
        0x5D => "IntlYen",
        0x5E => "IntlRo",
        0x68 => "Lang1",
        0x66 => "Lang2",
        0x7A => "F1",
        0x78 => "F2",
        0x63 => "F3",
        0x76 => "F4",
        0x60 => "F5",
        0x61 => "F6",
        0x62 => "F7",
        0x64 => "F8",
        0x65 => "F9",
        0x6D => "F10",
        0x67 => "F11",
        0x6F => "F12",
        0x69 => "F13",
        0x6B => "F14",
        0x71 => "F15",
        0x6A => "F16",
        0x40 => "F17",
        0x4F => "F18",
        0x50 => "F19",
        0x5A => "F20",
        _ => return None,
    })
}

/// macOS media keys: the `NX_KEYTYPE_*` number in a system-defined event (subtype 8).
pub fn mac_media_code(nx_keytype: u32) -> Option<&'static str> {
    Some(match nx_keytype {
        0 => "AudioVolumeUp",
        1 => "AudioVolumeDown",
        7 => "AudioVolumeMute",
        16 => "MediaPlayPause",
        17 | 19 => "MediaTrackNext",
        18 | 20 => "MediaTrackPrevious",
        _ => return None,
    })
}

/// The modifier a macOS `flagsChanged` key code is, with the device-dependent flag bit
/// (`NX_DEVICE*KEYMASK`) that says whether that side is down now.
pub fn mac_modifier_bit(keycode: u16) -> Option<u64> {
    Some(match keycode {
        0x3B => 0x0000_0001, // left control
        0x3E => 0x0000_2000, // right control
        0x38 => 0x0000_0002, // left shift
        0x3C => 0x0000_0004, // right shift
        0x37 => 0x0000_0008, // left command
        0x36 => 0x0000_0010, // right command
        0x3A => 0x0000_0020, // left option
        0x3D => 0x0000_0040, // right option
        0x39 => 0x0001_0000, // caps lock (the lock state, not a side)
        0x3F => 0x0080_0000, // fn
        _ => return None,
    })
}

/// Windows key to DOM code, from a low-level hook's virtual key, scan code (set 1) and
/// extended flag. Media keys go by virtual key; everything else by scan code, which, like
/// the DOM code, names the physical key whatever the layout.
pub fn win_code(vk: u32, scan: u32, extended: bool) -> Option<&'static str> {
    let media = match vk {
        0xAD => Some("AudioVolumeMute"),
        0xAE => Some("AudioVolumeDown"),
        0xAF => Some("AudioVolumeUp"),
        0xB0 => Some("MediaTrackNext"),
        0xB1 => Some("MediaTrackPrevious"),
        0xB2 => Some("MediaStop"),
        0xB3 => Some("MediaPlayPause"),
        0x13 => Some("Pause"),
        _ => None,
    };
    if media.is_some() {
        return media;
    }
    const ROW: [&str; 0x59] = [
        "",
        "Escape",
        "Digit1",
        "Digit2",
        "Digit3",
        "Digit4",
        "Digit5",
        "Digit6",
        "Digit7",
        "Digit8",
        "Digit9",
        "Digit0",
        "Minus",
        "Equal",
        "Backspace",
        "Tab",
        "KeyQ",
        "KeyW",
        "KeyE",
        "KeyR",
        "KeyT",
        "KeyY",
        "KeyU",
        "KeyI",
        "KeyO",
        "KeyP",
        "BracketLeft",
        "BracketRight",
        "Enter",
        "ControlLeft",
        "KeyA",
        "KeyS",
        "KeyD",
        "KeyF",
        "KeyG",
        "KeyH",
        "KeyJ",
        "KeyK",
        "KeyL",
        "Semicolon",
        "Quote",
        "Backquote",
        "ShiftLeft",
        "Backslash",
        "KeyZ",
        "KeyX",
        "KeyC",
        "KeyV",
        "KeyB",
        "KeyN",
        "KeyM",
        "Comma",
        "Period",
        "Slash",
        "ShiftRight",
        "NumpadMultiply",
        "AltLeft",
        "Space",
        "CapsLock",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "NumLock",
        "ScrollLock",
        "Numpad7",
        "Numpad8",
        "Numpad9",
        "NumpadSubtract",
        "Numpad4",
        "Numpad5",
        "Numpad6",
        "NumpadAdd",
        "Numpad1",
        "Numpad2",
        "Numpad3",
        "Numpad0",
        "NumpadDecimal",
        "",
        "",
        "IntlBackslash",
        "F11",
        "F12",
    ];
    if extended {
        return Some(match scan {
            0x1C => "NumpadEnter",
            0x1D => "ControlRight",
            0x35 => "NumpadDivide",
            0x37 => "PrintScreen",
            0x38 => "AltRight",
            0x47 => "Home",
            0x48 => "ArrowUp",
            0x49 => "PageUp",
            0x4B => "ArrowLeft",
            0x4D => "ArrowRight",
            0x4F => "End",
            0x50 => "ArrowDown",
            0x51 => "PageDown",
            0x52 => "Insert",
            0x53 => "Delete",
            0x5B => "MetaLeft",
            0x5C => "MetaRight",
            0x5D => "ContextMenu",
            _ => return None,
        });
    }
    match scan {
        0x64..=0x6E => Some(["F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23"][(scan - 0x64) as usize]),
        0x76 => Some("F24"),
        _ => ROW.get(scan as usize).copied().filter(|c| !c.is_empty()),
    }
}

/// The DOM `key` for a code on a US layout (the page mostly reads `code`; `key` is a
/// best effort for pages that read it).
pub fn dom_key(code: &str, shift: bool) -> String {
    if let Some(l) = code.strip_prefix("Key") {
        return if shift { l.to_string() } else { l.to_ascii_lowercase() };
    }
    if let Some(d) = code.strip_prefix("Digit") {
        let shifted = [")", "!", "@", "#", "$", "%", "^", "&", "*", "("];
        let i: usize = d.parse().unwrap_or(0);
        return if shift { shifted[i].to_string() } else { d.to_string() };
    }
    if let Some(n) = code.strip_prefix("Numpad").and_then(|d| d.parse::<u8>().ok()) {
        return n.to_string();
    }
    let pair = |a: &str, b: &str| if shift { b.to_string() } else { a.to_string() };
    match code {
        "Minus" => pair("-", "_"),
        "Equal" => pair("=", "+"),
        "BracketLeft" => pair("[", "{"),
        "BracketRight" => pair("]", "}"),
        "Backslash" | "IntlBackslash" => pair("\\", "|"),
        "Semicolon" => pair(";", ":"),
        "Quote" => pair("'", "\""),
        "Backquote" => pair("`", "~"),
        "Comma" => pair(",", "<"),
        "Period" => pair(".", ">"),
        "Slash" => pair("/", "?"),
        "Space" => " ".into(),
        "NumpadEnter" => "Enter".into(),
        "NumpadDecimal" => ".".into(),
        "NumpadMultiply" => "*".into(),
        "NumpadAdd" => "+".into(),
        "NumpadSubtract" => "-".into(),
        "NumpadDivide" => "/".into(),
        "ControlLeft" | "ControlRight" => "Control".into(),
        "AltLeft" | "AltRight" => "Alt".into(),
        "ShiftLeft" | "ShiftRight" => "Shift".into(),
        "MetaLeft" | "MetaRight" => "Meta".into(),
        other => other.to_string(),
    }
}

/// The legacy `keyCode` (Windows virtual key numbers, which browsers use).
pub fn key_code(code: &str) -> u32 {
    if let Some(l) = code.strip_prefix("Key") {
        return l.bytes().next().map_or(0, u32::from);
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return 48 + d.parse::<u32>().unwrap_or(0);
    }
    if let Some(n) = code.strip_prefix("Numpad").and_then(|d| d.parse::<u32>().ok()) {
        return 96 + n;
    }
    if let Some(n) = code.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()) {
        return 111 + n;
    }
    match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" | "NumpadEnter" => 13,
        "ShiftLeft" | "ShiftRight" => 16,
        "ControlLeft" | "ControlRight" => 17,
        "AltLeft" | "AltRight" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "PrintScreen" => 44,
        "Insert" => 45,
        "Delete" => 46,
        "MetaLeft" => 91,
        "MetaRight" => 92,
        "ContextMenu" => 93,
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "NumLock" => 144,
        "ScrollLock" => 145,
        "AudioVolumeMute" => 173,
        "AudioVolumeDown" => 174,
        "AudioVolumeUp" => 175,
        "MediaTrackNext" => 176,
        "MediaTrackPrevious" => 177,
        "MediaStop" => 178,
        "MediaPlayPause" => 179,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        "IntlBackslash" => 226,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_keys_map_to_dom_codes() {
        assert_eq!(mac_code(0x00), Some("KeyA"));
        assert_eq!(mac_code(0x28), Some("KeyK"));
        assert_eq!(mac_code(0x1D), Some("Digit0"));
        assert_eq!(mac_code(0x37), Some("MetaLeft"));
        assert_eq!(mac_code(0x36), Some("MetaRight"));
        assert_eq!(mac_code(0x3A), Some("AltLeft"));
        assert_eq!(mac_code(0x30), Some("Tab"));
        assert_eq!(mac_code(0x31), Some("Space"));
        assert_eq!(mac_code(0x75), Some("Delete"));
        assert_eq!(mac_code(0x33), Some("Backspace"));
        assert_eq!(mac_code(0x7A), Some("F1"));
        assert_eq!(mac_code(0x7E), Some("ArrowUp"));
        assert_eq!(mac_code(0xFF), None);
        assert_eq!(mac_media_code(0), Some("AudioVolumeUp"));
        assert_eq!(mac_media_code(7), Some("AudioVolumeMute"));
        assert_eq!(mac_media_code(16), Some("MediaPlayPause"));
        assert_eq!(mac_media_code(2), None); // brightness: left to the Mac
        assert_eq!(mac_modifier_bit(0x37), Some(0x08));
        assert_eq!(mac_modifier_bit(0x00), None);
        // Every mac code is a known DOM code, and no two keys share one.
        let codes: Vec<_> = (0..=0x7F).filter_map(mac_code).collect();
        assert!(codes.iter().all(|c| ALL_CODES.contains(c)), "{codes:?}");
        let mut uniq = codes.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), codes.len());
    }

    #[test]
    fn windows_keys_map_to_dom_codes() {
        assert_eq!(win_code(0x41, 0x1E, false), Some("KeyA"));
        assert_eq!(win_code(0x09, 0x0F, false), Some("Tab"));
        assert_eq!(win_code(0x5B, 0x5B, true), Some("MetaLeft"));
        assert_eq!(win_code(0x12, 0x38, false), Some("AltLeft"));
        assert_eq!(win_code(0x12, 0x38, true), Some("AltRight"));
        assert_eq!(win_code(0x0D, 0x1C, true), Some("NumpadEnter"));
        assert_eq!(win_code(0x26, 0x48, true), Some("ArrowUp"));
        assert_eq!(win_code(0x68, 0x48, false), Some("Numpad8"));
        assert_eq!(win_code(0xAF, 0x30, true), Some("AudioVolumeUp"));
        assert_eq!(win_code(0xB3, 0x22, true), Some("MediaPlayPause"));
        assert_eq!(win_code(0x7C, 0x64, false), Some("F13"));
        assert_eq!(win_code(0, 0x54, false), None);
        for scan in 0..0x80 {
            for ext in [false, true] {
                if let Some(c) = win_code(0, scan, ext) {
                    assert!(ALL_CODES.contains(&c), "{c}");
                }
            }
        }
    }

    #[test]
    fn dom_keys_and_key_codes() {
        assert_eq!(dom_key("KeyA", false), "a");
        assert_eq!(dom_key("KeyA", true), "A");
        assert_eq!(dom_key("Digit2", true), "@");
        assert_eq!(dom_key("Slash", true), "?");
        assert_eq!(dom_key("MetaLeft", false), "Meta");
        assert_eq!(dom_key("Tab", false), "Tab");
        assert_eq!(key_code("KeyA"), 65);
        assert_eq!(key_code("Digit0"), 48);
        assert_eq!(key_code("F12"), 123);
        assert_eq!(key_code("MetaLeft"), 91);
        assert_eq!(key_code("Tab"), 9);
        assert_eq!(key_code("Numpad5"), 101);
    }

    #[test]
    fn shortcuts_parse_print_and_match() {
        let s: Shortcut = "Ctrl+Alt+Shift+K".parse().unwrap();
        assert_eq!(s, Shortcut::default());
        assert_eq!(s.code, "KeyK");
        assert_eq!("cmd+option+F12".parse::<Shortcut>().unwrap().to_string(), "Alt+Cmd+F12");
        assert_eq!("Win+Space".parse::<Shortcut>().unwrap().code, "Space");
        assert_eq!("F13".parse::<Shortcut>().unwrap().to_string(), "F13");
        assert_eq!("Pause".parse::<Shortcut>().unwrap().code, "Pause");
        assert_eq!("ctrl+shift+KeyR".parse::<Shortcut>().unwrap().to_string(), "Ctrl+Shift+R");
        assert!("K".parse::<Shortcut>().is_err(), "a bare letter would steal typing");
        assert!("Ctrl+Alt".parse::<Shortcut>().is_err());
        assert!("Ctrl+K+J".parse::<Shortcut>().is_err());
        assert!("Ctrl+Nope".parse::<Shortcut>().is_err());
        let m = Mods { ctrl: true, alt: true, shift: true, meta: false };
        assert!(s.matches("KeyK", m));
        assert!(!s.matches("KeyK", Mods { meta: true, ..m }), "exact modifiers only");
        assert!(!s.matches("KeyJ", m));
    }

    #[test]
    fn modifiers_are_tracked_per_side() {
        let mut t = ModTracker::default();
        assert!(t.update("ShiftLeft", true).shift);
        assert!(t.update("ShiftRight", true).shift);
        assert!(t.update("ShiftLeft", false).shift, "right shift still down");
        assert!(!t.update("ShiftRight", false).shift);
        let m = t.update("MetaLeft", true);
        assert_eq!(m.bits(), 8);
        assert_eq!(t.update("KeyA", true).bits(), 8, "letters do not change modifiers");
        assert_eq!(t.held(), ["MetaLeft"]);
        t.clear();
        assert_eq!(t.mods(), Mods::default());
    }

    #[test]
    fn events_become_page_script() {
        let e = KeyEvent { down: false, code: "Quote", mods: Mods { shift: true, ..Mods::default() }, repeat: true };
        assert_eq!(e.script(), r#"window.__kitchenWrap&&window.__kitchenWrap.key("keyup","\"","Quote",222,4,true)"#);
    }
}
