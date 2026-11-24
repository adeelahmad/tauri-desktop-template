//! The compatibility rules: which features need which, which only work on the desktop,
//! which point at another feature, and which bring in code under a licence other than
//! MIT or Apache-2.0.
//!
//! This is the one rule table. Kitchen checks it before `new`, `enable`, `disable` and
//! `set` render anything, and `kitchen features --check` checks an app's `kitchen.toml`
//! against it. The template's cargo-generate pre-hook (`template/hooks/compat.rhai`) holds
//! the same tables, so plain `cargo generate` refuses the same combinations; a test parses
//! both and fails when they drift, and another keeps the "Feature dependencies" tables in
//! the README and the guide equal to [`markdown`].
//!
//! Every rule comes from the template's code: the Cargo features (`daemon = ["ipc-worker",
//! ...]`), the conditional questions in `cargo-generate.toml`, and the
//! `cfg(all(desktop, ...))` gates.
//!
//! ```
//! use kitchen::compat::{self, Level};
//!
//! // daemon on, ipc_worker off: refused, with the reason.
//! let on = |f: &str| f == "daemon";
//! let found = compat::check(on);
//! let e = found.iter().find(|f| f.level == Level::Error).unwrap();
//! assert_eq!(e.feature, "daemon");
//! assert!(e.message.contains("needs ipc_worker"));
//!
//! // What `--with-deps` would turn on for `kitchen enable daemon`.
//! assert_eq!(compat::missing_needs(&["daemon"], |_| false), [("ipc_worker", "daemon")]);
//! ```

use std::collections::BTreeSet;

/// `feature` only works with `needs` on: generating one without the other is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requires {
    /// The answer (cargo-generate placeholder) that needs another, e.g. `daemon`.
    pub feature: &'static str,
    /// The answer it needs, e.g. `ipc_worker`.
    pub needs: &'static str,
    /// Why, in a few words, from the code.
    pub why: &'static str,
}

/// `feature` works without `suggests`, but part of it does not: a warning, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Advice {
    /// The answer that is on, e.g. `oauth`.
    pub feature: &'static str,
    /// The answer that completes it, e.g. `tauri_deep_link`.
    pub suggests: &'static str,
    /// What is missing without it.
    pub why: &'static str,
}

/// A feature compiled out on iOS and Android (`cfg(desktop)`): a warning when `mobile` is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopOnly {
    /// The answer, e.g. `daemon`.
    pub feature: &'static str,
    /// What the mobile build does instead.
    pub on_mobile: &'static str,
}

/// What a feature adds to the app's licences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Licence {
    /// The answer, e.g. `postgres`.
    pub feature: &'static str,
    /// The code it brings in and its licences (SPDX), checked against each crate and project.
    pub licences: &'static str,
    /// A short notice when anything is not MIT or Apache-2.0; empty when all of it is.
    pub notice: &'static str,
}

/// Hard dependencies. The first four are Cargo features (`bm25 = ["postgres"]`,
/// `llama = ["ai", ...]`, `background-agent = ["ipc-worker"]`, `daemon = ["ipc-worker",
/// ...]`); all of them are also conditional questions in `cargo-generate.toml`, rendered
/// only inside `{% if <needs> %}`.
pub const REQUIRES: &[Requires] = &[
    Requires {
        feature: "background_agent",
        needs: "ipc_worker",
        why: "it starts the worker process at login",
    },
    Requires {
        feature: "daemon",
        needs: "ipc_worker",
        why: "the daemon is the worker process run headless",
    },
    Requires {
        feature: "bm25",
        needs: "postgres",
        why: "VectorChord-BM25 is a Postgres extension",
    },
    Requires {
        feature: "local_llm",
        needs: "ai",
        why: "llama.cpp is the local provider of AI chat",
    },
    Requires {
        feature: "release_channels",
        needs: "updater",
        why: "channels, staged rollout and rollback steer the updater",
    },
    Requires {
        feature: "tauri_dialog",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_fs",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_clipboard",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_notification",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_opener",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_store",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_global_shortcut",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_autostart",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_deep_link",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_window_state",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_os",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_process",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_log",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_http",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_positioner",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "tauri_shell",
        needs: "tauri_plugins",
        why: "each Tauri plugin is wired in only with tauri_plugins",
    },
    Requires {
        feature: "macos_pkg",
        needs: "installers",
        why: "the release workflow builds it",
    },
    Requires {
        feature: "install_scripts",
        needs: "installers",
        why: "they download what the release workflow builds",
    },
    Requires {
        feature: "linux",
        needs: "installers",
        why: "the release workflow builds them",
    },
    Requires {
        feature: "bundle_embedding_model",
        needs: "embeddings",
        why: "it puts that feature's model inside the installer",
    },
    Requires {
        feature: "bundle_stt_model",
        needs: "stt",
        why: "it puts that feature's model inside the installer",
    },
    Requires {
        feature: "bundle_tts_voice",
        needs: "tts",
        why: "it puts that feature's model inside the installer",
    },
    Requires {
        feature: "bundle_llm",
        needs: "local_llm",
        why: "it puts that feature's model inside the installer",
    },
];

