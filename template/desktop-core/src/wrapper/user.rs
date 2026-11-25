//! What the end user may change in Preferences, and nothing more.
//!
//! The wrapper's settings are the **developer's**: the `wrapper` section of `config/`,
//! compiled into the app ([`WrapperConfig`]). They are read-only at run time. The developer
//! names the few fields the user may change in [`WrapperConfig::user_editable`] (by default
//! [`DEFAULT_EDITABLE`]: the release shortcut, zoom, mute and "start captured"); only those
//! show in Preferences, and [`UserOverrides::change`] refuses every other field, so the
//! check holds in Rust whatever the page sends.
//!
//! The user's side is [`UserWrapper`], saved as `config.wrapper` in the user's
//! `settings.json`: the overrides ([`UserOverrides`], one optional value per field that can
//! ever be editable) and, apart from them, the certificate pinned on first use
//! ([`UserWrapper::learned_pin`]). [`UserWrapper::merge`] lays the allowed overrides over
//! the developer's settings; an override the developer did not allow is ignored, even when
//! it is in the file.
//!
//! # Example
//!
//! ```
//! use desktop_core::wrapper::{ChangeError, UserWrapper, WrapperConfig};
//! use serde_json::json;
//!
//! let baked = WrapperConfig { url: "https://pikvm.local/".into(), ..WrapperConfig::default() };
//! let mut user = UserWrapper::default();
//!
//! // Zoom is on the default allowlist: accepted, and merged over the developer's settings.
//! user.overrides = user.overrides.change(&baked, &json!({"zoom": 1.5})).unwrap();
//! assert_eq!(user.merge(&baked).zoom, 1.5);
//!
//! // The address is the developer's: refused, whatever the page sends.
//! let e = user.overrides.change(&baked, &json!({"url": "https://evil.example/"})).unwrap_err();
//! assert!(matches!(e, ChangeError::NotEditable(ref f) if f == "url"));
//! assert_eq!(user.merge(&baked).url, "https://pikvm.local/");
//! ```

use super::WrapperConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A wrapper setting the developer may let the user change (its camelCase key in
/// `userEditable`). The address, TLS, the pin, headers, auto-fill, the User-Agent, browser
/// features and the proxy are never on this list: they stay as built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editable {
    /// `captureKeys`: send system keys to the page.
    CaptureKeys,
    /// `startCaptured`: capture as soon as the window has focus.
    StartCaptured,
    /// `releaseShortcut`: the shortcut that gives the keys back.
    ReleaseShortcut,
    /// `zoom`: page zoom.
    Zoom,
    /// `muted`: start with the page's sound muted.
    Muted,
    /// `microphone`: let the site use the microphone.
    Microphone,
    /// `camera`: let the site use the camera.
    Camera,
    /// `screen`: let the site share the screen.
    Screen,
}

/// The allowlist when the developer sets none: `["releaseShortcut", "zoom", "muted",
/// "startCaptured"]`.
pub const DEFAULT_EDITABLE: [Editable; 4] = [Editable::ReleaseShortcut, Editable::Zoom, Editable::Muted, Editable::StartCaptured];

impl Editable {
    /// Every field that can be put on the allowlist.
    pub const ALL: [Editable; 8] = [
        Editable::CaptureKeys,
        Editable::StartCaptured,
        Editable::ReleaseShortcut,
        Editable::Zoom,
        Editable::Muted,
        Editable::Microphone,
        Editable::Camera,
        Editable::Screen,
    ];

