//! Translations (feature `i18n`): every text the app shows, in the user's language.
//!
//! The messages live in [Fluent](https://projectfluent.org) files, one per locale:
//! `locales/<code>/app.ftl` at the app's root (`en` is the source; every other locale
//! translates it). The same files serve every part of the app:
//!
//! - the backend: [`tr`] and the [`t!`](crate::t) macro, for native menus, the tray, action
//!   results, the test email and error messages;
//! - the React UI: Vite embeds the files at build time (`ui-react/src/i18n.js`, `@fluent/bundle`);
//! - the plain UI: the `i18n_bundle` command hands it the messages ([`Catalog::bundle`]).
//!
//! `build.rs` embeds every `locales/*/app.ftl` it finds, so a new locale (`kitchen generate
//! locale fr`) needs no code. A message missing from a locale falls back to `en`, and a
//! message missing everywhere shows its id, so nothing ever fails to render.
//!
//! # Locale
//!
//! The settings' `ui.language` ([`UiConfig`]) is `system` (the OS language, read with
//! [sys-locale](https://docs.rs/sys-locale)) or a locale code. [`set_locale`] resolves it
//! to the closest locale the app has ([`negotiate`]: `de-AT` uses `de`, `pt-BR` uses `pt`
//! when there is no `pt-BR`, anything else uses `en`). [`direction`] says whether it is
//! written right to left, which the UI puts on `<html dir>`.
//!
//! # Plurals, numbers and dates
//!
//! Plurals use Fluent's select expressions with the CLDR plural rules of each language
//! (`intl_pluralrules`, built into fluent-bundle): Arabic has six forms, German two. Numbers
//! are formatted for the locale (decimal and group separators, digits), with Fluent's
//! `NUMBER()` options (`minimumFractionDigits`, `maximumFractionDigits`, `useGrouping`,
//! `style: "percent"`). `DATETIME($ms, dateStyle: "long", timeStyle: "short")` formats a
//! time (milliseconds since 1970, or an RFC 3339 text) in the local time zone with
//! [jiff](https://docs.rs/jiff), from a small table of month names and patterns per language;
//! in the React UI the same message uses the browser's `Intl`.
//!
//! # Why Fluent, and not ICU4X
//!
//! fluent-bundle and unic-langid are pure Rust and small (about ten crates, none with C code),
//! and Fluent has a JavaScript implementation that reads the same files, so the backend and
//! the UI share one format and one set of files. ICU4X would format numbers and dates for
//! every CLDR locale, but it adds a large data set (or a data provider to build) for what the
//! app needs: separators, digits, month names and a few patterns, which [`format_number`] and
//! [`format_date`] cover for the common languages (others get ISO dates and `1,234.5`).
//!
//! # Example
//!
//! ```
//! use desktop_core::i18n::{Catalog, direction};
//!
//! let catalog = Catalog::new(&[
//!     ("en", "files = { $count ->\n    [one] One file\n   *[other] { $count } files\n}\nhello = Hello, { $name }!"),
//!     ("de", "files = { $count ->\n    [one] Eine Datei\n   *[other] { $count } Dateien\n}"),
//! ]);
//! let mut args = desktop_core::i18n::FluentArgs::new();
//! args.set("count", 1234);
//! assert_eq!(catalog.format("de", "files", Some(&args)), "1.234 Dateien");
//! args.set("count", 1);
//! assert_eq!(catalog.format("en", "files", Some(&args)), "One file");
//! args.set("name", "Ann");
//! // Not translated into German yet: the English text, not an error.
//! assert_eq!(catalog.format("de", "hello", Some(&args)), "Hello, Ann!");
//! assert_eq!(catalog.negotiate("de-AT"), "de");
//! assert_eq!(catalog.negotiate("fr"), "en");
//! assert_eq!(direction("ar"), "rtl");
//! ```
//!
//! In the app, the built-in catalogue and the user's language are global:
//!
//! ```
//! use desktop_core::{i18n, t};
//!
//! i18n::set_locale("en");
//! assert_eq!(t!("action-quit"), "Quit");
//! assert_eq!(t!("no-such-message"), "no-such-message");
//! ```

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::types::{FluentNumber, FluentNumberStyle};
use fluent_bundle::FluentResource;
use fluent_syntax::ast;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::{FluentArgs, FluentValue};

/// The source locale: every message is written here first, and every other locale falls
/// back to it.
pub const SOURCE: &str = "en";

/// The locales built into this app as `(code, contents of app.ftl)`, the source locale
/// first. `build.rs` makes this list from `locales/*/app.ftl`.
pub static LOCALES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// The language settings (`ui` in [`crate::Config`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct UiConfig {
    /// `system` (the OS language, the default) or a locale code such as `de` or `ar`.
    pub language: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { language: "system".into() }
    }
}