/// Soft dependencies: the feature works, one part of it does not.
pub const ADVICE: &[Advice] = &[Advice {
    feature: "oauth",
    suggests: "tauri_deep_link",
    why: "the deep-link redirect (oauth.redirect = deepLink) needs the deep-link plugin; without it sign-in uses the loopback redirect",
}];

/// Features gated on `cfg(desktop)` in the template (or desktop-only Tauri plugins, listed
/// in `capabilities/desktop.json`).
pub const DESKTOP_ONLY: &[DesktopOnly] = &[
    DesktopOnly {
        feature: "ipc_worker",
        on_mobile: "the backend runs inside the app",
    },
    DesktopOnly {
        feature: "background_agent",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "daemon",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "updater",
        on_mobile: "the app stores update the app",
    },
    DesktopOnly {
        feature: "release_channels",
        on_mobile: "the app stores update the app (use their test tracks)",
    },
    DesktopOnly {
        feature: "single_instance",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "mcp_server",
        on_mobile: "it is compiled out (no stdio server on a phone)",
    },
    DesktopOnly {
        feature: "python_example",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "claude_mcp",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "watch_folders",
        on_mobile: "it is compiled out (no user folders to watch)",
    },
    DesktopOnly {
        feature: "postgres",
        on_mobile: "the file store is used instead",
    },
    DesktopOnly {
        feature: "http_api",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "webview",
        on_mobile: "it is compiled out",
    },
    DesktopOnly {
        feature: "tray_menu",
        on_mobile: "it is compiled out (no tray, menu bar or global shortcuts)",
    },
    DesktopOnly {
        feature: "app_shell",
        on_mobile: "the titlebar and Settings window are compiled out; themes and fonts still work",
    },
    DesktopOnly {
        feature: "tauri_global_shortcut",
        on_mobile: "the plugin is left out",
    },
    DesktopOnly {
        feature: "tauri_autostart",
        on_mobile: "the plugin is left out",
    },
    DesktopOnly {
        feature: "tauri_window_state",
        on_mobile: "the plugin is left out",
    },
    DesktopOnly {
        feature: "tauri_process",
        on_mobile: "the plugin is left out",
    },
    DesktopOnly {
        feature: "tauri_positioner",
        on_mobile: "the plugin is left out",
    },
    DesktopOnly {
        feature: "tauri_shell",
        on_mobile: "the plugin is left out",
    },
];

