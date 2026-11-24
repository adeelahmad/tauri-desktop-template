//! Plugins: app extensions compiled into the binary, with no dynamic loading.
//!
//! A plugin implements [`Plugin`] and is listed in [`builtin`]. It can
//! - change, add or drop chunks before they are stored ([`Plugin::on_ingest`]),
//! - add results to every search ([`Plugin::search`]), for example from another source,
//! - answer its own methods, which the backend reaches as `plugin.<name>.<method>`
//!   ([`Plugin::call`]),
//! - describe its settings with a JSON Schema ([`Plugin::settings_schema`]), so the UI can
//!   draw a form; the values live in `plugins.settings.<name>` and reach every hook.
//!
//! Plugins are off until `plugins.enabled` is switched on in settings, and each one can be
//! turned off on its own (`plugins.disabled`). Being static keeps them safe: a plugin is
//! reviewed and built with the app, like any other module, and cannot be swapped on disk.
//!
//! ```
//! use desktop_core::plugin::{Plugin, PluginsConfig, Registry};
//! use desktop_core::Chunk;
//! use serde_json::{json, Value};
//!
//! /// Upper-cases every chunk and answers `shout.hello`.
//! struct Shout;
//!
//! impl Plugin for Shout {
//!     fn name(&self) -> &str {
//!         "shout"
//!     }
//!     fn version(&self) -> &str {
//!         "1.0.0"
//!     }
//!     fn on_ingest(&self, _source: &str, chunks: &mut Vec<Chunk>, _settings: &Value) -> Result<(), String> {
//!         chunks.iter_mut().for_each(|c| c.text = c.text.to_uppercase());
//!         Ok(())
//!     }
//!     fn methods(&self) -> Vec<&str> {
//!         vec!["hello"]
//!     }
//!     fn call(&self, method: &str, params: &Value, _settings: &Value) -> Result<Value, String> {
//!         match method {
//!             "hello" => Ok(json!(format!("HELLO, {}!", params["name"].as_str().unwrap_or("world")))),
//!             other => Err(format!("no method {other}")),
//!         }
//!     }
//! }
//!
//! let registry = Registry::new(vec![Box::new(Shout)]);
//! let config = PluginsConfig { enabled: true, ..PluginsConfig::default() };
//! let reply = registry.call(&config, "plugin.shout.hello", &json!({"name": "Ada"})).unwrap();
//! assert_eq!(reply, json!("HELLO, Ada!"));
//! ```

use crate::chunk::Chunk;
use crate::store::Hit;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// An app extension. Only [`name`](Plugin::name) and [`version`](Plugin::version) are
/// required; every hook does nothing by default.
pub trait Plugin: Send + Sync {
    /// A short, unique id: lowercase letters, digits and `-` (it appears in method names).
    fn name(&self) -> &str;
    /// The plugin's own version, shown in Settings.
    fn version(&self) -> &str;
    /// One line for Settings.
    fn description(&self) -> &str {
        ""
    }
    /// Runs before a source's chunks are stored. Change their text, add chunks, or clear
    /// the list to keep the source out of the index. An error stops that ingest.
    fn on_ingest(&self, _source: &str, _chunks: &mut Vec<Chunk>, _settings: &Value) -> Result<(), String> {
        Ok(())
    }
    /// Extra results for a search, best first (at most `k`). They are interleaved with the
    /// store's results; use `source` to say where each came from.
    fn search(&self, _query: &str, _k: usize, _settings: &Value) -> Result<Vec<Hit>, String> {
        Ok(Vec::new())
    }
    /// The methods [`call`](Plugin::call) answers (reached as `plugin.<name>.<method>`).
    fn methods(&self) -> Vec<&str> {
        Vec::new()
    }
    /// Answer one of [`methods`](Plugin::methods).
    fn call(&self, method: &str, _params: &Value, _settings: &Value) -> Result<Value, String> {
        Err(format!("{} has no method {method:?}", self.name()))
    }
    /// A JSON Schema (an `object` with `properties`) for this plugin's settings.
    fn settings_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
}

/// Plugin settings (`plugins` in settings): all off by default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginsConfig {
    /// Run plugins at all.
    pub enabled: bool,
    /// Names of plugins to leave out.
    pub disabled: Vec<String>,
    /// Each plugin's settings, by name (see [`Plugin::settings_schema`]).
    pub settings: Map<String, Value>,
}