    /// The camelCase key, as in `config/settings.json` and in Preferences.
    pub fn key(self) -> &'static str {
        match self {
            Editable::CaptureKeys => "captureKeys",
            Editable::StartCaptured => "startCaptured",
            Editable::ReleaseShortcut => "releaseShortcut",
            Editable::Zoom => "zoom",
            Editable::Muted => "muted",
            Editable::Microphone => "microphone",
            Editable::Camera => "camera",
            Editable::Screen => "screen",
        }
    }

    /// The field for a key, if it can ever be editable.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.key() == key)
    }

    /// The field's value in `cfg`, as JSON.
    pub fn read(self, cfg: &WrapperConfig) -> Value {
        match self {
            Editable::CaptureKeys => cfg.capture_keys.into(),
            Editable::StartCaptured => cfg.start_captured.into(),
            Editable::ReleaseShortcut => cfg.release_shortcut.clone().into(),
            Editable::Zoom => cfg.zoom.into(),
            Editable::Muted => cfg.muted.into(),
            Editable::Microphone => cfg.microphone.into(),
            Editable::Camera => cfg.camera.into(),
            Editable::Screen => cfg.screen.into(),
        }
    }
}

/// Why [`UserOverrides::change`] refused a change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChangeError {
    /// The field is the developer's (not in `userEditable`, or no setting at all).
    #[error("{0} is set by the app's developer and cannot be changed here")]
    NotEditable(String),
    /// The value has the wrong type, or the request is not an object.
    #[error("{0}")]
    Invalid(String),
}

/// The user's changes: one optional value per field that can ever be editable. `None`
/// keeps the developer's value. Only the fields on the allowlist are ever applied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct UserOverrides {
    /// `captureKeys`.
    pub capture_keys: Option<bool>,
    /// `startCaptured`.
    pub start_captured: Option<bool>,
    /// `releaseShortcut`.
    pub release_shortcut: Option<String>,
    /// `zoom`.
    pub zoom: Option<f64>,
    /// `muted`.
    pub muted: Option<bool>,
    /// `microphone`.
    pub microphone: Option<bool>,
    /// `camera`.
    pub camera: Option<bool>,
    /// `screen`.
    pub screen: Option<bool>,
}

impl UserOverrides {
    /// `baked` with the overrides the developer allowed laid over it; others are ignored.
    pub fn apply(&self, baked: &WrapperConfig) -> WrapperConfig {
        let mut c = baked.clone();
        let allowed = |f| baked.is_editable(f);
        let put = |f: Editable, field: &mut bool, v: Option<bool>| {
            if let Some(v) = v.filter(|_| allowed(f)) {
                *field = v;
            }
        };
        put(Editable::CaptureKeys, &mut c.capture_keys, self.capture_keys);
        put(Editable::StartCaptured, &mut c.start_captured, self.start_captured);
        put(Editable::Muted, &mut c.muted, self.muted);
        put(Editable::Microphone, &mut c.microphone, self.microphone);
        put(Editable::Camera, &mut c.camera, self.camera);
        put(Editable::Screen, &mut c.screen, self.screen);
        if let Some(s) = self.release_shortcut.as_ref().filter(|_| allowed(Editable::ReleaseShortcut)) {
            c.release_shortcut = s.clone();
        }
        if let Some(z) = self.zoom.filter(|_| allowed(Editable::Zoom)) {
            c.zoom = z;
        }
        c
    }

    /// These overrides with `changes` (a JSON object of camelCase keys) applied. Every key
    /// must be on `baked`'s allowlist, else nothing changes and the first such key is named
    /// ([`ChangeError::NotEditable`]); `null` resets a field to the developer's value. The
    /// values are not range-checked here: [`UserWrapper::try_merge`] does that.
    pub fn change(&self, baked: &WrapperConfig, changes: &Value) -> Result<UserOverrides, ChangeError> {
        let Value::Object(map) = changes else {
            return Err(ChangeError::Invalid("the changes are not an object".into()));
        };
        let mut out = self.clone();
        for (key, v) in map {
            let Some(f) = Editable::from_key(key).filter(|f| baked.is_editable(*f)) else {
                return Err(ChangeError::NotEditable(key.clone()));
            };
            out.set(f, v)?;
        }
        Ok(out)
    }