/// Licences of what each feature brings in beyond the MIT / Apache-2.0 crates every app has.
pub const LICENCES: &[Licence] = &[
    Licence {
        feature: "postgres",
        licences: "PostgreSQL 16 server and pgvector: PostgreSQL; postgresql_embedded: (MIT OR Apache-2.0) AND PostgreSQL",
        notice: "PostgreSQL and pgvector are under the PostgreSQL Licence (permissive, BSD-like): keep their copyright notice with the app",
    },
    Licence {
        feature: "bm25",
        licences: "VectorChord-BM25: AGPL-3.0 OR Elastic-2.0; pg_tokenizer: Apache-2.0",
        notice: "VectorChord-BM25 is AGPL-3.0 or Elastic License 2.0: pick one before you ship (ELv2 forbids offering it as a managed service; AGPL asks for your source)",
    },
    Licence {
        feature: "embeddings",
        licences: "fastembed, hf-hub: Apache-2.0; ONNX Runtime: MIT; Oniguruma: BSD-2-Clause; models: Apache-2.0 (nomic, MiniLM) or MIT (BGE, E5)",
        notice: "the tokenizer's Oniguruma regex library is BSD-2-Clause: keep its copyright notice with the app",
    },
    Licence {
        feature: "pdf",
        licences: "docling: MIT; docling-pdf: MIT AND BSD-3-Clause (Adobe mapping resources and glyph list); tesseract (optional, not bundled): Apache-2.0",
        notice: "docling-pdf embeds Adobe's PDF mapping resources and glyph list (BSD-3-Clause): keep their licence text with the app",
    },
    Licence {
        feature: "watch_folders",
        licences: "notify: CC0-1.0; notify-debouncer-full: MIT OR Apache-2.0",
        notice: "notify is CC0-1.0 (public domain dedication): nothing to do",
    },
    Licence { feature: "local_llm", licences: "llama.cpp: MIT; llama-cpp-2: MIT OR Apache-2.0", notice: "" },
    Licence { feature: "tts", licences: "ONNX Runtime: MIT; ort: MIT OR Apache-2.0; tiny-voca model: Apache-2.0", notice: "" },
    Licence { feature: "stt", licences: "ONNX Runtime: MIT; ort: MIT OR Apache-2.0; Whisper models: MIT", notice: "" },
    Licence { feature: "rclone", licences: "rclone (downloaded on first use): MIT", notice: "" },
    Licence { feature: "daemon", licences: "service-manager, signal-hook, windows-service: MIT OR Apache-2.0", notice: "" },
    Licence { feature: "oauth", licences: "keyring: MIT OR Apache-2.0", notice: "" },
    Licence { feature: "ipc_worker", licences: "interprocess: 0BSD OR Apache-2.0", notice: "" },
    Licence { feature: "background_jobs", licences: "croner: MIT; jiff: Unlicense OR MIT", notice: "" },
    Licence { feature: "http_api", licences: "axum: MIT; tokio: MIT", notice: "" },
];

/// How serious a [`Finding`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// The combination does not build or does nothing: refused.
    Error,
    /// It builds, but part of a feature is missing (mobile, advice).
    Warning,
    /// A licence to know about.
    Notice,
}

/// One result of [`check`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Finding {
    /// Error, warning or licence notice.
    pub level: Level,
    /// The answer it is about.
    pub feature: &'static str,
    /// For errors: the answer it needs; for advice: the one it suggests.
    pub other: Option<&'static str>,
    /// What is wrong (or what to know), in one line.
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tag = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Notice => "licence",
        };
        write!(f, "{tag}: {}", self.message)
    }
}

/// Check a set of answers: `on(name)` says whether a yes/no answer is on. Errors first,
/// then warnings, then licence notices, each in table order.
pub fn check(on: impl Fn(&str) -> bool) -> Vec<Finding> {
    let mut out = Vec::new();
    for r in REQUIRES.iter().filter(|r| on(r.feature) && !on(r.needs)) {
        out.push(Finding {
            level: Level::Error,
            feature: r.feature,
            other: Some(r.needs),
            message: format!("{} needs {}: {}", r.feature, r.needs, r.why),
        });
    }
    for a in ADVICE.iter().filter(|a| on(a.feature) && !on(a.suggests)) {
        out.push(Finding {
            level: Level::Warning,
            feature: a.feature,
            other: Some(a.suggests),
            message: format!("{} without {}: {}", a.feature, a.suggests, a.why),
        });
    }
    if on("mobile") {
        for d in DESKTOP_ONLY.iter().filter(|d| on(d.feature)) {
            out.push(Finding {
                level: Level::Warning,
                feature: d.feature,
                other: None,
                message: format!(
                    "{} is desktop only: on iOS and Android, {}",
                    d.feature, d.on_mobile
                ),
            });
        }
    }
    for l in LICENCES
        .iter()
        .filter(|l| on(l.feature) && !l.notice.is_empty())
    {
        out.push(Finding {
            level: Level::Notice,
            feature: l.feature,
            other: None,
            message: format!("{}: {}", l.feature, l.notice),
        });
    }
    out
}