impl PluginsConfig {
    pub fn validated(mut self) -> Self {
        self.disabled = self.disabled.iter().map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).collect();
        self.disabled.sort();
        self.disabled.dedup();
        self
    }

    /// Whether the plugin called `name` runs.
    pub fn runs(&self, name: &str) -> bool {
        self.enabled && !self.disabled.iter().any(|d| d == name)
    }

    fn settings_of(&self, name: &str) -> &Value {
        static EMPTY: Value = Value::Null;
        self.settings.get(name).unwrap_or(&EMPTY)
    }
}

/// What Settings shows about a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub methods: Vec<String>,
    pub schema: Value,
    /// Runs with the current settings.
    pub active: bool,
}

/// The plugins built into this app.
#[derive(Default)]
pub struct Registry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl Registry {
    /// A registry of these plugins. Names must be unique and valid (see [`Plugin::name`]);
    /// a plugin that breaks the rule is left out with a warning.
    pub fn new(plugins: Vec<Box<dyn Plugin>>) -> Self {
        let mut kept: Vec<Box<dyn Plugin>> = Vec::with_capacity(plugins.len());
        for p in plugins {
            if !valid_name(p.name()) {
                log::warn!("plugin {:?} left out: names are lowercase letters, digits and '-'", p.name());
            } else if kept.iter().any(|k| k.name() == p.name()) {
                log::warn!("plugin {:?} left out: the name is taken", p.name());
            } else {
                kept.push(p);
            }
        }
        Self { plugins: kept }
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Every plugin with its schema, and whether it runs under `config`.
    pub fn list(&self, config: &PluginsConfig) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| PluginInfo {
                name: p.name().into(),
                version: p.version().into(),
                description: p.description().into(),
                methods: p.methods().into_iter().map(String::from).collect(),
                schema: p.settings_schema(),
                active: config.runs(p.name()),
            })
            .collect()
    }

    fn active<'a>(&'a self, config: &'a PluginsConfig) -> impl Iterator<Item = &'a dyn Plugin> + 'a {
        self.plugins.iter().map(|p| p.as_ref()).filter(|p| config.runs(p.name()))
    }

    /// Run every active plugin's [`Plugin::on_ingest`], in order.
    pub fn on_ingest(&self, config: &PluginsConfig, source: &str, chunks: &mut Vec<Chunk>) -> Result<(), String> {
        for p in self.active(config) {
            p.on_ingest(source, chunks, config.settings_of(p.name())).map_err(|e| format!("plugin {}: {e}", p.name()))?;
        }
        Ok(())
    }

    /// The store's hits interleaved with every active plugin's (one from each list in
    /// turn, so no source crowds out the others), cut to `k`. A plugin that fails is
    /// logged and skipped: search keeps working.
    pub fn search(&self, config: &PluginsConfig, query: &str, k: usize, store_hits: Vec<Hit>) -> Vec<Hit> {
        let mut lists = vec![store_hits];
        for p in self.active(config) {
            match p.search(query, k, config.settings_of(p.name())) {
                Ok(mut hits) => {
                    hits.truncate(k);
                    lists.push(hits);
                }
                Err(e) => log::warn!("plugin {} search failed: {e}", p.name()),
            }
        }
        interleave(lists, k)
    }

    /// Answer `plugin.<name>.<method>`.
    pub fn call(&self, config: &PluginsConfig, method: &str, params: &Value) -> Result<Value, String> {
        let (name, inner) = split_method(method).ok_or_else(|| format!("not a plugin method: {method:?} (use plugin.<name>.<method>)"))?;
        let p = self.plugins.iter().find(|p| p.name() == name).ok_or_else(|| format!("no plugin called {name:?}"))?;
        if !config.runs(name) {
            return Err(format!("plugin {name} is turned off in Settings"));
        }
        if !p.methods().contains(&inner) {
            return Err(format!("plugin {name} has no method {inner:?}"));
        }
        p.call(inner, params, config.settings_of(name))
    }
}

