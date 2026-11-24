//! One registry of app actions for the system tray, global shortcuts and the app menu
//! (feature `tray-menu`).
//!
//! Every entry point names an action by its id (`show_hide`, `search`, `quit`, or one of
//! the app's own), and every entry point runs it the same way: [`Registry::dispatch`]
//! turns the id into an [`Action`] and calls the matching [`Host`] method. The Tauri
//! shell implements [`Host`]; tests use a fake. Nothing here depends on Tauri, so the
//! parsing, validation and dispatch are tested without a window.
//!
//! The settings live in [`crate::Config`] as `tray` ([`TrayConfig`]), `shortcuts`
//! ([`ShortcutBinding`]) and `menu` ([`MenuConfig`]). [`Registry::plan`] checks them
//! against the registry and returns what to build plus the problems to show (unknown
//! actions, bad accelerators, two actions on one key), instead of failing.
//!
//! ```
//! use desktop_core::actions::{Accelerator, Action, Host, Registry};
//!
//! let mut r = Registry::new();
//! r.register("export_csv", "Export CSV").unwrap();
//! r.default_shortcut("export_csv", "CmdOrCtrl+Shift+E").unwrap();
//!
//! assert_eq!(r.resolve("quit").unwrap(), Action::Quit);
//! assert_eq!(r.resolve("export_csv").unwrap(), Action::Custom("export_csv".into()));
//! assert!(r.resolve("nope").is_err());
//!
//! let a: Accelerator = "ctrl+shift+k".parse().unwrap();
//! assert_eq!(a.to_string(), "Ctrl+Shift+K");
//!
//! /// Records what ran.
//! #[derive(Default)]
//! struct Log(std::cell::RefCell<Vec<String>>);
//! impl Host for Log {
//!     fn show_hide(&self) -> Result<(), String> { self.0.borrow_mut().push("show_hide".into()); Ok(()) }
//!     fn show(&self) -> Result<(), String> { Ok(()) }
//!     fn hide(&self) -> Result<(), String> { Ok(()) }
//!     fn focus_search(&self) -> Result<(), String> { Ok(()) }
//!     fn ingest_clipboard(&self) -> Result<(), String> { Ok(()) }
//!     fn ingest_drop_folder(&self) -> Result<(), String> { Ok(()) }
//!     fn pause_resume(&self) -> Result<(), String> { Ok(()) }
//!     fn open_settings(&self) -> Result<(), String> { Ok(()) }
//!     fn quit(&self) -> Result<(), String> { Ok(()) }
//!     fn custom(&self, id: &str) -> Result<(), String> { self.0.borrow_mut().push(id.into()); Ok(()) }
//! }
//! let log = Log::default();
//! r.dispatch("show_hide", &log).unwrap();
//! r.dispatch("export_csv", &log).unwrap();
//! assert_eq!(*log.0.borrow(), ["show_hide", "export_csv"]);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// Why an action id, accelerator or binding was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    /// No built-in or registered action has this id.
    #[error("unknown action {0:?}")]
    Unknown(String),
    /// Ids are snake_case: a lower-case letter, then `a-z`, `0-9` or `_`.
    #[error("{0:?} is not a valid action id (use snake_case, like export_csv)")]
    BadId(String),
    /// The id is already a built-in or registered action.
    #[error("action {0:?} already exists")]
    Duplicate(String),
    /// The accelerator could not be read.
    #[error("{0}")]
    Accelerator(String),
    /// The host could not run the action.
    #[error("{action} failed: {message}")]
    Failed { action: String, message: String },
}

/// What an id stands for: a built-in action or one the app registered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// Show the main window, or hide it when it is in front.
    ShowHide,
    /// Show and focus the main window.
    Show,
    /// Hide the main window (the app keeps running in the tray).
    Hide,
    /// Show the window and focus the search box.
    Search,
    /// Ingest the text on the clipboard.
    IngestClipboard,
    /// Ingest everything in the drop folder now.
    IngestDropFolder,
    /// Pause background work (ingest and folder watching), or resume it.
    PauseResume,
    /// Show the window at its Settings.
    OpenSettings,
    /// Quit the app.
    Quit,
    /// An action the app registered: the UI gets an event and the backend a call.
    Custom(String),
}