/// The bundled-models checks (`models = bundle` or `bundle_with_fallback`): how much the
/// models add to the installer (a notice, always), a warning past about 200 MB or with a
/// large model (the 2 GB release asset limit, notarization uploads), and that bundling
/// redistributes each model under its licence. `get(name)` is an answer as text.
pub fn check_models(get: impl Fn(&str) -> Option<String>) -> Vec<Finding> {
    let chosen = crate::models::chosen(&get);
    if chosen.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = chosen
        .iter()
        .map(|m| format!("{} {}", m.name, crate::models::mb(m.bytes())))
        .collect();
    let (size, warn) = crate::models::size_notes(
        crate::models::total(&chosen),
        chosen.iter().any(|m| m.kind == "llm"),
    );
    let finding = |level, message| Finding {
        level,
        feature: "models",
        other: None,
        message,
    };
    let mut out = vec![finding(
        Level::Notice,
        format!("models: {size} ({})", names.join(", ")),
    )];
    out.extend(
        warn.into_iter()
            .map(|w| finding(Level::Warning, format!("models: {w}"))),
    );
    let licences: Vec<String> = chosen
        .iter()
        .map(|m| format!("{} {}", m.name, m.licence))
        .collect();
    out.push(finding(
        Level::Notice,
        format!(
            "models: bundling redistributes the models; ship their licences with the app ({})",
            licences.join(", ")
        ),
    ));
    out
}

/// What `--with-deps` turns on for `enable`: every answer the named ones need (and what
/// those need), that is not on yet, as `(needed, needed_by)`, in the order found.
pub fn missing_needs(
    enable: &[&str],
    on: impl Fn(&str) -> bool,
) -> Vec<(&'static str, &'static str)> {
    let mut seen: BTreeSet<&str> = enable.iter().copied().collect();
    let mut queue: Vec<&str> = enable.to_vec();
    let mut out = Vec::new();
    while let Some(name) = queue.pop() {
        for r in REQUIRES
            .iter()
            .filter(|r| r.feature == name && !on(r.needs))
        {
            if seen.insert(r.needs) {
                out.push((r.needs, r.feature));
                queue.push(r.needs);
            }
        }
    }
    out
}

/// What `--with-deps` turns off for `disable`: every answer that is on and needs one of
/// the named ones (directly or through another), as `(dependent, because_of)`.
pub fn dependents(
    disable: &[&str],
    on: impl Fn(&str) -> bool,
) -> Vec<(&'static str, &'static str)> {
    let mut seen: BTreeSet<&str> = disable.iter().copied().collect();
    let mut queue: Vec<&str> = disable.to_vec();
    let mut out = Vec::new();
    while let Some(name) = queue.pop() {
        for r in REQUIRES.iter().filter(|r| r.needs == name && on(r.feature)) {
            if seen.insert(r.feature) {
                out.push((r.feature, r.needs));
                queue.push(r.feature);
            }
        }
    }
    out
}

/// The answers `feature` needs directly.
pub fn needs_of(feature: &str) -> Vec<&'static str> {
    REQUIRES
        .iter()
        .filter(|r| r.feature == feature)
        .map(|r| r.needs)
        .collect()
}

/// The "Feature dependencies" section of the README and the guide, generated from the
/// tables (rows with the same need and reason are grouped).
pub fn markdown() -> String {
    let mut s = String::from(
        "Generated from `kitchen/src/compat.rs`; the template's pre-hook (`hooks/compat.rhai`) and\n\
         `kitchen enable` / `disable` / `set` / `features --check` apply the same rules.\n\n\
         | Feature | Needs | Why |\n|---|---|---|\n",
    );
    let mut done = vec![false; REQUIRES.len()];
    for (i, r) in REQUIRES.iter().enumerate() {
        if done[i] {
            continue;
        }
        let mut names = Vec::new();
        for (j, x) in REQUIRES.iter().enumerate().skip(i) {
            if x.needs == r.needs && x.why == r.why {
                done[j] = true;
                names.push(format!("`{}`", x.feature));
            }
        }
        s.push_str(&format!(
            "| {} | `{}` | {} |\n",
            names.join(", "),
            r.needs,
            r.why
        ));
    }
    s.push_str("\nWarnings (it still generates):\n\n| Feature | Without | What is missing |\n|---|---|---|\n");
    for a in ADVICE {
        s.push_str(&format!(
            "| `{}` | `{}` | {} |\n",
            a.feature, a.suggests, a.why
        ));
    }
    let desktop: Vec<String> = DESKTOP_ONLY
        .iter()
        .map(|d| format!("`{}`", d.feature))
        .collect();
    s.push_str(&format!(
        "| {} | a desktop target (`mobile` on) | desktop only: compiled out on iOS and Android |\n",
        desktop.join(", ")
    ));
    s.push_str("\nLicences checked for each feature (the pre-hook prints a notice for anything beyond MIT / Apache-2.0):\n\n| Feature | Brings in | Notice |\n|---|---|---|\n");
    for l in LICENCES {
        let notice = if l.notice.is_empty() {
            "none (MIT / Apache-2.0)"
        } else {
            l.notice
        };
        s.push_str(&format!(
            "| `{}` | {} | {} |\n",
            l.feature, l.licences, notice
        ));
    }
    s
}