impl UiConfig {
    /// `system`, or a well-formed locale code (letters, digits, `-` and `_`, at most 35
    /// characters); anything else becomes `system`.
    pub fn validated(mut self) -> Self {
        let l = self.language.trim();
        let ok = !l.is_empty() && l.len() <= 35 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        self.language = if ok { l.to_string() } else { "system".into() };
        self
    }
}

/// One locale's messages, parsed: what [`Catalog`] formats with.
struct Locale {
    code: String,
    source: String,
    bundle: FluentBundle<FluentResource>,
    /// Parse errors (the rest of the file still works).
    errors: Vec<String>,
}

/// A set of locales and their messages, with fallback to the source locale.
///
/// The app uses the built-in one ([`catalog`], from [`LOCALES`]); tests and tools can build
/// their own with [`Catalog::new`].
pub struct Catalog {
    locales: Vec<Locale>,
}

impl Catalog {
    /// Parse each `(code, ftl)`. The first is the source locale (normally `en`). A file with
    /// syntax errors keeps the messages that parsed; [`Catalog::problems`] lists the rest.
    pub fn new(sources: &[(&str, &str)]) -> Self {
        let locales = sources
            .iter()
            .map(|(code, ftl)| {
                let lang: LanguageIdentifier = code.parse().unwrap_or_default();
                let (resource, mut errors) = match FluentResource::try_new(ftl.to_string()) {
                    Ok(r) => (r, Vec::new()),
                    Err((r, errs)) => (r, errs.iter().map(|e| format!("{code}/app.ftl: {e:?}")).collect()),
                };
                let mut bundle = FluentBundle::new_concurrent(vec![lang]);
                // Unicode isolation marks around placeables keep a file name or number in
                // its own direction; only right-to-left text needs them.
                bundle.set_use_isolating(direction(code) == "rtl");
                bundle.set_formatter(Some(number_formatter));
                let _ = bundle.add_builtins();
                let date_locale = code.to_string();
                let _ = bundle.add_function("DATETIME", move |positional, named| datetime(&date_locale, positional, named));
                if let Err(errs) = bundle.add_resource(resource) {
                    errors.extend(errs.iter().map(|e| format!("{code}/app.ftl: {e}")));
                }
                Locale { code: code.to_string(), source: ftl.to_string(), bundle, errors }
            })
            .collect();
        Self { locales }
    }

    /// The locale codes, the source locale first.
    pub fn codes(&self) -> Vec<&str> {
        self.locales.iter().map(|l| l.code.as_str()).collect()
    }

    /// Syntax errors and duplicate messages, one line each (empty when every file is fine).
    pub fn problems(&self) -> Vec<String> {
        self.locales.iter().flat_map(|l| l.errors.iter().cloned()).collect()
    }

    fn get(&self, code: &str) -> Option<&Locale> {
        self.locales.iter().find(|l| l.code.eq_ignore_ascii_case(code))
    }

    /// The closest locale this catalogue has for `requested` (a BCP 47 tag such as `de-AT`
    /// or a POSIX one such as `de_AT.UTF-8`): the same tag, else the same language, else the
    /// source locale.
    pub fn negotiate(&self, requested: &str) -> &str {
        let want = requested.split(['.', '@']).next().unwrap_or("").replace('_', "-");
        if let Some(l) = self.get(&want) {
            return &l.code;
        }
        let lang = |tag: &str| tag.split('-').next().unwrap_or("").to_ascii_lowercase();
        let want_lang = lang(&want);
        if let Some(l) = self.locales.iter().find(|l| !want_lang.is_empty() && lang(&l.code) == want_lang) {
            return &l.code;
        }
        self.locales.first().map(|l| l.code.as_str()).unwrap_or(SOURCE)
    }

    /// Format message `id` (or `id.attribute`) in `locale`, falling back to the source
    /// locale, then to the id itself. Arguments fill `{ $name }`; numbers choose plural forms.
    pub fn format(&self, locale: &str, id: &str, args: Option<&FluentArgs>) -> String {
        let (msg_id, attr) = match id.split_once('.') {
            Some((m, a)) => (m, Some(a)),
            None => (id, None),
        };
        let chain = [self.get(self.negotiate(locale)), self.locales.first()];
        for l in chain.into_iter().flatten() {
            let Some(msg) = l.bundle.get_message(msg_id) else { continue };
            let pattern = match attr {
                Some(a) => msg.get_attribute(a).map(|a| a.value()),
                None => msg.value(),
            };
            if let Some(p) = pattern {
                let mut errors = Vec::new();
                let text = l.bundle.format_pattern(p, args, &mut errors).into_owned();
                if !errors.is_empty() {
                    log::debug!("i18n {}: {id}: {errors:?}", l.code);
                }
                return text;
            }
        }
        id.to_string()
    }