/// The built-in actions: id, label and the action.
pub const BUILT_IN: &[(&str, &str, Action)] = &[
    ("show_hide", "Show/Hide window", Action::ShowHide),
    ("show", "Show window", Action::Show),
    ("hide", "Hide window", Action::Hide),
    ("search", "Search…", Action::Search),
    ("ingest_clipboard", "Add clipboard text", Action::IngestClipboard),
    ("ingest_drop_folder", "Import the drop folder", Action::IngestDropFolder),
    ("pause_resume", "Pause background work", Action::PauseResume),
    ("settings", "Settings…", Action::OpenSettings),
    ("quit", "Quit", Action::Quit),
];

/// The id that draws a separator line in a menu instead of an item.
pub const SEPARATOR: &str = "separator";

impl Action {
    /// The action's id, as settings and menus name it.
    pub fn id(&self) -> &str {
        match self {
            Action::Custom(id) => id,
            builtin => BUILT_IN.iter().find(|(_, _, a)| a == builtin).map(|(id, _, _)| *id).unwrap_or_default(),
        }
    }
}

/// What actually happens: the Tauri shell implements it, tests use a fake. Each method
/// returns a message on failure, which the caller shows; nothing panics.
pub trait Host {
    /// [`Action::ShowHide`].
    fn show_hide(&self) -> Result<(), String>;
    /// [`Action::Show`].
    fn show(&self) -> Result<(), String>;
    /// [`Action::Hide`].
    fn hide(&self) -> Result<(), String>;
    /// [`Action::Search`].
    fn focus_search(&self) -> Result<(), String>;
    /// [`Action::IngestClipboard`].
    fn ingest_clipboard(&self) -> Result<(), String>;
    /// [`Action::IngestDropFolder`].
    fn ingest_drop_folder(&self) -> Result<(), String>;
    /// [`Action::PauseResume`].
    fn pause_resume(&self) -> Result<(), String>;
    /// [`Action::OpenSettings`].
    fn open_settings(&self) -> Result<(), String>;
    /// [`Action::Quit`].
    fn quit(&self) -> Result<(), String>;
    /// An app-registered action, by id.
    fn custom(&self, id: &str) -> Result<(), String>;
}

/// The actions this app knows, with code-defined menu items and default shortcuts.
///
/// Start from [`Registry::new`] (the built-ins), then [`register`](Registry::register)
/// the app's own actions. `kitchen generate menu` and `kitchen generate shortcut` add
/// those calls for you.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    custom: BTreeMap<String, String>,
    menu: Vec<MenuEntry>,
    shortcuts: Vec<ShortcutBinding>,
}