/// The marker lines around the generated section in the README and the guide.
pub const DOC_MARKERS: (&str, &str) = (
    "<!-- kitchen:feature-dependencies (generated; see kitchen/src/compat.rs) -->",
    "<!-- /kitchen:feature-dependencies -->",
);

/// The tables as the Rhai pre-hook holds them (between its `kitchen:rules` markers).
pub fn rhai_tables() -> String {
    let row = |cells: &[&str]| {
        let quoted: Vec<String> = cells.iter().map(|c| format!("\"{c}\"")).collect();
        format!("    [{}],\n", quoted.join(", "))
    };
    let mut s = String::from("let REQUIRES = [\n");
    for r in REQUIRES {
        s.push_str(&row(&[r.feature, r.needs, r.why]));
    }
    s.push_str("];\nlet ADVICE = [\n");
    for a in ADVICE {
        s.push_str(&row(&[a.feature, a.suggests, a.why]));
    }
    s.push_str("];\nlet DESKTOP_ONLY = [\n");
    for d in DESKTOP_ONLY {
        s.push_str(&row(&[d.feature, d.on_mobile]));
    }
    s.push_str("];\nlet LICENCES = [\n");
    for l in LICENCES {
        s.push_str(&row(&[l.feature, l.notice]));
    }
    s.push_str("];\nlet MODELS = [\n");
    for m in crate::models::CATALOG
        .iter()
        .filter(|m| !m.answer.is_empty())
    {
        let (k, v) = m.when.unwrap_or(("", ""));
        let mb = ((m.bytes() + 500_000) / 1_000_000).to_string();
        s.push_str(&row(&[m.answer, k, v, m.name, &mb, m.kind, m.licence]));
    }
    s.push_str("];\n");
    s
}

/// Parse the tables out of a Rhai script written like [`rhai_tables`]: `(name, rows)`,
/// each row the string cells of one `[...]` line.
pub fn parse_rhai_tables(script: &str) -> Vec<(String, Vec<Vec<String>>)> {
    let mut out: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    for line in script.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("let ") {
            if let Some(name) = rest.strip_suffix(" = [") {
                out.push((name.to_string(), Vec::new()));
            }
        } else if line.starts_with('[') && line.ends_with("],") {
            if let Some((_, rows)) = out.last_mut() {
                rows.push(strings(line));
            }
        }
    }
    out
}