    /// Whether `locale` itself (not the fallback) has message `id`.
    pub fn has(&self, locale: &str, id: &str) -> bool {
        self.get(locale).is_some_and(|l| l.bundle.has_message(id))
    }

    /// Everything a UI needs to show `locale` ([`Bundle`]): its direction, the locales to
    /// pick from, the Fluent sources (the locale's own, then the source locale's), and the
    /// messages as plain JSON for the plain UI (which has no Fluent library).
    pub fn bundle(&self, locale: &str) -> Bundle {
        let code = self.negotiate(locale).to_string();
        let mut sources = Vec::new();
        let mut messages = Map::new();
        for l in [self.get(&code), self.locales.first()].into_iter().flatten() {
            if sources.iter().any(|s: &BundleSource| s.locale == l.code) {
                continue;
            }
            sources.push(BundleSource { locale: l.code.clone(), ftl: l.source.clone() });
            for (id, v) in compile(&l.source) {
                messages.entry(id).or_insert(v);
            }
        }
        Bundle { dir: direction(&code).into(), available: self.available(), locale: code, sources, messages: Value::Object(messages) }
    }

    /// The locales to offer in a language picker, each named in its own language (its
    /// `language-name` message), the source locale first.
    pub fn available(&self) -> Vec<LocaleInfo> {
        self.locales
            .iter()
            .map(|l| {
                let name = l.bundle.get_message("language-name").and_then(|m| m.value()).map(|p| {
                    let mut e = Vec::new();
                    l.bundle.format_pattern(p, None, &mut e).into_owned()
                });
                LocaleInfo { code: l.code.clone(), name: name.unwrap_or_else(|| l.code.clone()), dir: direction(&l.code).into() }
            })
            .collect()
    }
}

/// A locale in the language picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LocaleInfo {
    /// The code, e.g. `de`.
    pub code: String,
    /// Its name in its own language, e.g. `Deutsch`.
    pub name: String,
    /// `ltr` or `rtl`.
    pub dir: String,
}

/// One Fluent file in a [`Bundle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleSource {
    /// The locale code.
    pub locale: String,
    /// The file's text.
    pub ftl: String,
}

/// What the `i18n_bundle` command returns: one locale, ready for a UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Bundle {
    /// The locale used (the closest one to what was asked for).
    pub locale: String,
    /// `ltr` or `rtl`, for `<html dir>`.
    pub dir: String,
    /// Every locale to choose from.
    pub available: Vec<LocaleInfo>,
    /// The Fluent files, most specific first (the locale, then the source locale).
    pub sources: Vec<BundleSource>,
    /// Each message (and `message.attribute`) as text, or as parts for messages with
    /// variables and plurals: `{"parts": ["Added ", {"var": "count"}, " files"]}`,
    /// `{"select": {"var": "count"}, "variants": {"one": ..., "other": ...}, "default": "other"}`.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub messages: Value,
}

/// The built-in catalogue ([`LOCALES`]), parsed once.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let c = Catalog::new(LOCALES);
        for p in c.problems() {
            log::warn!("translations: {p}");
        }
        c
    })
}

static CURRENT: RwLock<String> = RwLock::new(String::new());

/// The OS language as a BCP 47 tag (`en-US`), or `en` when the OS does not say.
pub fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| SOURCE.into())
}

/// The closest built-in locale to `requested` (see [`Catalog::negotiate`]); `system` or an
/// empty text asks the OS.
pub fn negotiate(requested: &str) -> String {
    let r = requested.trim();
    let r = if r.is_empty() || r.eq_ignore_ascii_case("system") { system_locale() } else { r.to_string() };
    catalog().negotiate(&r).to_string()
}

/// Use `requested` (`system` or a locale code, as in [`UiConfig::language`]) from now on;
/// returns the locale chosen.
pub fn set_locale(requested: &str) -> String {
    let code = negotiate(requested);
    *CURRENT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = code.clone();
    code
}

/// The locale in use ([`set_locale`]; the OS language until it is called).
pub fn locale() -> String {
    let current = CURRENT.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    if current.is_empty() {
        set_locale("system")
    } else {
        current
    }
}

/// Message `id` (or `id.attribute`) in the current locale; see [`Catalog::format`]. The
/// [`t!`](crate::t) macro builds the arguments.
pub fn tr(id: &str, args: Option<&FluentArgs>) -> String {
    catalog().format(&locale(), id, args)
}

