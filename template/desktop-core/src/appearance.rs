//! Theme, light/dark/system mode and fonts for the app shell (feature `app-shell`).
//!
//! The settings live in [`crate::Config`] as `appearance` ([`AppearanceConfig`]) and are
//! saved with the rest of `settings.json`. [`AppearanceConfig::validated`] puts every value
//! back into a known set before it is used, because the values end up in CSS and in a
//! script the window runs before the page loads ([`AppearanceConfig::init_script`], which
//! is what keeps a dark-mode user from seeing a white flash at start).
//!
//! - `mode`: `system` (follow the OS, the default), `light` or `dark`;
//! - `theme`: one of [`THEMES`] (the CSS for each lives in the UI's `shell.css`);
//! - `uiFont`, `monoFont`: empty for the system font, a preset from [`UI_FONTS`] /
//!   [`MONO_FONTS`], or the name of an installed or bundled font family;
//! - `fontSize`: the base size in pixels (10 to 24).
//!
//! A font name that could break out of CSS (quotes, `;`, braces, `<`, ...) is dropped
//! and the system font is used instead.
//!
//! # Example
//!
//! ```
//! use desktop_core::appearance::{AppearanceConfig, Mode};
//!
//! let a: AppearanceConfig = serde_json::from_str(
//!     r#"{"mode": "dark", "theme": "nord", "uiFont": "Inter", "monoFont": "x;} body{display:none", "fontSize": 99}"#,
//! )?;
//! let a = a.validated();
//! assert_eq!(a.mode, "dark");
//! assert_eq!(a.parsed_mode(), Mode::Dark);
//! assert_eq!(a.theme, "nord");
//! assert_eq!(a.ui_font, "Inter");
//! assert_eq!(a.mono_font, ""); // unsafe name dropped: the system monospace font is used
//! assert_eq!(a.font_size, 24); // clamped
//!
//! // Unknown values fall back to the defaults instead of failing the whole settings file.
//! let b: AppearanceConfig = serde_json::from_str(r#"{"mode": "sepia", "theme": "nope"}"#)?;
//! assert_eq!(b.validated(), AppearanceConfig::default());
//! # Ok::<(), serde_json::Error>(())
//! ```

use serde::{Deserialize, Serialize};

/// A named colour theme: its id (saved in settings, used as `data-theme` in CSS) and the
/// label the Settings screen shows. Each has a light and a dark variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Theme {
    /// The id, e.g. `nord`.
    pub id: &'static str,
    /// What the Settings screen shows.
    pub label: &'static str,
}

/// The themes the UI ships (`shell.css` has the colours). Add one here and there.
pub const THEMES: &[Theme] = &[
    Theme { id: "kitchen", label: "Kitchen" },
    Theme { id: "nord", label: "Nord" },
    Theme { id: "solarized", label: "Solarized" },
    Theme { id: "forest", label: "Forest" },
    Theme { id: "rose", label: "Rosé" },
    Theme { id: "contrast", label: "High contrast" },
];

/// Font presets for the interface: `(id, CSS font stack)`. Families that are not installed
/// are skipped by the web view, so each stack ends with a generic family.
pub const UI_FONTS: &[(&str, &str)] = &[
    ("system", "system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif"),
    ("humanist", "Seravek, 'Gill Sans Nova', Ubuntu, Calibri, 'DejaVu Sans', source-sans-pro, sans-serif"),
    ("rounded", "ui-rounded, 'Hiragino Maru Gothic ProN', Quicksand, Comfortaa, Manjari, 'Arial Rounded MT', sans-serif"),
    ("serif", "Charter, 'Bitstream Charter', 'Sitka Text', Cambria, Georgia, serif"),
];

/// Font presets for code and editors: `(id, CSS font stack)`.
pub const MONO_FONTS: &[(&str, &str)] = &[
    ("system", "ui-monospace, 'Cascadia Code', 'Source Code Pro', Menlo, Consolas, 'DejaVu Sans Mono', monospace"),
    ("cascadia", "'Cascadia Code', 'Cascadia Mono', ui-monospace, monospace"),
    ("jetbrains", "'JetBrains Mono', ui-monospace, monospace"),
    ("fira", "'Fira Code', 'Fira Mono', ui-monospace, monospace"),
];

/// The modes, as saved.
pub const MODES: &[&str] = &["system", "light", "dark"];