    fn set(&mut self, f: Editable, v: &Value) -> Result<(), ChangeError> {
        let bad = || ChangeError::Invalid(format!("{}: wrong type", f.key()));
        let flag = |v: &Value| if v.is_null() { Ok(None) } else { v.as_bool().map(Some).ok_or_else(bad) };
        match f {
            Editable::CaptureKeys => self.capture_keys = flag(v)?,
            Editable::StartCaptured => self.start_captured = flag(v)?,
            Editable::Muted => self.muted = flag(v)?,
            Editable::Microphone => self.microphone = flag(v)?,
            Editable::Camera => self.camera = flag(v)?,
            Editable::Screen => self.screen = flag(v)?,
            Editable::ReleaseShortcut => {
                self.release_shortcut = if v.is_null() { None } else { Some(v.as_str().ok_or_else(bad)?.trim().to_string()) }
            }
            Editable::Zoom => self.zoom = if v.is_null() { None } else { Some(v.as_f64().ok_or_else(bad)?) },
        }
        Ok(())
    }

    /// The values of `cfg`'s editable fields, for Preferences (`{key: value}`).
    pub fn shown(cfg: &WrapperConfig) -> Map<String, Value> {
        cfg.editable().map(|f| (f.key().to_string(), f.read(cfg))).collect()
    }
}

/// The user's part of the wrapper, saved as `config.wrapper` in `settings.json`. An older
/// file that holds a whole [`WrapperConfig`] there reads as no overrides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct UserWrapper {
    /// The user's changes to the editable fields.
    pub overrides: UserOverrides,
    /// The certificate pinned on first use (hex SHA-256; empty: none yet). Used only when
    /// the developer turned `insecureTls` on and pinned no certificate (see
    /// [`WrapperConfig::learns_pin`]); "Forget" in Preferences clears it.
    pub learned_pin: String,
}

impl UserWrapper {
    /// The settings in force: `baked` with the allowed overrides and the learned pin, or an
    /// error listing what does not pass [`WrapperConfig::check`].
    pub fn try_merge(&self, baked: &WrapperConfig) -> Result<WrapperConfig, Vec<String>> {
        let mut c = self.overrides.apply(baked);
        if baked.learns_pin() {
            c.pinned_sha256 = self.learned_pin.clone();
        }
        c.check().map(|_| c)
    }

    /// The settings in force. When the user's part makes them invalid, the developer's
    /// settings are used as built (with the learned pin, when it is valid).
    pub fn merge(&self, baked: &WrapperConfig) -> WrapperConfig {
        match self.try_merge(baked) {
            Ok(c) => c,
            Err(e) => {
                if baked.check().is_ok() {
                    log::warn!("your wrapper changes are ignored: {}", e.join("; "));
                }
                let pin_only = UserWrapper { overrides: UserOverrides::default(), learned_pin: self.learned_pin.clone() };
                pin_only.try_merge(baked).unwrap_or_else(|_| baked.clone())
            }
        }
    }