/// The `"..."` literals in a line (no escapes: the tables never need them).
fn strings(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_set<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |n| names.contains(&n)
    }

    #[test]
    fn the_named_combinations_are_refused() {
        for (feature, needs) in [
            ("background_agent", "ipc_worker"),
            ("daemon", "ipc_worker"),
            ("bm25", "postgres"),
            ("local_llm", "ai"),
        ] {
            let f = check(on_set(&[feature]));
            assert!(
                f.iter().any(|x| x.level == Level::Error
                    && x.feature == feature
                    && x.other == Some(needs)),
                "{feature}: {f:?}"
            );
            assert!(check(on_set(&[feature, needs]))
                .iter()
                .all(|x| x.level != Level::Error));
        }
    }

    #[test]
    fn mobile_warns_about_desktop_only_features() {
        let f = check(on_set(&[
            "mobile",
            "webview",
            "daemon",
            "ipc_worker",
            "tray_menu",
            "app_shell",
            "tauri_plugins",
            "tauri_shell",
        ]));
        let warned: Vec<&str> = f
            .iter()
            .filter(|x| x.level == Level::Warning)
            .map(|x| x.feature)
            .collect();
        for d in [
            "webview",
            "daemon",
            "ipc_worker",
            "tray_menu",
            "app_shell",
            "tauri_shell",
        ] {
            assert!(warned.contains(&d), "{d}: {warned:?}");
        }
        assert!(f.iter().all(|x| x.level != Level::Error));
        assert!(check(on_set(&["webview"])).is_empty());
    }

    #[test]
    fn oauth_without_deep_link_is_a_warning() {
        let f = check(on_set(&["oauth"]));
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].level, f[0].other),
            (Level::Warning, Some("tauri_deep_link"))
        );
        assert!(check(on_set(&["oauth", "tauri_plugins", "tauri_deep_link"])).is_empty());
    }

    #[test]
    fn licence_notices_only_for_non_mit_apache() {
        let f = check(on_set(&[
            "postgres",
            "bm25",
            "local_llm",
            "rclone",
            "daemon",
            "ipc_worker",
        ]));
        let notices: Vec<&str> = f
            .iter()
            .filter(|x| x.level == Level::Notice)
            .map(|x| x.feature)
            .collect();
        assert_eq!(notices, ["postgres", "bm25"]);
    }

    #[test]
    fn with_deps_follows_chains_both_ways() {
        assert_eq!(
            missing_needs(&["tauri_deep_link"], |_| false),
            [("tauri_plugins", "tauri_deep_link")]
        );
        assert!(missing_needs(&["daemon"], |n| n == "ipc_worker").is_empty());
        let mut d = dependents(&["ipc_worker"], on_set(&["daemon", "background_agent"]));
        d.sort();
        assert_eq!(
            d,
            [("background_agent", "ipc_worker"), ("daemon", "ipc_worker")]
        );
        assert_eq!(needs_of("bm25"), ["postgres"]);
    }

    #[test]
    fn table_cells_are_rhai_and_markdown_safe() {
        let cells = REQUIRES
            .iter()
            .flat_map(|r| [r.feature, r.needs, r.why])
            .chain(ADVICE.iter().flat_map(|a| [a.feature, a.suggests, a.why]))
            .chain(DESKTOP_ONLY.iter().flat_map(|d| [d.feature, d.on_mobile]))
            .chain(
                LICENCES
                    .iter()
                    .flat_map(|l| [l.feature, l.licences, l.notice]),
            );
        for c in cells {
            for bad in ['"', '\\', '|', '{', '}', '`'] {
                assert!(!c.contains(bad), "{c:?} has {bad:?}");
            }
        }
    }

    #[test]
    fn bundled_models_print_their_size_and_warn_when_large() {
        let get = |llm: bool| {
            move |k: &str| match k {
                "models" => Some("bundle_with_fallback".to_string()),
                "bundle_stt_model" | "bundle_tts_voice" => Some("true".into()),
                "bundle_llm" => Some(llm.to_string()),
                _ => None,
            }
        };
        let small = check_models(get(false));
        assert!(small.iter().all(|f| f.level == Level::Notice));
        assert!(small[0].message.contains("MB") && small[0].message.contains("whisper-base"));
        assert!(small[1].message.contains("tiny-voca Apache-2.0"));
        let big = check_models(get(true));
        let warn: Vec<_> = big.iter().filter(|f| f.level == Level::Warning).collect();
        assert!(warn.iter().any(|f| f.message.contains("200 MB")));
        assert!(warn
            .iter()
            .any(|f| f.message.contains("2 GB") && f.message.contains("notarization")));
        assert!(check_models(|_| None).is_empty());
    }

    #[test]
    fn rhai_tables_round_trip() {
        let t = parse_rhai_tables(&rhai_tables());
        let names: Vec<&str> = t.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["REQUIRES", "ADVICE", "DESKTOP_ONLY", "LICENCES", "MODELS"]
        );
        assert!(t[4]
            .1
            .iter()
            .any(|r| r[0] == "bundle_stt_model" && r[3] == "whisper-base"));
        assert_eq!(t[0].1.len(), REQUIRES.len());
        assert_eq!(t[0].1[1], ["daemon", "ipc_worker", REQUIRES[1].why]);
    }
}