/// [`tr`] with arguments set by `fill` (what [`t!`](crate::t) expands to).
pub fn tr_with<'a>(id: &str, fill: impl FnOnce(&mut FluentArgs<'a>)) -> String {
    let mut args = FluentArgs::new();
    fill(&mut args);
    tr(id, Some(&args))
}

/// [`Catalog::bundle`] of the built-in catalogue; `system` asks the OS.
pub fn bundle(requested: &str) -> Bundle {
    catalog().bundle(&negotiate(requested))
}

/// A translated message: `t!("id")`, or with arguments `t!("added-files", count = 3, folder = path)`.
///
/// ```
/// desktop_core::i18n::set_locale("en");
/// assert_eq!(desktop_core::t!("action-done-paused"), "Background work paused.");
/// ```
#[macro_export]
macro_rules! t {
    ($id:expr) => {
        $crate::i18n::tr($id, None)
    };
    ($id:expr, $($key:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr_with($id, |args| {
            $(args.set(stringify!($key), $value);)+
        })
    };
}

/// `rtl` for languages written right to left (Arabic, Hebrew, Persian, Urdu, ...), else `ltr`.
pub fn direction(code: &str) -> &'static str {
    const RTL: &[&str] = &["ar", "arc", "ckb", "dv", "fa", "he", "ks", "ku-arab", "ps", "sd", "ug", "ur", "yi"];
    let lower = code.to_ascii_lowercase().replace('_', "-");
    let lang = lower.split('-').next().unwrap_or("");
    if RTL.contains(&lang) || lower.contains("-arab") {
        "rtl"
    } else {
        "ltr"
    }
}

// ---- Numbers -------------------------------------------------------------------------

/// How a language writes numbers: separators and digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NumberStyle {
    decimal: &'static str,
    group: &'static str,
    /// Native digits 0-9, or `None` for ASCII digits.
    digits: Option<[char; 10]>,
}

const ARABIC_INDIC: [char; 10] = ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'];
const PERSIAN: [char; 10] = ['۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹'];

fn number_style(code: &str) -> NumberStyle {
    let lower = code.to_ascii_lowercase().replace('_', "-");
    let lang = lower.split('-').next().unwrap_or("");
    let latn = lower.contains("-latn") || ["ar-ma", "ar-dz", "ar-tn", "ar-ly"].iter().any(|p| lower.starts_with(p));
    let style = |decimal, group| NumberStyle { decimal, group, digits: None };
    match lang {
        "ar" if !latn => NumberStyle { decimal: "٫", group: "٬", digits: Some(ARABIC_INDIC) },
        "fa" => NumberStyle { decimal: "٫", group: "٬", digits: Some(PERSIAN) },
        "de" if lower.ends_with("-ch") || lower.ends_with("-li") => style(".", "’"),
        "de" | "es" | "it" | "nl" | "pt" | "id" | "tr" | "da" | "el" | "ro" | "hr" | "sl" | "sr" | "vi" | "ca" | "is" => style(",", "."),
        "fr" => style(",", "\u{202f}"),
        "ru" | "pl" | "cs" | "sk" | "sv" | "nb" | "no" | "nn" | "fi" | "uk" | "hu" | "bg" | "lt" | "lv" | "et" | "kk" | "be" => {
            style(",", "\u{a0}")
        }
        _ => style(".", ","),
    }
}

/// The locale a bundle formats numbers for, kept in fluent's per-locale memoizer (the
/// formatter fluent-bundle calls is a plain `fn` and gets the locale only through it).
struct NumberFormat(NumberStyle);

impl intl_memoizer::Memoizable for NumberFormat {
    type Args = ();
    type Error = ();
    fn construct(lang: LanguageIdentifier, _: ()) -> Result<Self, ()> {
        Ok(NumberFormat(number_style(&lang.to_string())))
    }
}

fn number_formatter(value: &FluentValue, intls: &intl_memoizer::concurrent::IntlLangMemoizer) -> Option<String> {
    let FluentValue::Number(n) = value else { return None };
    intls.with_try_get::<NumberFormat, _, _>((), |f| render_number(n, f.0)).ok()
}

/// `n` written for locale `code`: `format_number("de", 1234.5)` is `1.234,5`.
///
/// ```
/// use desktop_core::i18n::format_number;
/// assert_eq!(format_number("en", 1234567.891), "1,234,567.891");
/// assert_eq!(format_number("de", 1234.5), "1.234,5");
/// assert_eq!(format_number("ar", 1234.5), "١٬٢٣٤٫٥");
/// ```
pub fn format_number(code: &str, n: f64) -> String {
    render_number(&FluentNumber::from(n), number_style(code))
}