    /// Drop values that can never be valid (a zoom out of range, a shortcut that does not
    /// parse, a pin that is not a SHA-256), so a hand-edited file cannot break the app.
    pub fn validated(mut self) -> Self {
        let o = &mut self.overrides;
        o.zoom = o.zoom.filter(|z| (0.25..=5.0).contains(z));
        o.release_shortcut = o.release_shortcut.take().filter(|s| s.parse::<crate::keymap::Shortcut>().is_ok());
        if !self.learned_pin.is_empty() {
            self.learned_pin = super::pin::normalise(&self.learned_pin).unwrap_or_default();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn baked() -> WrapperConfig {
        WrapperConfig { url: "https://h.example/".into(), ..WrapperConfig::default() }
    }

    #[test]
    fn the_default_allowlist() {
        let c = baked();
        let keys: Vec<_> = c.editable().map(Editable::key).collect();
        assert_eq!(keys, ["releaseShortcut", "zoom", "muted", "startCaptured"]);
        assert_eq!(serde_json::to_value(&c).unwrap()["userEditable"], json!(["releaseShortcut", "zoom", "muted", "startCaptured"]));
        for f in Editable::ALL {
            assert_eq!(Editable::from_key(f.key()), Some(f));
        }
        assert_eq!(Editable::from_key("url"), None);
    }

    #[test]
    fn fields_the_developer_owns_are_refused() {
        let c = baked();
        let o = UserOverrides::default();
        for key in ["url", "insecureTls", "pinnedSha256", "headers", "autofill", "userAgent", "features", "proxy", "userEditable"] {
            let e = o.change(&c, &json!({ key: true })).unwrap_err();
            assert_eq!(e, ChangeError::NotEditable(key.into()), "{key}");
        }
        // Can be editable, but this app does not allow it.
        assert_eq!(o.change(&c, &json!({"microphone": true})).unwrap_err(), ChangeError::NotEditable("microphone".into()));
        // One bad key refuses the whole request.
        assert!(o.change(&c, &json!({"zoom": 2.0, "url": "https://x.example/"})).is_err());
        assert!(matches!(o.change(&c, &json!([1])), Err(ChangeError::Invalid(_))));
        assert!(matches!(o.change(&c, &json!({"zoom": "big"})), Err(ChangeError::Invalid(_))));
    }

    #[test]
    fn allowed_fields_change_and_null_resets() {
        let c = WrapperConfig { user_editable: vec!["zoom".into(), "microphone".into()], ..baked() };
        let o = UserOverrides::default().change(&c, &json!({"zoom": 2.0, "microphone": true})).unwrap();
        let m = UserWrapper { overrides: o.clone(), ..UserWrapper::default() }.merge(&c);
        assert!(m.microphone && m.zoom == 2.0);
        let o = o.change(&c, &json!({"zoom": null})).unwrap();
        assert_eq!(o.zoom, None);
        assert_eq!(UserOverrides::shown(&m), json!({"zoom": 2.0, "microphone": true}).as_object().unwrap().clone());
    }

    #[test]
    fn overrides_outside_the_allowlist_are_ignored_on_merge() {
        // e.g. written by hand, or allowed by an older build.
        let saved: UserWrapper = serde_json::from_value(json!({"overrides": {"microphone": true, "zoom": 3.0}})).unwrap();
        let m = saved.merge(&baked());
        assert!(!m.microphone);
        assert_eq!(m.zoom, 3.0);
        let none = WrapperConfig { user_editable: vec![], ..baked() };
        assert_eq!(saved.merge(&none).zoom, 1.0);
    }

    #[test]
    fn invalid_changes_fail_try_merge_and_fall_back_on_merge() {
        let c = baked();
        let o = UserOverrides::default().change(&c, &json!({"zoom": 9.0, "releaseShortcut": "K"})).unwrap();
        let u = UserWrapper { overrides: o, ..UserWrapper::default() };
        assert_eq!(u.try_merge(&c).unwrap_err().len(), 2);
        assert_eq!(u.merge(&c), c);
        let v = u.validated();
        assert_eq!(v.overrides, UserOverrides::default());
    }

    #[test]
    fn the_learned_pin_is_kept_apart_and_only_used_without_a_baked_pin() {
        let fp = "ab".repeat(32);
        let u = UserWrapper { learned_pin: fp.clone(), ..UserWrapper::default() };
        assert_eq!(u.merge(&baked()).pinned_sha256, "", "no insecureTls: no pin");
        let tofu = WrapperConfig { insecure_tls: true, ..baked() };
        assert!(tofu.learns_pin());
        assert_eq!(u.merge(&tofu).pinned_sha256, fp);
        let fixed = WrapperConfig { pinned_sha256: "cd".repeat(32), ..tofu };
        assert!(!fixed.learns_pin());
        assert_eq!(u.merge(&fixed).pinned_sha256, "cd".repeat(32), "the developer's pin wins");
        assert_eq!(UserWrapper { learned_pin: "nope".into(), ..u }.validated().learned_pin, "");
    }

    #[test]
    fn an_old_whole_config_reads_as_no_overrides() {
        let old = serde_json::to_value(WrapperConfig { url: "https://other.example/".into(), zoom: 2.0, ..WrapperConfig::default() }).unwrap();
        let u: UserWrapper = serde_json::from_value(old).unwrap();
        assert_eq!(u, UserWrapper::default());
    }
}
