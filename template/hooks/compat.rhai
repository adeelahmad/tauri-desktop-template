// Compatibility checks: cargo-generate runs this before it renders the app (a `pre` hook,
// registered in cargo-generate.toml). It refuses answers that do not work together, warns
// about desktop-only features in a mobile build, and prints licence notices.
//
// The tables between the kitchen:rules markers are generated from kitchen/src/compat.rs,
// the one rule table the Kitchen CLI uses too; `cargo test` in kitchen/ fails when they
// drift (KITCHEN_BLESS=1 rewrites them).

// Whether a yes/no answer is on. Questions that were not asked (their parent is off) are
// unset; `-d name=true` may arrive as text.
fn on(name) {
    if !variable::is_set(name) {
        return false;
    }
    let v = variable::get(name);
    v == true || v == "true"
}

// kitchen:rules begin
let REQUIRES = [
    ["background_agent", "ipc_worker", "it starts the worker process at login"],
    ["daemon", "ipc_worker", "the daemon is the worker process run headless"],
    ["bm25", "postgres", "VectorChord-BM25 is a Postgres extension"],
    ["local_llm", "ai", "llama.cpp is the local provider of AI chat"],
    ["release_channels", "updater", "channels, staged rollout and rollback steer the updater"],
    ["tauri_dialog", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_fs", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_clipboard", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_notification", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_opener", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_store", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_global_shortcut", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_autostart", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_deep_link", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_window_state", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_os", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_process", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_log", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_http", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_positioner", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["tauri_shell", "tauri_plugins", "each Tauri plugin is wired in only with tauri_plugins"],
    ["macos_pkg", "installers", "the release workflow builds it"],
    ["install_scripts", "installers", "they download what the release workflow builds"],
    ["linux", "installers", "the release workflow builds them"],
];
let ADVICE = [
    ["oauth", "tauri_deep_link", "the deep-link redirect (oauth.redirect = deepLink) needs the deep-link plugin; without it sign-in uses the loopback redirect"],
];
let DESKTOP_ONLY = [
    ["ipc_worker", "the backend runs inside the app"],
    ["background_agent", "it is compiled out"],
    ["daemon", "it is compiled out"],
    ["updater", "the app stores update the app"],
    ["release_channels", "the app stores update the app (use their test tracks)"],
    ["single_instance", "it is compiled out"],
    ["mcp_server", "it is compiled out (no stdio server on a phone)"],
    ["python_example", "it is compiled out"],
    ["claude_mcp", "it is compiled out"],
    ["watch_folders", "it is compiled out (no user folders to watch)"],
    ["postgres", "the file store is used instead"],
    ["http_api", "it is compiled out"],
    ["webview", "it is compiled out"],
    ["tray_menu", "it is compiled out (no tray, menu bar or global shortcuts)"],
    ["app_shell", "the titlebar and Settings window are compiled out; themes and fonts still work"],
    ["tauri_global_shortcut", "the plugin is left out"],
    ["tauri_autostart", "the plugin is left out"],
    ["tauri_window_state", "the plugin is left out"],
    ["tauri_process", "the plugin is left out"],
    ["tauri_positioner", "the plugin is left out"],
    ["tauri_shell", "the plugin is left out"],
];
let LICENCES = [
    ["postgres", "PostgreSQL and pgvector are under the PostgreSQL Licence (permissive, BSD-like): keep their copyright notice with the app"],
    ["bm25", "VectorChord-BM25 is AGPL-3.0 or Elastic License 2.0: pick one before you ship (ELv2 forbids offering it as a managed service; AGPL asks for your source)"],
    ["embeddings", "the tokenizer's Oniguruma regex library is BSD-2-Clause: keep its copyright notice with the app"],
    ["pdf", "docling-pdf embeds Adobe's PDF mapping resources and glyph list (BSD-3-Clause): keep their licence text with the app"],
    ["watch_folders", "notify is CC0-1.0 (public domain dedication): nothing to do"],
    ["local_llm", ""],
    ["tts", ""],
    ["stt", ""],
    ["rclone", ""],
    ["daemon", ""],
    ["oauth", ""],
    ["ipc_worker", ""],
    ["background_jobs", ""],
    ["http_api", ""],
];
// kitchen:rules end

let errors = [];
for r in REQUIRES {
    if on(r[0]) && !on(r[1]) {
        errors.push(`  - ${r[0]} needs ${r[1]}: ${r[2]}.
      Fix: add -d ${r[1]}=true, or -d ${r[0]}=false.`);
    }
}

let warnings = [];
for a in ADVICE {
    if on(a[0]) && !on(a[1]) {
        warnings.push(`  - ${a[0]} without ${a[1]}: ${a[2]}.`);
    }
}
if on("mobile") {
    for d in DESKTOP_ONLY {
        if on(d[0]) {
            warnings.push(`  - ${d[0]} is desktop only: on iOS and Android, ${d[1]}.`);
        }
    }
}
if warnings.len() > 0 {
    print("Kitchen warnings (the app still generates):");
    for w in warnings {
        print(w);
    }
}

let notices = [];
for l in LICENCES {
    if on(l[0]) && l[1] != "" {
        notices.push(`  - ${l[1]}.`);
    }
}
if notices.len() > 0 {
    print("Licence notices (code under licences other than MIT / Apache-2.0):");
    for n in notices {
        print(n);
    }
}

if errors.len() > 0 {
    let msg = "These answers do not work together:\n";
    for e in errors {
        msg += e + "\n";
    }
    msg += "See \"Feature dependencies\" in the README (or `kitchen features --check`).";
    abort(msg);
}