/// An id is snake_case and not too long.
pub fn valid_id(id: &str) -> bool {
    id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Registry {
    /// The built-in actions only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an app action. It runs [`Host::custom`] with its id.
    pub fn register(&mut self, id: &str, label: &str) -> Result<(), ActionError> {
        if !valid_id(id) || id == SEPARATOR {
            return Err(ActionError::BadId(id.into()));
        }
        if self.resolve(id).is_ok() {
            return Err(ActionError::Duplicate(id.into()));
        }
        self.custom.insert(id.into(), label.into());
        Ok(())
    }

    /// Put an action in the app-defined menu from code (settings can add more), with an
    /// optional accelerator shown next to it and active while the app is in front.
    pub fn menu_item(&mut self, action: &str, accelerator: Option<&str>) -> Result<(), ActionError> {
        self.resolve(action)?;
        let hint = accelerator.map(|a| a.parse::<Accelerator>().map(|a| a.to_string())).transpose()?;
        self.menu.push(MenuEntry { id: action.into(), label: String::new(), action: action.into(), shortcut_hint: hint, enabled: true });
        Ok(())
    }

    /// A global shortcut that applies until the user binds that action or key in Settings.
    pub fn default_shortcut(&mut self, action: &str, accelerator: &str) -> Result<(), ActionError> {
        self.resolve(action)?;
        let a: Accelerator = accelerator.parse()?;
        self.shortcuts.push(ShortcutBinding { accelerator: a.to_string(), action: action.into() });
        Ok(())
    }

    /// The action an id names.
    pub fn resolve(&self, id: &str) -> Result<Action, ActionError> {
        if let Some((_, _, a)) = BUILT_IN.iter().find(|(b, _, _)| *b == id) {
            return Ok(a.clone());
        }
        if self.custom.contains_key(id) {
            return Ok(Action::Custom(id.into()));
        }
        Err(ActionError::Unknown(id.into()))
    }

    /// The label for an action (its id when unknown).
    pub fn label(&self, id: &str) -> String {
        BUILT_IN
            .iter()
            .find(|(b, _, _)| *b == id)
            .map(|(_, l, _)| l.to_string())
            .or_else(|| self.custom.get(id).cloned())
            .unwrap_or_else(|| id.to_string())
    }

    /// Every action as `(id, label)`, built-ins first: for pickers in Settings.
    pub fn list(&self) -> Vec<(String, String)> {
        BUILT_IN
            .iter()
            .map(|(id, l, _)| (id.to_string(), l.to_string()))
            .chain(self.custom.iter().map(|(id, l)| (id.clone(), l.clone())))
            .collect()
    }

    /// Run the action `id` on `host`.
    pub fn dispatch(&self, id: &str, host: &dyn Host) -> Result<Action, ActionError> {
        let action = self.resolve(id)?;
        let r = match &action {
            Action::ShowHide => host.show_hide(),
            Action::Show => host.show(),
            Action::Hide => host.hide(),
            Action::Search => host.focus_search(),
            Action::IngestClipboard => host.ingest_clipboard(),
            Action::IngestDropFolder => host.ingest_drop_folder(),
            Action::PauseResume => host.pause_resume(),
            Action::OpenSettings => host.open_settings(),
            Action::Quit => host.quit(),
            Action::Custom(id) => host.custom(id),
        };
        r.map(|()| action).map_err(|message| ActionError::Failed { action: id.into(), message })
    }

    /// Check the settings against this registry: the tray items, the app menu (code
    /// items, then settings items) and the global shortcuts to register. Problems are
    /// collected, and the bad entries left out, so one typo never breaks the rest.
    pub fn plan(&self, tray: &TrayConfig, menu: &MenuConfig, shortcuts: &[ShortcutBinding]) -> Plan {
        let mut problems = Vec::new();
        let mut items = |list: &[MenuEntry], place: &str| -> Vec<PlannedItem> {
            list.iter()
                .filter_map(|e| match self.item(e) {
                    Ok(i) => Some(i),
                    Err(err) => {
                        problems.push(format!("{place} item {:?}: {err}", e.label_or_id()));
                        None
                    }
                })
                .collect()
        };
        let tray_items = if tray.enabled { items(&tray.items, "Tray") } else { Vec::new() };
        let menu_items = items(&[self.menu.as_slice(), menu.extra.as_slice()].concat(), "Menu");
        let mac = cfg!(target_os = "macos");
        let mut taken: BTreeMap<(u8, String), String> = BTreeMap::new();
        let mut bound: Vec<(Accelerator, String)> = Vec::new();
        // Settings first; a code default is used only when settings bind neither its key
        // nor its action. An empty accelerator in settings turns an action's default off.
        let user_actions: Vec<&str> = shortcuts.iter().map(|s| s.action.as_str()).collect();
        let defaults = self.shortcuts.iter().filter(|d| !user_actions.contains(&d.action.as_str()));
        for (s, from_code) in shortcuts.iter().map(|s| (s, false)).chain(defaults.map(|s| (s, true))) {
            if s.accelerator.trim().is_empty() {
                continue;
            }
            let what = format!("Shortcut {} ({})", s.accelerator, s.action);
            if let Err(e) = self.resolve(&s.action) {
                problems.push(format!("{what}: {e}"));
                continue;
            }
            let a = match s.accelerator.parse::<Accelerator>().and_then(|a| a.check_global().map(|()| a)) {
                Ok(a) => a,
                Err(e) => {
                    problems.push(format!("{what}: {e}"));
                    continue;
                }
            };
            match taken.get(&a.key_on(mac)) {
                Some(other) if from_code => log::debug!("default shortcut {a} for {} skipped: {other} has it", s.action),
                Some(other) => problems.push(format!("{what}: the same keys as {other}")),
                None => {
                    taken.insert(a.key_on(mac), s.action.clone());
                    bound.push((a, s.action.clone()));
                }
            }
        }
        Plan { tray: tray_items, menu: menu_items, shortcuts: bound, problems }
    }

    fn item(&self, e: &MenuEntry) -> Result<PlannedItem, ActionError> {
        if e.action == SEPARATOR {
            return Ok(PlannedItem::Separator);
        }
        self.resolve(&e.action)?;
        let accelerator = match e.shortcut_hint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(h) => Some(h.parse::<Accelerator>()?),
            None => None,
        };
        let label = if e.label.trim().is_empty() { self.label(&e.action) } else { e.label.trim().to_string() };
        Ok(PlannedItem::Item { menu_id: menu_id(&e.action, &e.id), label, accelerator, enabled: e.enabled })
    }
}