fn render_number(n: &FluentNumber, style: NumberStyle) -> String {
    let o = &n.options;
    let mut value = n.value;
    let percent = o.style == FluentNumberStyle::Percent;
    if percent {
        value *= 100.0;
    }
    let min_fd = o.minimum_fraction_digits.unwrap_or(0);
    let max_fd = o.maximum_fraction_digits.unwrap_or(if percent { 0 } else { 3 }).max(min_fd).min(20);
    let mut text = format!("{:.*}", max_fd, value.abs());
    if let Some(dot) = text.find('.') {
        // Drop trailing zeros past the minimum.
        let keep = text.trim_end_matches('0').len().max(dot + 1 + min_fd);
        text.truncate(keep);
        if text.ends_with('.') {
            text.pop();
        }
    }
    let (int, frac) = match text.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (text.clone(), None),
    };
    let int = format!("{int:0>width$}", width = o.minimum_integer_digits.unwrap_or(1));
    let mut out = String::new();
    if value < 0.0 && (value.abs() * 10f64.powi(max_fd as i32)).round() != 0.0 {
        out.push('-');
    }
    let len = int.len();
    for (i, c) in int.chars().enumerate() {
        if o.use_grouping && i > 0 && (len - i) % 3 == 0 {
            out.push_str(style.group);
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push_str(style.decimal);
        out.push_str(&f);
    }
    if percent {
        out.push(if style.digits.is_some() { '٪' } else { '%' });
    }
    localize_digits(&out, style)
}

fn localize_digits(text: &str, style: NumberStyle) -> String {
    match style.digits {
        Some(d) => text.chars().map(|c| c.to_digit(10).map(|v| d[v as usize]).unwrap_or(c)).collect(),
        None => text.to_string(),
    }
}

// ---- Dates -----------------------------------------------------------------------------

