//! {{Title}}: say in one line what this module does.
//!
//! Its settings are `{{camelName}}` in the app's settings (Settings → {{Title}}).
//!
//! ```
//! use desktop_core::{{name}}::{{{Name}}Config, greet};
//!
//! let config = {{Name}}Config { enabled: true, ..{{Name}}Config::default() };
//! assert_eq!(greet(&config, "world"), Some("Hello, world!".to_string()));
//! ```

use serde::{Deserialize, Serialize};

/// Settings for this module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct {{Name}}Config {
    /// Turn the module on.
    pub enabled: bool,
    // kitchen:fields (`kitchen generate setting {{name}}.<key> <type>` adds fields above this line)
}

impl Default for {{Name}}Config {
    fn default() -> Self {
        Self {
            enabled: false,
            // kitchen:defaults
        }
    }
}

impl {{Name}}Config {
    /// Clamp values into safe ranges (called whenever settings are loaded or changed).
    pub fn validated(self) -> Self {
        self
    }
}

/// An example to replace: a greeting when the module is on.
pub fn greet(config: &{{Name}}Config, who: &str) -> Option<String> {
    config.enabled.then(|| format!("Hello, {who}!"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default() {
        assert_eq!(greet(&{{Name}}Config::default(), "x"), None);
    }
}