/// What to build, from [`Registry::plan`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// The tray menu, in order (empty when the tray is off).
    pub tray: Vec<PlannedItem>,
    /// The app-defined menu, in order (empty: no extra menu).
    pub menu: Vec<PlannedItem>,
    /// Global shortcuts to register, each with its action id.
    pub shortcuts: Vec<(Accelerator, String)>,
    /// What was left out, and why, for the Settings page.
    pub problems: Vec<String>,
}

/// One checked menu entry.
#[derive(Debug, Clone, PartialEq)]
pub enum PlannedItem {
    /// A clickable item. `menu_id` is [`menu_id`]; menu events carry it back.
    Item { menu_id: String, label: String, accelerator: Option<Accelerator>, enabled: bool },
    /// A separator line.
    Separator,
}

/// The id a menu item carries: the action, then `#` and the item's own id, so two items
/// may run the same action and still be told apart.
///
/// ```
/// use desktop_core::actions::{action_of, menu_id};
/// assert_eq!(menu_id("quit", "q"), "quit#q");
/// assert_eq!(action_of("quit#q"), Some("quit"));
/// assert_eq!(action_of("about"), None);
/// ```
pub fn menu_id(action: &str, item_id: &str) -> String {
    format!("{action}#{item_id}")
}

/// The action of a menu id made by [`menu_id`]; `None` for other menu items (the
/// platform's own Edit and Window items).
pub fn action_of(menu_id: &str) -> Option<&str> {
    menu_id.split_once('#').map(|(a, _)| a).filter(|a| valid_id(a))
}

/// A tray or menu entry in settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuEntry {
    /// The item's own id (unique in its menu).
    pub id: String,
    /// The text shown; empty uses the action's label.
    pub label: String,
    /// The action it runs, or `separator`.
    pub action: String,
    /// An accelerator shown next to it (in the app menu it also works while the app is
    /// in front), e.g. `CmdOrCtrl+Shift+K`.
    pub shortcut_hint: Option<String>,
    /// Greyed out when false.
    pub enabled: bool,
}

impl Default for MenuEntry {
    fn default() -> Self {
        Self { id: String::new(), label: String::new(), action: String::new(), shortcut_hint: None, enabled: true }
    }
}

impl MenuEntry {
    /// An entry that runs `action` with its default label.
    pub fn new(action: &str) -> Self {
        Self { id: action.into(), action: action.into(), ..Self::default() }
    }

    fn label_or_id(&self) -> &str {
        if self.label.is_empty() {
            &self.action
        } else {
            &self.label
        }
    }
}

/// The system tray (Windows, Linux) or menu-bar icon (macOS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrayConfig {
    /// Show the icon. Off by default.
    pub enabled: bool,
    /// The menu, in order.
    pub items: Vec<MenuEntry>,
    /// macOS: no Dock icon, only the menu-bar icon (the window still opens from it).
    pub show_in_menu_bar_only: bool,
    /// Closing the window hides it; quit from the tray menu.
    pub close_to_tray: bool,
    /// A left click shows the window (the menu opens with a right click). When off, a
    /// left click opens the menu.
    pub left_click_opens: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        let items = ["show_hide", "search", "ingest_clipboard", "pause_resume", SEPARATOR, "settings", "quit"]
            .iter()
            .enumerate()
            .map(|(i, a)| if *a == SEPARATOR { MenuEntry { id: format!("sep{i}"), ..MenuEntry::new(a) } } else { MenuEntry::new(a) })
            .collect();
        Self { enabled: false, items, show_in_menu_bar_only: false, close_to_tray: false, left_click_opens: true }
    }
}

/// The app-defined menu next to the platform's own (App, Edit, Window on macOS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuConfig {
    /// Its title in the menu bar.
    pub title: String,
    /// Items after the ones the code defines (`Registry::menu_item`).
    pub extra: Vec<MenuEntry>,
}

impl Default for MenuConfig {
    fn default() -> Self {
        Self { title: "Actions".into(), extra: Vec::new() }
    }
}

/// A global shortcut: works while another app is in front.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ShortcutBinding {
    /// Like `CmdOrCtrl+Shift+K`; empty turns off the code's default for this action.
    pub accelerator: String,
    /// The action it runs.
    pub action: String,
}

impl TrayConfig {
    /// Trimmed ids and actions; items with no action dropped; ids filled in and unique.
    pub fn validated(mut self) -> Self {
        self.items = clean(self.items);
        self
    }
}

impl MenuConfig {
    /// Like [`TrayConfig::validated`]; an empty title becomes `Actions`.
    pub fn validated(mut self) -> Self {
        self.extra = clean(self.extra);
        if self.title.trim().is_empty() {
            self.title = MenuConfig::default().title;
        }
        self
    }
}