/// Month names and patterns of one language. Pattern tokens: `{d}` day, `{dd}` two-digit
/// day, `{M}` month number, `{MM}` two-digit month, `{MMM}` short name, `{MMMM}` name,
/// `{y}` year, `{yy}` two-digit year; times: `{H}` 0-23, `{HH}`, `{h}` 1-12, `{mm}`, `{a}` am/pm.
struct DateNames {
    lang: &'static str,
    months: [&'static str; 12],
    short_months: [&'static str; 12],
    short: &'static str,
    medium: &'static str,
    long: &'static str,
    time: &'static str,
    am_pm: [&'static str; 2],
}

const DATE_NAMES: &[DateNames] = &[
    DateNames {
        lang: "en",
        months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
        short_months: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
        short: "{M}/{d}/{yy}",
        medium: "{MMM} {d}, {y}",
        long: "{MMMM} {d}, {y}",
        time: "{h}:{mm} {a}",
        am_pm: ["AM", "PM"],
    },
    DateNames {
        lang: "de",
        months: ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"],
        short_months: ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."],
        short: "{dd}.{MM}.{yy}",
        medium: "{dd}.{MM}.{y}",
        long: "{d}. {MMMM} {y}",
        time: "{HH}:{mm}",
        am_pm: ["AM", "PM"],
    },
    DateNames {
        lang: "fr",
        months: ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"],
        short_months: ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."],
        short: "{dd}/{MM}/{y}",
        medium: "{d} {MMM} {y}",
        long: "{d} {MMMM} {y}",
        time: "{HH}:{mm}",
        am_pm: ["AM", "PM"],
    },
    DateNames {
        lang: "es",
        months: ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
        short_months: ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"],
        short: "{d}/{M}/{yy}",
        medium: "{d} {MMM} {y}",
        long: "{d} de {MMMM} de {y}",
        time: "{H}:{mm}",
        am_pm: ["a. m.", "p. m."],
    },
    DateNames {
        lang: "ar",
        months: ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"],
        short_months: ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"],
        short: "{d}\u{200f}/{M}\u{200f}/{y}",
        medium: "{dd}\u{200f}/{MM}\u{200f}/{y}",
        long: "{d} {MMMM} {y}",
        time: "{h}:{mm} {a}",
        am_pm: ["ص", "م"],
    },
];

/// Which date patterns [`format_date`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// No date (a time only).
    None,
    /// `11/24/26`, `24.11.26`.
    Short,
    /// `Nov 24, 2026`, `24.11.2026` (the default).
    Medium,
    /// `November 24, 2026`, `24. November 2026`.
    Long,
}

impl DateStyle {
    fn parse(s: &str) -> Self {
        match s {
            "short" => DateStyle::Short,
            "long" | "full" => DateStyle::Long,
            "none" => DateStyle::None,
            _ => DateStyle::Medium,
        }
    }
}

/// A local date (and time, with `time`) written for locale `code`. `ms` is milliseconds
/// since 1970 (UTC); the OS time zone decides the local day and hour. Languages without a
/// table get `2026-11-24 15:05`.
///
/// ```
/// use desktop_core::i18n::{format_date_in, DateStyle};
/// use jiff::tz::TimeZone;
///
/// let ms = 1_795_525_500_000; // 2026-11-24 13:05 UTC
/// assert_eq!(format_date_in("en", ms, DateStyle::Long, false, &TimeZone::UTC), "November 24, 2026");
/// assert_eq!(format_date_in("de", ms, DateStyle::Long, true, &TimeZone::UTC), "24. November 2026 13:05");
/// assert_eq!(format_date_in("en", ms, DateStyle::Medium, true, &TimeZone::UTC), "Nov 24, 2026 1:05 PM");
/// assert_eq!(format_date_in("ar", ms, DateStyle::Long, false, &TimeZone::UTC), "٢٤ نوفمبر ٢٠٢٦");
/// assert_eq!(format_date_in("ja", ms, DateStyle::Medium, true, &TimeZone::UTC), "2026-11-24 13:05");
/// ```
pub fn format_date_in(code: &str, ms: i64, date: DateStyle, time: bool, tz: &jiff::tz::TimeZone) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(ms) else { return ms.to_string() };
    let z = tz.to_datetime(ts);
    let lower = code.to_ascii_lowercase();
    let lang = lower.split(['-', '_']).next().unwrap_or("");
    let names = DATE_NAMES.iter().find(|n| n.lang == lang);
    let (y, mo, d, h, mi) = (z.year(), z.month() as usize, z.day(), z.hour(), z.minute());
    let fill = |pattern: &str| {
        let n = names;
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        let tokens: [(&str, String); 14] = [
            ("{MMMM}", n.map(|n| n.months[mo - 1].to_string()).unwrap_or_default()),
            ("{MMM}", n.map(|n| n.short_months[mo - 1].to_string()).unwrap_or_default()),
            ("{MM}", format!("{mo:02}")),
            ("{M}", mo.to_string()),
            ("{dd}", format!("{d:02}")),
            ("{d}", d.to_string()),
            ("{yy}", format!("{:02}", y.rem_euclid(100))),
            ("{y}", y.to_string()),
            ("{HH}", format!("{h:02}")),
            ("{H}", h.to_string()),
            ("{h}", h12.to_string()),
            ("{mm}", format!("{mi:02}")),
            ("{a}", n.map(|n| n.am_pm[usize::from(h >= 12)].to_string()).unwrap_or_default()),
            ("", String::new()),
        ];
        let mut out = pattern.to_string();
        for (k, v) in tokens.iter().filter(|(k, _)| !k.is_empty()) {
            out = out.replace(k, v);
        }
        out
    };
    let date_text = match (names, date) {
        (_, DateStyle::None) => String::new(),
        (Some(n), DateStyle::Short) => fill(n.short),
        (Some(n), DateStyle::Medium) => fill(n.medium),
        (Some(n), DateStyle::Long) => fill(n.long),
        (None, _) => format!("{y:04}-{mo:02}-{d:02}"),
    };
    let time_text = if time { fill(names.map(|n| n.time).unwrap_or("{HH}:{mm}")) } else { String::new() };
    let text = [date_text, time_text].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    localize_digits(&text, number_style(code))
}

/// [`format_date_in`] in the OS time zone.
pub fn format_date(code: &str, ms: i64, date: DateStyle, time: bool) -> String {
    format_date_in(code, ms, date, time, &jiff::tz::TimeZone::system())
}

/// Fluent's `DATETIME($when, dateStyle: "long", timeStyle: "short")`, as `Intl` reads it.
fn datetime<'a>(code: &str, positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    let ms = match positional.first() {
        Some(FluentValue::Number(n)) => n.value as i64,
        Some(FluentValue::String(s)) => match s.parse::<jiff::Timestamp>() {
            Ok(t) => t.as_millisecond(),
            Err(_) => return FluentValue::Error,
        },
        _ => return FluentValue::Error,
    };
    let opt = |k: &str| match named.get(k) {
        Some(FluentValue::String(s)) => Some(s.to_string()),
        _ => None,
    };
    let (date, time) = (opt("dateStyle"), opt("timeStyle"));
    let date_style = match (&date, &time) {
        (None, Some(_)) => DateStyle::None,
        (d, _) => DateStyle::parse(d.as_deref().unwrap_or("medium")),
    };
    FluentValue::String(format_date(code, ms, date_style, time.is_some_and(|t| t != "none")).into())
}