/// Light, dark, or follow the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Follow the OS (`prefers-color-scheme`).
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// The `appearance` section of the settings.
///
/// `mode` and `theme` are plain strings (checked by [`validated`](Self::validated)) so a
/// value written by a newer or older app never makes the whole settings file unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceConfig {
    /// `system`, `light` or `dark`.
    pub mode: String,
    /// A theme id from [`THEMES`].
    pub theme: String,
    /// Interface font: empty (system), a [`UI_FONTS`] id, or a font family name.
    pub ui_font: String,
    /// Code and editor font: empty (system), a [`MONO_FONTS`] id, or a font family name.
    pub mono_font: String,
    /// Base font size in pixels, 10 to 24.
    pub font_size: u8,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self { mode: "system".into(), theme: THEMES[0].id.into(), ui_font: String::new(), mono_font: String::new(), font_size: 15 }
    }
}

/// Whether `name` is safe to use as a font family in CSS: letters (any script), digits,
/// spaces, `-`, `_` and `.`, at most 64 characters.
pub fn valid_font(name: &str) -> bool {
    name.chars().count() <= 64 && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}

impl AppearanceConfig {
    /// The same settings with every value in its known set (see the module docs).
    pub fn validated(mut self) -> Self {
        let d = Self::default();
        self.mode = self.mode.trim().to_ascii_lowercase();
        if !MODES.contains(&self.mode.as_str()) {
            self.mode = d.mode;
        }
        self.theme = self.theme.trim().to_ascii_lowercase();
        if !THEMES.iter().any(|t| t.id == self.theme) {
            self.theme = d.theme;
        }
        for font in [&mut self.ui_font, &mut self.mono_font] {
            *font = font.trim().to_string();
            if !valid_font(font) || font == "system" {
                font.clear();
            }
        }
        self.font_size = self.font_size.clamp(10, 24);
        self
    }

    /// The mode as an enum (`System` for anything unknown).
    pub fn parsed_mode(&self) -> Mode {
        match self.mode.as_str() {
            "light" => Mode::Light,
            "dark" => Mode::Dark,
            _ => Mode::System,
        }
    }

    /// The CSS `font-family` for the interface.
    pub fn ui_stack(&self) -> String {
        stack(&self.ui_font, UI_FONTS)
    }

    /// The CSS `font-family` for code and editors.
    pub fn mono_stack(&self) -> String {
        stack(&self.mono_font, MONO_FONTS)
    }

    /// What the UI needs to draw itself: the validated settings plus the resolved font
    /// stacks, as JSON (the `appearance` event and the init script carry this).
    pub fn resolved(&self) -> serde_json::Value {
        let a = self.clone().validated();
        serde_json::json!({
            "mode": a.mode,
            "theme": a.theme,
            "uiFont": a.ui_font,
            "monoFont": a.mono_font,
            "fontSize": a.font_size,
            "uiStack": a.ui_stack(),
            "monoStack": a.mono_stack(),
        })
    }

    /// A script for the web view to run before the page loads: it sets `data-theme`,
    /// `data-mode`, `data-scheme` (the resolved `light`/`dark`) and `data-os` on `<html>`
    /// plus the font variables, and keeps `data-scheme` in step with the OS in `system`
    /// mode. The UI (`ui/shell.js`, `ui-react/src/shell.js`) calls `window.__appearance.apply` for later
    /// changes.
    ///
    /// A window can get two of these: the app-wide one built at start, and a fresher one
    /// for a window opened later. The higher `priority` wins, whatever order they run in.
    ///
    /// ```
    /// use desktop_core::appearance::AppearanceConfig;
    ///
    /// let js = AppearanceConfig::default().init_script("macos", 0);
    /// assert!(js.contains(r#""theme":"kitchen""#));
    /// assert!(js.contains(r#"root.dataset.os = "macos""#));
    /// ```
    pub fn init_script(&self, os: &str, priority: u8) -> String {
        // serde_json escapes quotes and control characters, so the values stay data.
        let data = self.resolved().to_string().replace("</", "<\\/");
        let os = serde_json::Value::from(os).to_string();
        INIT_SCRIPT.replace("__PRIORITY__", &priority.to_string()).replace("__OS__", &os).replace("__DATA__", &data)
    }
}