/// Trimmed bindings; ones with no action dropped; accelerators written the standard way
/// when they parse (bad ones are kept so Settings can show the problem).
pub fn validated_shortcuts(list: Vec<ShortcutBinding>) -> Vec<ShortcutBinding> {
    list.into_iter()
        .map(|s| ShortcutBinding {
            accelerator: s.accelerator.parse::<Accelerator>().map(|a| a.to_string()).unwrap_or_else(|_| s.accelerator.trim().to_string()),
            action: s.action.trim().to_string(),
        })
        .filter(|s| !s.action.is_empty())
        .take(100)
        .collect()
}

fn clean(list: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut seen = std::collections::BTreeSet::new();
    list.into_iter()
        .map(|mut e| {
            e.action = e.action.trim().to_string();
            e.id = e.id.trim().to_string();
            e
        })
        .filter(|e| !e.action.is_empty())
        .take(100)
        .enumerate()
        .map(|(i, mut e)| {
            if e.id.is_empty() || e.id.contains('#') || seen.contains(&e.id) {
                e.id = format!("{}{i}", e.action);
            }
            seen.insert(e.id.clone());
            e
        })
        .collect()
}

/// Modifier keys. `cmd_or_ctrl` is Command on macOS and Control elsewhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub cmd_or_ctrl: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command on macOS, the Windows key elsewhere.
    pub super_key: bool,
}

/// A key combination like `CmdOrCtrl+Shift+K`, in the syntax Tauri's menus and the
/// global-shortcut plugin both read. Parsing accepts the usual aliases (`Cmd`,
/// `Command`, `Option`, `Control`, `Esc`, `Return`, `Up`, `,`) in any case, and
/// `to_string` writes the standard form.
///
/// ```
/// use desktop_core::actions::Accelerator;
/// let a: Accelerator = "cmd+option+,".parse().unwrap();
/// assert_eq!(a.to_string(), "Alt+Super+Comma");
/// assert!("Ctrl+".parse::<Accelerator>().is_err());
/// assert!("Shift+K+L".parse::<Accelerator>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accelerator {
    pub modifiers: Modifiers,
    /// The main key, in its standard name (`K`, `1`, `F5`, `Space`, `ArrowUp`, `Comma`).
    pub key: String,
}

impl FromStr for Accelerator {
    type Err = ActionError;

    fn from_str(s: &str) -> Result<Self, ActionError> {
        let bad = |why: &str| ActionError::Accelerator(format!("{s:?}: {why}"));
        let s = s.trim();
        if s.is_empty() {
            return Err(bad("empty"));
        }
        let (head, last) = match s.rsplit_once('+') {
            Some((h, l)) => (Some(h), l),
            None => (None, s),
        };
        let mut m = Modifiers::default();
        for part in head.into_iter().flat_map(|h| h.split('+')) {
            let flag = match part.trim().to_ascii_uppercase().as_str() {
                "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => &mut m.cmd_or_ctrl,
                "CONTROL" | "CTRL" => &mut m.ctrl,
                "ALT" | "OPTION" => &mut m.alt,
                "SHIFT" => &mut m.shift,
                "SUPER" | "CMD" | "COMMAND" | "META" | "WIN" => &mut m.super_key,
                "" => return Err(bad("an empty part")),
                other if key_name(other).is_some() => return Err(bad("only one key after the modifiers")),
                other => return Err(bad(&format!("unknown modifier {other:?}"))),
            };
            if *flag {
                return Err(bad("a modifier twice"));
            }
            *flag = true;
        }
        let last = last.trim();
        if last.is_empty() {
            return Err(bad("no key after the modifiers"));
        }
        if is_modifier(last) {
            return Err(bad("it ends with a modifier; add a key"));
        }
        let key = key_name(last).ok_or_else(|| bad(&format!("unknown key {last:?}")))?;
        Ok(Self { modifiers: m, key })
    }
}