/// `plugin.<name>.<method>` as `(name, method)`.
pub fn split_method(method: &str) -> Option<(&str, &str)> {
    let rest = method.strip_prefix("plugin.")?;
    let (name, inner) = rest.split_once('.')?;
    (!name.is_empty() && !inner.is_empty()).then_some((name, inner))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn interleave(lists: Vec<Vec<Hit>>, k: usize) -> Vec<Hit> {
    let mut iters: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
    let mut out = Vec::new();
    while out.len() < k {
        let before = out.len();
        for it in &mut iters {
            if out.len() == k {
                break;
            }
            out.extend(it.next());
        }
        if out.len() == before {
            break;
        }
    }
    out
}

/// The plugins compiled into this app. Add yours here.
pub fn builtin() -> Registry {
    Registry::new(vec![
        Box::new(Glossary),
        // kitchen:plugins
    ])
}

/// The example plugin: a glossary of terms (`plugins.settings.glossary.terms`, term →
/// definition) found by search, a `define` method, and optional redaction of email
/// addresses before text is stored (`redactEmails`).
pub struct Glossary;

impl Glossary {
    fn terms(settings: &Value) -> Vec<(String, String)> {
        let mut terms: Vec<(String, String)> = settings
            .get("terms")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        terms.sort();
        terms
    }
}

impl Plugin for Glossary {
    fn name(&self) -> &str {
        "glossary"
    }
    fn version(&self) -> &str {
        "1.0.0"
    }
    fn description(&self) -> &str {
        "Your own glossary in search results, and optional email redaction before indexing"
    }
    fn on_ingest(&self, _source: &str, chunks: &mut Vec<Chunk>, settings: &Value) -> Result<(), String> {
        if settings.get("redactEmails").and_then(Value::as_bool).unwrap_or(false) {
            for c in chunks.iter_mut() {
                c.text = redact_emails(&c.text);
            }
        }
        Ok(())
    }
    fn search(&self, query: &str, k: usize, settings: &Value) -> Result<Vec<Hit>, String> {
        let words: Vec<String> = query.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(str::to_lowercase).collect();
        let mut hits: Vec<Hit> = Self::terms(settings)
            .into_iter()
            .filter_map(|(term, def)| {
                let t = term.to_lowercase();
                let n = words.iter().filter(|w| t.split_whitespace().any(|tw| tw == w.as_str())).count();
                (n > 0).then(|| Hit {
                    source: format!("glossary:{term}"),
                    page: None,
                    text: format!("{term}: {def}"),
                    score: n as f32 / words.len().max(1) as f32,
                    vector_score: None,
                    keyword_score: None,
                    prov: None,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.source.cmp(&b.source)));
        hits.truncate(k);
        Ok(hits)
    }
    fn methods(&self) -> Vec<&str> {
        vec!["define", "terms"]
    }
    fn call(&self, method: &str, params: &Value, settings: &Value) -> Result<Value, String> {
        let terms = Self::terms(settings);
        match method {
            "terms" => Ok(json!(terms.into_iter().map(|(t, _)| t).collect::<Vec<_>>())),
            "define" => {
                let want = params.get("term").and_then(Value::as_str).ok_or("define needs {term}")?;
                Ok(terms.into_iter().find(|(t, _)| t.eq_ignore_ascii_case(want)).map_or(Value::Null, |(_, d)| json!(d)))
            }
            other => Err(format!("glossary has no method {other:?}")),
        }
    }
    fn settings_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "terms": {"type": "object", "title": "Terms (term → definition)", "additionalProperties": {"type": "string"}},
                "redactEmails": {"type": "boolean", "title": "Replace email addresses with [email] before indexing", "default": false}
            }
        })
    }
}