// ---- The plain UI's messages ---------------------------------------------------------------

/// Each message of `ftl` as JSON the plain UI's `t()` reads (see [`Bundle::messages`]).
fn compile(ftl: &str) -> Vec<(String, Value)> {
    let resource = match fluent_syntax::parser::parse_runtime(ftl) {
        Ok(r) => r,
        Err((r, _)) => r,
    };
    let mut out = Vec::new();
    for entry in &resource.body {
        if let ast::Entry::Message(m) = entry {
            if let Some(v) = &m.value {
                out.push((m.id.name.to_string(), pattern(v)));
            }
            for a in &m.attributes {
                out.push((format!("{}.{}", m.id.name, a.id.name), pattern(&a.value)));
            }
        }
    }
    out
}

fn pattern(p: &ast::Pattern<&str>) -> Value {
    let parts: Vec<Value> = p
        .elements
        .iter()
        .map(|e| match e {
            ast::PatternElement::TextElement { value } => Value::String(value.to_string()),
            ast::PatternElement::Placeable { expression } => expr(expression),
        })
        .collect();
    match parts.as_slice() {
        [] => Value::String(String::new()),
        [one] => one.clone(),
        _ => json!({ "parts": parts }),
    }
}

fn expr(e: &ast::Expression<&str>) -> Value {
    match e {
        ast::Expression::Inline(i) => inline(i),
        ast::Expression::Select { selector, variants } => {
            let mut map = BTreeMap::new();
            let mut default = String::new();
            for v in variants {
                let key = match &v.key {
                    ast::VariantKey::Identifier { name } => name.to_string(),
                    ast::VariantKey::NumberLiteral { value } => value.to_string(),
                };
                if v.default {
                    default = key.clone();
                }
                map.insert(key, pattern(&v.value));
            }
            json!({ "select": inline(selector), "variants": map, "default": default })
        }
    }
}