impl fmt::Display for Accelerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = &self.modifiers;
        for (on, name) in [(m.cmd_or_ctrl, "CmdOrCtrl"), (m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.super_key, "Super")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

impl Accelerator {
    /// The keys as pressed on this platform (`mac`: CmdOrCtrl is Command), to find two
    /// bindings that are really the same.
    pub fn key_on(&self, mac: bool) -> (u8, String) {
        let m = &self.modifiers;
        let (ctrl, sup) = (m.ctrl || (m.cmd_or_ctrl && !mac), m.super_key || (m.cmd_or_ctrl && mac));
        let bits = u8::from(ctrl) | u8::from(m.alt) << 1 | u8::from(m.shift) << 2 | u8::from(sup) << 3;
        (bits, self.key.clone())
    }

    /// Whether it may be a global shortcut: a letter, digit, punctuation or Space key
    /// needs a modifier other than Shift, or it would swallow typing in every app.
    pub fn check_global(&self) -> Result<(), ActionError> {
        let m = &self.modifiers;
        let typing = self.key.len() == 1 || PUNCTUATION.iter().any(|(_, n)| *n == self.key) || self.key == "Space";
        if typing && !(m.cmd_or_ctrl || m.ctrl || m.alt || m.super_key) {
            return Err(ActionError::Accelerator(format!("{self}: add Ctrl, Alt or Cmd, or it would take over typing everywhere")));
        }
        Ok(())
    }
}

const PUNCTUATION: &[(&str, &str)] = &[
    (",", "Comma"),
    (".", "Period"),
    ("-", "Minus"),
    ("=", "Equal"),
    (";", "Semicolon"),
    ("'", "Quote"),
    ("/", "Slash"),
    ("\\", "Backslash"),
    ("`", "Backquote"),
    ("[", "BracketLeft"),
    ("]", "BracketRight"),
];

const NAMED: &[&str] = &[
    "Space", "Enter", "Tab", "Escape", "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
];

fn is_modifier(s: &str) -> bool {
    matches!(
        s.to_ascii_uppercase().as_str(),
        "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" | "CONTROL" | "CTRL" | "ALT" | "OPTION" | "SHIFT" | "SUPER" | "CMD" | "COMMAND" | "META" | "WIN"
    )
}

/// The standard name of a key, from any common spelling.
fn key_name(k: &str) -> Option<String> {
    let up = k.to_ascii_uppercase();
    let one = |c: char| (c.is_ascii_alphanumeric()).then(|| c.to_ascii_uppercase().to_string());
    if let Some(c) = (k.chars().count() == 1).then(|| k.chars().next()).flatten() {
        return one(c).or_else(|| PUNCTUATION.iter().find(|(p, _)| p.starts_with(c)).map(|(_, n)| n.to_string()));
    }
    if let Some(rest) = up.strip_prefix("KEY").filter(|r| r.len() == 1 && r.chars().all(|c| c.is_ascii_uppercase())) {
        return Some(rest.into());
    }
    if let Some(rest) = up.strip_prefix("DIGIT").filter(|r| r.len() == 1 && r.chars().all(|c| c.is_ascii_digit())) {
        return Some(rest.into());
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(format!("F{n}"));
    }
    let alias = match up.as_str() {
        "ESC" => "ESCAPE",
        "RETURN" => "ENTER",
        "UP" => "ARROWUP",
        "DOWN" => "ARROWDOWN",
        "LEFT" => "ARROWLEFT",
        "RIGHT" => "ARROWRIGHT",
        "DEL" => "DELETE",
        other => other,
    };
    NAMED
        .iter()
        .chain(PUNCTUATION.iter().map(|(_, n)| n))
        .find(|n| n.eq_ignore_ascii_case(alias))
        .map(|n| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        ran: RefCell<Vec<String>>,
        fail: bool,
    }

    impl Fake {
        fn go(&self, what: &str) -> Result<(), String> {
            self.ran.borrow_mut().push(what.into());
            if self.fail {
                Err("no window".into())
            } else {
                Ok(())
            }
        }
    }

    impl Host for Fake {
        fn show_hide(&self) -> Result<(), String> {
            self.go("show_hide")
        }
        fn show(&self) -> Result<(), String> {
            self.go("show")
        }
        fn hide(&self) -> Result<(), String> {
            self.go("hide")
        }
        fn focus_search(&self) -> Result<(), String> {
            self.go("search")
        }
        fn ingest_clipboard(&self) -> Result<(), String> {
            self.go("ingest_clipboard")
        }
        fn ingest_drop_folder(&self) -> Result<(), String> {
            self.go("ingest_drop_folder")
        }
        fn pause_resume(&self) -> Result<(), String> {
            self.go("pause_resume")
        }
        fn open_settings(&self) -> Result<(), String> {
            self.go("settings")
        }
        fn quit(&self) -> Result<(), String> {
            self.go("quit")
        }
        fn custom(&self, id: &str) -> Result<(), String> {
            self.go(&format!("custom:{id}"))
        }
    }

    fn acc(s: &str) -> String {
        s.parse::<Accelerator>().map(|a| a.to_string()).unwrap_or_else(|e| format!("ERR {e}"))
    }

    #[test]
    fn parses_accelerators_in_any_spelling() {
        assert_eq!(acc("CmdOrCtrl+Shift+K"), "CmdOrCtrl+Shift+K");
        assert_eq!(acc("commandorcontrol+shift+k"), "CmdOrCtrl+Shift+K");
        assert_eq!(acc("Shift+Ctrl+KeyK"), "Ctrl+Shift+K");
        assert_eq!(acc("Option+Cmd+Digit1"), "Alt+Super+1");
        assert_eq!(acc("ctrl+esc"), "Ctrl+Escape");
        assert_eq!(acc("Alt+up"), "Alt+ArrowUp");
        assert_eq!(acc("F13"), "F13");
        assert_eq!(acc("Ctrl+/"), "Ctrl+Slash");
        assert_eq!(acc("Ctrl+Return"), "Ctrl+Enter");
        for bad in ["", "Ctrl+", "Ctrl+Shift", "Hyper+K", "Ctrl+K+L", "Ctrl+Ctrl+K", "Ctrl+F25", "Ctrl+Ñ", "+K", "Ctrl++"] {
            assert!(bad.parse::<Accelerator>().is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn global_shortcuts_need_a_real_modifier_on_typing_keys() {
        let ok = |s: &str| s.parse::<Accelerator>().unwrap().check_global().is_ok();
        assert!(ok("Ctrl+K"));
        assert!(ok("F5"));
        assert!(ok("Shift+F5"));
        assert!(!ok("K"));
        assert!(!ok("Shift+K"));
        assert!(!ok("Shift+Comma"));
        assert!(!ok("Space"));
    }

    #[test]
    fn cmd_or_ctrl_matches_the_platform_key() {
        let a: Accelerator = "CmdOrCtrl+K".parse().unwrap();
        assert_eq!(a.key_on(true), "Super+K".parse::<Accelerator>().unwrap().key_on(true));
        assert_eq!(a.key_on(false), "Ctrl+K".parse::<Accelerator>().unwrap().key_on(false));
        assert_ne!(a.key_on(true), "Ctrl+K".parse::<Accelerator>().unwrap().key_on(true));
    }

    #[test]
    fn registry_resolves_registers_and_refuses() {
        let mut r = Registry::new();
        assert_eq!(r.resolve("pause_resume").unwrap(), Action::PauseResume);
        assert_eq!(Action::PauseResume.id(), "pause_resume");
        assert_eq!(r.register("quit", "Mine"), Err(ActionError::Duplicate("quit".into())));
        assert_eq!(r.register("Bad-Id", "x"), Err(ActionError::BadId("Bad-Id".into())));
        assert_eq!(r.register(SEPARATOR, "x"), Err(ActionError::BadId(SEPARATOR.into())));
        r.register("export_csv", "Export CSV").unwrap();
        assert_eq!(r.register("export_csv", "again"), Err(ActionError::Duplicate("export_csv".into())));
        assert_eq!(r.label("export_csv"), "Export CSV");
        assert_eq!(r.label("quit"), "Quit");
        assert_eq!(r.list().len(), BUILT_IN.len() + 1);
        assert!(r.default_shortcut("nope", "Ctrl+K").is_err());
        assert!(r.default_shortcut("quit", "Ctrl+").is_err());
        assert!(r.menu_item("nope", None).is_err());
    }

    #[test]
    fn dispatch_reaches_the_right_host_method() {
        let mut r = Registry::new();
        r.register("export_csv", "Export CSV").unwrap();
        let host = Fake::default();
        for (id, _, _) in BUILT_IN {
            r.dispatch(id, &host).unwrap();
        }
        assert_eq!(r.dispatch("export_csv", &host).unwrap(), Action::Custom("export_csv".into()));
        assert_eq!(
            *host.ran.borrow(),
            ["show_hide", "show", "hide", "search", "ingest_clipboard", "ingest_drop_folder", "pause_resume", "settings", "quit", "custom:export_csv"]
        );
        assert_eq!(r.dispatch("nope", &host), Err(ActionError::Unknown("nope".into())));
        let failing = Fake { fail: true, ..Fake::default() };
        let e = r.dispatch("show", &failing).unwrap_err();
        assert_eq!(e.to_string(), "show failed: no window");
    }

    #[test]
    fn plan_reports_problems_and_keeps_the_rest() {
        let mut r = Registry::new();
        r.register("export_csv", "Export CSV").unwrap();
        r.menu_item("export_csv", Some("cmdorctrl+e")).unwrap();
        r.default_shortcut("export_csv", "CmdOrCtrl+Shift+E").unwrap();
        r.default_shortcut("search", "CmdOrCtrl+Shift+F").unwrap();
        let tray = TrayConfig {
            enabled: true,
            items: vec![MenuEntry::new("show_hide"), MenuEntry::new("nope"), MenuEntry::new(SEPARATOR), MenuEntry { label: "Bye".into(), ..MenuEntry::new("quit") }],
            ..TrayConfig::default()
        };
        let menu = MenuConfig { extra: vec![MenuEntry { shortcut_hint: Some("Ctrl+".into()), ..MenuEntry::new("quit") }, MenuEntry::new("settings")], ..MenuConfig::default() };
        let shortcuts = vec![
            ShortcutBinding { accelerator: "Ctrl+Alt+Space".into(), action: "show_hide".into() },
            ShortcutBinding { accelerator: "ctrl+alt+space".into(), action: "quit".into() },
            ShortcutBinding { accelerator: "K".into(), action: "search".into() },
            ShortcutBinding { accelerator: "Ctrl+J".into(), action: "missing".into() },
            ShortcutBinding { accelerator: String::new(), action: "export_csv".into() },
        ];
        let p = r.plan(&tray, &menu, &shortcuts);
        assert_eq!(p.tray.len(), 3);
        assert_eq!(p.tray[0], PlannedItem::Item { menu_id: "show_hide#show_hide".into(), label: "Show/Hide window".into(), accelerator: None, enabled: true });
        assert_eq!(p.tray[1], PlannedItem::Separator);
        assert!(matches!(&p.tray[2], PlannedItem::Item { label, .. } if label == "Bye"));
        assert_eq!(p.menu.len(), 2, "code item + settings; the bad hint is left out");
        assert!(matches!(&p.menu[0], PlannedItem::Item { accelerator: Some(a), .. } if a.to_string() == "CmdOrCtrl+E"));
        // show_hide bound; quit clashes; K needs a modifier; missing is unknown; the
        // export_csv default is turned off; search's code default does not apply because
        // settings bind search (even though that binding is bad).
        let bound: Vec<String> = p.shortcuts.iter().map(|(a, id)| format!("{a}={id}")).collect();
        assert_eq!(bound, ["Ctrl+Alt+Space=show_hide"]);
        assert_eq!(p.problems.len(), 5, "{:#?}", p.problems);
        assert!(p.problems.iter().any(|m| m.contains("the same keys as show_hide")));
        assert!(p.problems.iter().any(|m| m.contains("unknown action \"nope\"")));
        let off = r.plan(&TrayConfig::default(), &MenuConfig::default(), &[]);
        assert!(off.tray.is_empty(), "tray off by default");
        assert_eq!(off.shortcuts.len(), 2, "code defaults apply when settings are empty");
        assert!(off.problems.is_empty());
    }

    #[test]
    fn settings_are_cleaned() {
        let t = TrayConfig {
            items: vec![MenuEntry::new(" quit "), MenuEntry::new(""), MenuEntry { id: "quit".into(), ..MenuEntry::new("show") }, MenuEntry { id: "a#b".into(), ..MenuEntry::new("hide") }],
            ..TrayConfig::default()
        }
        .validated();
        let ids: Vec<&str> = t.items.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["quit", "show1", "hide2"]);
        assert_eq!(MenuConfig { title: " ".into(), extra: vec![] }.validated().title, "Actions");
        let s = validated_shortcuts(vec![
            ShortcutBinding { accelerator: "ctrl+k".into(), action: " search ".into() },
            ShortcutBinding { accelerator: "bad+".into(), action: "quit".into() },
            ShortcutBinding { accelerator: "Ctrl+J".into(), action: String::new() },
        ]);
        assert_eq!(s, [ShortcutBinding { accelerator: "Ctrl+K".into(), action: "search".into() }, ShortcutBinding { accelerator: "bad+".into(), action: "quit".into() }]);
        let json: TrayConfig = serde_json::from_str(r#"{"enabled": true, "items": [{"action": "quit"}]}"#).unwrap();
        assert!(json.items[0].enabled, "items are enabled unless switched off");
        assert!(json.left_click_opens);
    }

    #[test]
    fn menu_ids_round_trip() {
        assert_eq!(action_of(&menu_id("export_csv", "x1")), Some("export_csv"));
        assert_eq!(action_of("Bad#x"), None);
        assert_eq!(action_of("copy"), None);
    }
}