/// Replace every `local@domain.tld` in `text` with `[email]`.
fn redact_emails(text: &str) -> String {
    let local = |c: char| c.is_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_alphanumeric() || ".-".contains(c);
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let (before, after) = rest.split_at(at);
        let start = before.rfind(|c: char| !local(c)).map_or(0, |i| i + before[i..].chars().next().map_or(1, char::len_utf8));
        let end = after[1..].find(|c: char| !domain(c)).map_or(after.len(), |i| i + 1);
        let host = after[1..end].trim_end_matches('.');
        let end = 1 + host.len();
        if start < before.len() && host.contains('.') && !host.starts_with('.') {
            out.push_str(&before[..start]);
            out.push_str("[email]");
        } else {
            out.push_str(before);
            out.push_str(&after[..end]);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> PluginsConfig {
        PluginsConfig { enabled: true, ..PluginsConfig::default() }
    }

    fn chunk(text: &str) -> Chunk {
        Chunk { source: "s".into(), page: None, text: text.into(), prov: None }
    }

    fn hit(source: &str) -> Hit {
        Hit { source: source.into(), page: None, text: source.into(), score: 1.0, vector_score: None, keyword_score: None, prov: None }
    }

    fn glossary_settings() -> PluginsConfig {
        let mut c = on();
        c.settings.insert(
            "glossary".into(),
            json!({"terms": {"RRF": "reciprocal rank fusion", "Vector search": "nearest neighbours by embedding"}, "redactEmails": true}),
        );
        c
    }

    #[test]
    fn method_names_split() {
        assert_eq!(split_method("plugin.glossary.define"), Some(("glossary", "define")));
        assert_eq!(split_method("plugin.a.b.c"), Some(("a", "b.c")));
        assert_eq!(split_method("plugin.glossary"), None);
        assert_eq!(split_method("plugin..x"), None);
        assert_eq!(split_method("search"), None);
    }

    #[test]
    fn off_by_default_and_per_plugin() {
        let r = builtin();
        let off = PluginsConfig::default();
        assert!(r.call(&off, "plugin.glossary.terms", &Value::Null).unwrap_err().contains("turned off"));
        assert!(!r.list(&off)[0].active);
        let mut c = on();
        c.disabled = vec!["glossary".into()];
        assert!(r.call(&c, "plugin.glossary.terms", &Value::Null).is_err());
        let mut chunks = vec![chunk("mail me@example.com")];
        r.on_ingest(&off, "s", &mut chunks).unwrap();
        assert_eq!(chunks[0].text, "mail me@example.com");
    }

    #[test]
    fn unknown_plugins_and_methods_are_errors() {
        let r = builtin();
        assert!(r.call(&on(), "plugin.nope.x", &Value::Null).unwrap_err().contains("no plugin"));
        assert!(r.call(&on(), "plugin.glossary.nope", &Value::Null).unwrap_err().contains("no method"));
        assert!(r.call(&on(), "glossary.define", &Value::Null).is_err());
    }

    #[test]
    fn bad_and_duplicate_names_are_left_out() {
        let r = Registry::new(vec![Box::new(Glossary), Box::new(Glossary)]);
        assert_eq!(r.list(&on()).len(), 1);
        assert!(valid_name("my-plugin2"));
        assert!(!valid_name("My.Plugin"));
        assert!(!valid_name(""));
    }

    #[test]
    fn glossary_defines_and_lists() {
        let (r, c) = (builtin(), glossary_settings());
        assert_eq!(r.call(&c, "plugin.glossary.define", &json!({"term": "rrf"})).unwrap(), json!("reciprocal rank fusion"));
        assert_eq!(r.call(&c, "plugin.glossary.define", &json!({"term": "bm25"})).unwrap(), Value::Null);
        assert_eq!(r.call(&c, "plugin.glossary.terms", &Value::Null).unwrap(), json!(["RRF", "Vector search"]));
        let info = &r.list(&c)[0];
        assert_eq!((info.name.as_str(), info.active), ("glossary", true));
        assert!(info.schema["properties"]["redactEmails"].is_object());
    }

    #[test]
    fn search_interleaves_and_cuts() {
        let (r, c) = (builtin(), glossary_settings());
        let hits = r.search(&c, "what is vector search", 3, vec![hit("a"), hit("b"), hit("c")]);
        let sources: Vec<_> = hits.iter().map(|h| h.source.as_str()).collect();
        assert_eq!(sources, ["a", "glossary:Vector search", "b"]);
        assert_eq!(r.search(&PluginsConfig::default(), "vector", 2, vec![hit("a"), hit("b"), hit("c")]).len(), 2);
        assert!(interleave(vec![vec![], vec![]], 5).is_empty());
    }

    #[test]
    fn engine_runs_the_hooks_and_search_sources() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::FileStore::open(&dir.path().join("store")).unwrap();
        let engine = crate::Engine::with_parts(Box::new(store), None, dir.path(), &mut |_| {}).unwrap();
        let config = crate::Config { plugins: glossary_settings(), ..crate::Config::default() };
        engine.configure(config).unwrap();
        engine.ingest_text("notes", "Ask ada@example.org about fusion.").unwrap();
        let hits = engine.search("fusion rrf", 5).unwrap();
        assert!(hits.iter().any(|h| h.text.contains("Ask [email] about fusion.")), "{hits:?}");
        assert!(hits.iter().any(|h| h.source == "glossary:RRF"), "{hits:?}");
        assert_eq!(engine.plugin_call("plugin.glossary.define", &json!({"term": "RRF"})).unwrap(), json!("reciprocal rank fusion"));
        // Switched off: stored as given, and no glossary hits.
        engine.configure(crate::Config::default()).unwrap();
        engine.ingest_text("notes", "Ask ada@example.org about fusion.").unwrap();
        let hits = engine.search("fusion rrf", 5).unwrap();
        assert!(hits.iter().all(|h| !h.source.starts_with("glossary:")) && hits[0].text.contains("ada@example.org"));
    }

    #[test]
    fn ingest_hook_redacts_emails() {
        let (r, c) = (builtin(), glossary_settings());
        let mut chunks = vec![chunk("Write to ada.l+x@math.example.org. Or bob@host, or @ alone, café@x.io")];
        r.on_ingest(&c, "s", &mut chunks).unwrap();
        assert_eq!(chunks[0].text, "Write to [email]. Or bob@host, or @ alone, [email]");
    }
}