/// The script behind [`AppearanceConfig::init_script`], with `__PRIORITY__`, `__OS__` and
/// `__DATA__` filled in.
const INIT_SCRIPT: &str = r#"(function () {
  var p = __PRIORITY__;
  var s = window.__appearance;
  if (s && s.priority > p) return;
  var q = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  function apply(a) {
    var root = document.documentElement;
    window.__appearance.current = a;
    if (!root || !a) return;
    root.dataset.os = __OS__;
    root.dataset.theme = a.theme;
    root.dataset.mode = a.mode;
    root.dataset.scheme = a.mode === 'system' ? (q && q.matches ? 'dark' : 'light') : a.mode;
    root.style.colorScheme = root.dataset.scheme;
    root.style.setProperty('--font-ui', a.uiStack);
    root.style.setProperty('--font-mono', a.monoStack);
    root.style.setProperty('--font-size', a.fontSize + 'px');
  }
  var first = !s;
  window.__appearance = { apply: apply, current: null, priority: p };
  var data = __DATA__;
  if (document.documentElement) apply(data);
  else {
    // Some web views run this before <html> exists: apply as soon as it does.
    window.__appearance.current = data;
    new MutationObserver(function (_, o) {
      if (!document.documentElement) return;
      o.disconnect();
      window.__appearance.apply(window.__appearance.current);
    }).observe(document, { childList: true });
  }
  if (q && first) q.addEventListener('change', function () { window.__appearance.apply(window.__appearance.current); });
})();"#;

fn stack(font: &str, presets: &[(&str, &str)]) -> String {
    if let Some((_, s)) = presets.iter().find(|(id, _)| *id == font) {
        return s.to_string();
    }
    let system = presets[0].1;
    if font.is_empty() || !valid_font(font) {
        system.to_string()
    } else {
        format!("'{font}', {system}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_system() {
        let a = AppearanceConfig::default();
        assert_eq!(a.parsed_mode(), Mode::System);
        assert_eq!(a.theme, "kitchen");
        assert_eq!(a.clone().validated(), a);
    }

    #[test]
    fn every_mode_and_theme_survives_validation() {
        for m in MODES {
            for t in THEMES {
                let a = AppearanceConfig { mode: m.to_string(), theme: t.id.into(), ..Default::default() }.validated();
                assert_eq!((a.mode.as_str(), a.theme.as_str()), (*m, t.id));
            }
        }
    }

    #[test]
    fn case_and_spaces_are_forgiven() {
        let a = AppearanceConfig { mode: " Dark ".into(), theme: "NORD".into(), ui_font: "  Inter  ".into(), ..Default::default() }.validated();
        assert_eq!((a.mode.as_str(), a.theme.as_str(), a.ui_font.as_str()), ("dark", "nord", "Inter"));
    }

    #[test]
    fn unknown_values_fall_back() {
        let a = AppearanceConfig { mode: "sepia".into(), theme: "../etc".into(), font_size: 3, ..Default::default() }.validated();
        assert_eq!(a.mode, "system");
        assert_eq!(a.theme, "kitchen");
        assert_eq!(a.font_size, 10);
    }

    #[test]
    fn font_names_cannot_escape_css() {
        for bad in ["a;b", "x'}", "\"Inter\"", "a{b}", "</style>", "url(x)", "a\\b", "a\nb", &"x".repeat(65)] {
            assert!(!valid_font(bad), "{bad:?}");
            let a = AppearanceConfig { ui_font: bad.into(), mono_font: bad.into(), ..Default::default() }.validated();
            assert_eq!((a.ui_font.as_str(), a.mono_font.as_str()), ("", ""), "{bad:?}");
        }
        for good in ["Inter", "JetBrains Mono", "Noto Sans JP", "Source_Code-Pro 2.0", "思源黑体"] {
            assert!(valid_font(good), "{good:?}");
        }
    }

    #[test]
    fn stacks_use_presets_names_or_the_system_font() {
        let a = AppearanceConfig { ui_font: "serif".into(), mono_font: "Iosevka".into(), ..Default::default() };
        assert!(a.ui_stack().starts_with("Charter"));
        assert!(a.mono_stack().starts_with("'Iosevka', ui-monospace"));
        assert_eq!(AppearanceConfig::default().ui_stack(), UI_FONTS[0].1);
        assert_eq!(AppearanceConfig { mono_font: "system".into(), ..Default::default() }.validated().mono_font, "");
    }

    #[test]
    fn a_missing_or_broken_section_never_breaks_the_settings() {
        let c: crate::Config = serde_json::from_str(r#"{"appearance": {"mode": 7}}"#).unwrap_or_default();
        assert_eq!(c.appearance, AppearanceConfig::default());
        let c: crate::Config = serde_json::from_str(r#"{"appearance": {"theme": "forest"}}"#).unwrap();
        assert_eq!(c.validated().appearance.theme, "forest");
    }

    #[test]
    fn the_init_script_carries_validated_data_only() {
        let a = AppearanceConfig { theme: "</script><script>alert(1)".into(), ui_font: "x');alert(1);('".into(), ..Default::default() };
        let js = a.init_script("linux", 0);
        assert!(!js.contains("alert"), "{js}");
        assert!(js.contains(r#"root.dataset.os = "linux""#));
        assert!(js.contains(r#""theme":"kitchen""#));
    }
}