fn inline(i: &ast::InlineExpression<&str>) -> Value {
    match i {
        ast::InlineExpression::StringLiteral { value } => Value::String(value.to_string()),
        ast::InlineExpression::NumberLiteral { value } => Value::String(value.to_string()),
        ast::InlineExpression::VariableReference { id } => json!({ "var": id.name }),
        ast::InlineExpression::FunctionReference { id, arguments } => {
            let mut options = Map::new();
            for n in &arguments.named {
                if let ast::InlineExpression::StringLiteral { value } | ast::InlineExpression::NumberLiteral { value } = &n.value {
                    options.insert(n.name.name.to_string(), Value::String(value.to_string()));
                }
            }
            let arg = arguments.positional.first().map(inline).unwrap_or(Value::Null);
            json!({ "fn": id.name, "arg": arg, "options": options })
        }
        ast::InlineExpression::MessageReference { id, attribute } => {
            json!({ "ref": match attribute { Some(a) => format!("{}.{}", id.name, a.name), None => id.name.to_string() } })
        }
        ast::InlineExpression::TermReference { id, .. } => json!({ "term": id.name }),
        ast::InlineExpression::Placeable { expression } => expr(expression),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in locale parses, and has exactly the source locale's messages.
    #[test]
    fn built_in_locales_are_complete() {
        let c = Catalog::new(LOCALES);
        assert_eq!(c.codes().first(), Some(&SOURCE));
        assert!(c.problems().is_empty(), "{:#?}", c.problems());
        let ids = |ftl: &str| -> std::collections::BTreeSet<String> { compile(ftl).into_iter().map(|(id, _)| id).collect() };
        let source = ids(LOCALES[0].1);
        for (code, ftl) in &LOCALES[1..] {
            let these = ids(ftl);
            let missing: Vec<_> = source.difference(&these).collect();
            let extra: Vec<_> = these.difference(&source).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{code}: missing {missing:?}, extra {extra:?}");
        }
    }

    #[test]
    fn plurals_follow_each_language() {
        let ftl_en = "n = { $count ->\n    [one] one\n   *[other] other\n}";
        let ftl_ar = "n = { $count ->\n    [zero] zero\n    [one] one\n    [two] two\n    [few] few\n    [many] many\n   *[other] other\n}";
        let c = Catalog::new(&[("en", ftl_en), ("ar", ftl_ar)]);
        let form = |code: &str, n: i64| {
            let mut a = FluentArgs::new();
            a.set("count", n);
            c.format(code, "n", Some(&a))
        };
        assert_eq!([0, 1, 2, 3, 11, 100].map(|n| form("ar", n)), ["zero", "one", "two", "few", "many", "other"]);
        assert_eq!([0, 1, 2].map(|n| form("en", n)), ["other", "one", "other"]);
    }

    #[test]
    fn numbers_use_the_locale() {
        let c = Catalog::new(&[
            ("en", "n = { $n }\np = { NUMBER($n, style: \"percent\") }\nf = { NUMBER($n, minimumFractionDigits: 2) }"),
            ("de", "n = { $n }"),
            ("ar", "n = { $n }"),
        ]);
        let f = |code: &str, id: &str, n: f64| {
            let mut a = FluentArgs::new();
            a.set("n", n);
            c.format(code, id, Some(&a))
        };
        assert_eq!(f("en", "n", 1234567.5), "1,234,567.5");
        assert_eq!(f("de", "n", 1234567.5), "1.234.567,5");
        assert_eq!(f("ar", "n", 12.5), "١٢٫٥"); // a lone placeable gets no isolation marks
        assert_eq!(f("en", "p", 0.256), "26%");
        assert_eq!(f("en", "f", 3.0), "3.00");
        assert_eq!(f("en", "n", -0.5), "-0.5");
        assert_eq!(format_number("fr", 1234.5), "1\u{202f}234,5");
    }

    #[test]
    fn dates_and_times() {
        let c = Catalog::new(&[("en", "d = { DATETIME($when, dateStyle: \"long\") }\nt = { DATETIME($when, timeStyle: \"short\") }")]);
        let mut a = FluentArgs::new();
        a.set("when", "2026-11-24T13:05:00Z");
        let d = c.format("en", "d", Some(&a));
        assert!(d.starts_with("November 2"), "{d}"); // the 24th or 25th in the local time zone
        assert!(c.format("en", "t", Some(&a)).contains(':'));
        assert_eq!(format_date_in("de", 0, DateStyle::Short, false, &jiff::tz::TimeZone::UTC), "01.01.70");
    }

    #[test]
    fn negotiation_and_direction() {
        let c = Catalog::new(&[("en", ""), ("de", ""), ("pt-BR", "")]);
        assert_eq!(c.negotiate("de_DE.UTF-8"), "de");
        assert_eq!(c.negotiate("pt-br"), "pt-BR");
        assert_eq!(c.negotiate("pt-PT"), "pt-BR");
        assert_eq!(c.negotiate("zz"), "en");
        assert_eq!(c.negotiate(""), "en");
        assert_eq!(direction("he-IL"), "rtl");
        assert_eq!(direction("en"), "ltr");
        assert_eq!(UiConfig { language: "de\"><".into() }.validated().language, "system");
        assert_eq!(UiConfig { language: " ar ".into() }.validated().language, "ar");
    }

    #[test]
    fn attributes_fallback_and_syntax_errors() {
        let c = Catalog::new(&[("en", "a = A\n    .help = Help for A\nb = B"), ("de", "a = Ä\nbroken = { $x\nc = C")]);
        assert_eq!(c.format("de", "a", None), "Ä");
        assert_eq!(c.format("de", "a.help", None), "Help for A"); // not in de: falls back
        assert_eq!(c.format("de", "b", None), "B");
        assert_eq!(c.format("de", "missing", None), "missing");
        assert!(c.has("de", "c"), "messages after the error still load");
        assert_eq!(c.problems().len(), 1, "{:?}", c.problems());
    }

    #[test]
    fn bundle_compiles_messages_for_the_plain_ui() {
        let c = Catalog::new(&[
            ("en", "language-name = English\nhi = Hi { $name }\nn = { $count ->\n    [one] one file\n   *[other] { $count } files\n}\nplain = Plain\n    .title = Tip"),
            ("ar", "language-name = العربية\nplain = عادي"),
        ]);
        let b = c.bundle("ar-EG");
        assert_eq!((b.locale.as_str(), b.dir.as_str()), ("ar", "rtl"));
        assert_eq!(b.available.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["English", "العربية"]);
        assert_eq!(b.sources.iter().map(|s| s.locale.as_str()).collect::<Vec<_>>(), ["ar", "en"]);
        let m = &b.messages;
        assert_eq!(m["plain"], "عادي");
        assert_eq!(m["plain.title"], "Tip");
        assert_eq!(m["hi"], json!({"parts": ["Hi ", {"var": "name"}]}));
        assert_eq!(m["n"]["select"], json!({"var": "count"}));
        assert_eq!(m["n"]["default"], "other");
        assert_eq!(m["n"]["variants"]["one"], "one file");
    }

    #[test]
    fn the_global_locale_and_macro() {
        set_locale("en");
        assert_eq!(locale(), "en");
        assert_eq!(crate::t!("action-done-imported", files = 2, chunks = 1500), "Imported the drop folder: 2 files, 1,500 chunks.");
        assert!(!negotiate("system").is_empty());
    }
}
