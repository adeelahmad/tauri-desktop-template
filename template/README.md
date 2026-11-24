# {{app_name}}

<!-- Badges: replace or remove as you like. Shields: https://shields.io -->
[![CI](https://github.com/{{github_repo}}/actions/workflows/ci.yml/badge.svg)](https://github.com/{{github_repo}}/actions/workflows/ci.yml)
{% if installers -%}
[![Release](https://img.shields.io/github/v/release/{{github_repo}}?sort=semver)](https://github.com/{{github_repo}}/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/{{github_repo}}/total)](https://github.com/{{github_repo}}/releases)
{% endif -%}
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-fe5196.svg)](https://www.conventionalcommits.org)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit)](https://github.com/pre-commit/pre-commit)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Windows{% if linux %}%20%7C%20Linux{% endif %}-lightgrey)

{{description}}

{% if installers -%}
## Install

{% if install_scripts -%}
- **macOS (Apple Silicon):** `curl -fsSL https://raw.githubusercontent.com/{{github_repo}}/main/scripts/install.sh | sh`
- **Windows (PowerShell):** `irm https://raw.githubusercontent.com/{{github_repo}}/main/scripts/install.ps1 | iex`
{% endif -%}
- Or download from [Releases](https://github.com/{{github_repo}}/releases/latest) (see the notes there for unsigned builds).

{% endif -%}

A Rust + [Tauri 2](https://tauri.app) desktop app for macOS, Windows{% if linux %} and Linux{% else %} (and Linux for development){% endif %}.
Everything runs on the user's machine.

## What's inside

| Part | Where | Cargo feature |
|---|---|---|
| Chunking (size, overlap), file store, BM25 + vector + hybrid (RRF) search, parallel ingest | `desktop-core/src/{chunk,store,engine,fuse,config}.rs` | always |
{% if embeddings -%}
| Local ONNX embeddings (fastembed-rs, `{{embedding_model}}`) | `desktop-core/src/embed.rs` | `embed` |
{% endif -%}
{% if postgres -%}
| Embedded Postgres 16 + pgvector (HNSW cosine, full-text keywords) | `desktop-core/src/pgstore.rs` | `postgres` |
{% if bm25 -%}
| BM25 in Postgres (VectorChord-BM25 + pg_tokenizer) for hybrid search | `desktop-core/src/pgstore.rs` | `bm25` |
{% endif -%}
{% endif -%}
{% if watch_folders -%}
| Drop folder + watch folders (debounced auto-ingest, deletes forgotten) | `desktop-core/src/watch.rs` | `watch` |
{% endif -%}
{% if ai -%}
| AI answers over your documents (Claude, OpenAI, Gemini, Ollama, any OpenAI-compatible server): streaming, citations, fallbacks, budgets. Turn on under Settings → AI | `desktop-core/src/llm.rs` | `ai` |
{% if local_llm -%}
| llama.cpp built in (static; Metal on Apple Silicon): provider `local` runs a GGUF model in the app | `desktop-core/src/local.rs` | `llama` |
{% endif -%}
{% endif -%}
{% if tts -%}
| Text to speech in Rust (tiny-voca, 24 kHz, ONNX Runtime; two voices, speed and pitch): "Read aloud" on answers. Turn on under Settings → Speech | `desktop-core/src/tts.rs` | `tts` |
{% endif -%}
{% if stt -%}
| Speech to text with Whisper (tiny/base/small, multilingual, language detection, translate to English) over ONNX Runtime; mel spectrogram and decoding in Rust. Dictate into search and Ask, or transcribe an audio file. Turn on under Settings → Dictation | `desktop-core/src/stt.rs`, `desktop-core/src/audio.rs` | `stt` |
{% endif -%}
{% if email -%}
| Email over SMTP in Rust (SSL/TLS on 465, STARTTLS on 587, `AUTH PLAIN`/`LOGIN`, HTML, attachments) on the rustls already in the tree; the password stays in an owner-only file. Turn on under Settings → Email | `desktop-core/src/mail.rs` | `mail` |
{% endif -%}
{% if oauth -%}
| Sign-in with OAuth 2 / OpenID Connect: Google, GitHub, Microsoft or any OIDC provider (discovery). Authorization code + PKCE in the system browser, back through a loopback server on `127.0.0.1` (random port) or the app's deep link; `state`, `nonce` and the ID token's signature checked; tokens refreshed before they expire, revoked on sign-out; tokens and client secrets in the OS keychain only. Settings → Sign-in (see [Sign-in](#sign-in-oauth)) | `desktop-core/src/oauth.rs` | `oauth` |
{% endif -%}
{% if plugins -%}
| Plugin API: compiled-in extensions (`Plugin` trait + registry) with ingest hooks, extra search sources, their own methods (`plugin.<name>.<method>`) and a settings schema; example plugin `glossary`. Turn on under Settings → Plugins | `desktop-core/src/plugin.rs` | `plugins` |
{% endif -%}
{% if background_jobs -%}
| Background jobs: a persistent queue (survives restarts and crashes) of named job types with typed JSON payloads; run now, later, or on cron/interval schedules; a concurrency limit per queue; retries with exponential backoff and jitter, then dead letters you can retry; cancel, timeouts, a pruned run history. Respects `resources` and `net.offline`. Examples `backup_now` and `reindex`; Settings → Jobs | `desktop-core/src/queue.rs`, `desktop-core/src/job_types/` | `jobs-queue` |
{% endif -%}
{% if rclone -%}
| Cloud sync of the backups (or data) folder to any cloud with rclone: pinned, checksummed binary downloaded on first use; remote config in an owner-only file; schedule, direction, bandwidth limit. Settings → Cloud sync | `desktop-core/src/rclone.rs` | `rclone` |
{% endif -%}
{% if tailscale -%}
| Tailscale: status, and `tailscale serve` to reach a local MCP/HTTP port from your tailnet (never Funnel, never installs anything). Settings → Tailnet | `desktop-core/src/tailscale.rs` | `tailscale` |
{% endif -%}
{% if http_api -%}
| Local REST API (Axum): health, features, status, search, settings, documents{% if ai %}, ask{% endif %}{% if background_jobs %}, jobs{% endif %} over HTTP for scripts, agents and browser UIs; 127.0.0.1 only, bearer token (owner-only file), no CORS until you list origins, request IDs, timeout, body limit. Turn on under Settings → HTTP API | `desktop-core/src/http_api/` | `http-api` |
{% endif -%}
{% if webview -%}
| Embedded web views (desktop): allowlisted remote pages in the app's own windows, with no Tauri IPC for them, cookies kept per profile (or incognito), and an opt-in message bridge. Settings → Web views | `desktop-core/src/webview.rs`, `src-tauri/src/webview.rs` | `webview` |
{% endif -%}
{% if tray_menu -%}
| Tray / macOS menu-bar icon with its own menu, global shortcuts (rebind by pressing keys) and an app-defined menu next to the platform's App/Edit/Window menus; all three run actions through one registry. Settings → Tray & shortcuts | `desktop-core/src/actions.rs`, `src-tauri/src/tray.rs` | `tray-menu` |
{% endif -%}
{% if tauri_plugins -%}
| Official Tauri plugins (see the table below) | `src-tauri/src/lib.rs`, `src-tauri/capabilities/` | |
{% endif -%}
{% if telemetry -%}
| Opt-in telemetry with no SDKs: crash reports to Sentry (always kept locally in `crashes/`), anonymous usage events to PostHog or Aptabase, OpenTelemetry traces over OTLP/HTTP. Off until the user switches it on under Settings → Privacy; keys come from build-time secrets | `desktop-core/src/telemetry.rs` | `telemetry` |
{% endif -%}
{% if single_instance -%}
| Single instance: a second launch focuses the running window (file lock + loopback knock, std only) | `src-tauri/src/single.rs` | `single-instance` |
{% endif -%}
| Security defaults: navigation limited to the app's own pages, strict CSP, frozen JS prototypes, no global Tauri object with React, dev tools in debug builds only (`APP_DEVTOOLS=1` opens them) | `src-tauri/src/guard.rs`, `tauri.conf.json` | always |
| JSON-lines logs per process in the app data `logs/` folder, rotated by size, home folder written as `~`; level in Settings → Logs or `APP_LOG=warn,desktop_core=debug` | `desktop-core/src/logging.rs` | always |
{% if mcp_server -%}
| MCP server in Rust (`app --mcp`, stdio): search, status, ask; write tools only when allowed; per-tool allow/ask/deny, where "ask" waits for Approve in Settings (refused when the app is closed); audit log. Settings → MCP server → Add to Claude Desktop | `desktop-core/src/mcp.rs`, `src-tauri/src/mcp.rs` | `mcp` |
{% endif -%}
{% if pdf -%}
| PDF text (docling.rs) with tesseract OCR fallback | `desktop-core/src/pdf.rs` | `pdf` |
{% endif -%}
{% if frontend == "react" -%}
| React 19 + Vite frontend | `ui-react/` | |
{% else -%}
| Plain HTML/JS frontend (no build step) | `ui/` | |
{% endif -%}
{% if ipc_worker -%}
| Backend in a separate worker process over local IPC (Unix socket / named pipe) | `src-tauri/src/{service,ipc}.rs` | `ipc-worker` |
{% if background_agent -%}
| Background agent: macOS LaunchAgent, Windows Run entry, Linux systemd user unit | `src-tauri/src/agent.rs` | `background-agent` |
{% endif -%}
{% if daemon -%}
| Daemon: `<app> --daemon` runs the backend headless (SIGTERM / ctrl-c / Windows stop, pid and health file, crash backoff); `<app> --service install\|uninstall\|start\|stop\|restart\|status\|logs [--system]` makes it a LaunchAgent/LaunchDaemon, systemd user/system unit or Windows Service that restarts on failure. Settings → Service | `src-tauri/src/daemon.rs`, `desktop-core/src/daemon.rs` | `daemon` |
{% endif -%}
{% endif -%}
{% if python_example -%}
| Python worker example (child process, JSON over stdio) | `python/`, `src-tauri/src/python.rs` | `python-example` |
{% endif -%}
{% if wizard -%}
| First-run wizard (store choice, model download with progress) | `ui/`, `run_setup` in `src-tauri/src/lib.rs` | `wizard` |
{% endif -%}
{% if updater -%}
| Auto-update from GitHub releases (signed) | `src-tauri/src/update.rs` | `updater` |
{% endif -%}
{% if claude_mcp -%}
| Bundled uv + Claude Desktop MCP registration | `src-tauri/src/claude.rs` | `claude-mcp` |
{% endif -%}
{% if mobile -%}
| iOS and Android from the same code (Tauri 2 mobile): file store, backend in the app, store updates; `Mobile` workflow builds an unsigned APK/AAB and an iOS simulator app | `src-tauri/tauri.{android,ios}.conf.json`, `src-tauri/capabilities/mobile.json`, `.github/workflows/mobile.yml` | always |
{% endif -%}
| `--self-test` (CI runs it on every installed build) | `self_test` in `src-tauri/src/lib.rs` | always |
{% if installers -%}
| Release workflow: unsigned .dmg{% if macos_pkg %}, .pkg{% endif %}, .exe{% if linux %}, .AppImage, .deb{% endif %}, tested after install | `.github/workflows/release.yml` | |
{% endif -%}
{% if install_scripts -%}
| One-line installers (`curl … \| sh`, PowerShell) | `scripts/` | |
{% endif %}
The default features are the parts you picked when generating. Turn one off for a build
with `--no-default-features --features ...`.
{% if tauri_plugins %}
### Tauri plugins

Chosen when the app was generated. Each is a crate in `src-tauri/Cargo.toml`, a
`.plugin(...)` call in `src-tauri/src/lib.rs`, permissions in `src-tauri/capabilities/`
(`desktop.json` for desktop-only ones){% if frontend == "react" %} and a package in `ui-react/package.json`{% else %}; the plain frontend
reaches them as `window.__TAURI__.<plugin>`{% endif %}. Widen a permission set only as far as you need.

| Plugin | Rust crate | JS package | Permissions |
| --- | --- | --- | --- |
{% if tauri_dialog -%}
| File and message dialogs | `tauri-plugin-dialog` | `@tauri-apps/plugin-dialog` | `dialog:default` |
{% endif -%}
{% if tauri_fs -%}
| File system (scoped) | `tauri-plugin-fs` | `@tauri-apps/plugin-fs` | `fs:default` (app folders; add scopes for more) |
{% endif -%}
{% if tauri_clipboard -%}
| Clipboard | `tauri-plugin-clipboard-manager` | `@tauri-apps/plugin-clipboard-manager` | read and write text |
{% endif -%}
{% if tauri_notification -%}
| Notifications | `tauri-plugin-notification` | `@tauri-apps/plugin-notification` | `notification:default` |
{% endif -%}
{% if tauri_opener -%}
| Open URLs and files, reveal in folder | `tauri-plugin-opener` | `@tauri-apps/plugin-opener` | `opener:default` (http(s), mailto, tel) |
{% endif -%}
{% if tauri_store -%}
| Key-value store | `tauri-plugin-store` | `@tauri-apps/plugin-store` | `store:default` |
{% endif -%}
{% if tauri_global_shortcut -%}
| Global shortcuts (desktop) | `tauri-plugin-global-shortcut` | `@tauri-apps/plugin-global-shortcut` | register, unregister, is-registered |
{% endif -%}
{% if tauri_autostart -%}
| Start at login (desktop) | `tauri-plugin-autostart` | `@tauri-apps/plugin-autostart` | `autostart:default` |
{% endif -%}
{% if tauri_deep_link -%}
| Deep links (`{{project-name}}://`, set in `tauri.conf.json`) | `tauri-plugin-deep-link` | `@tauri-apps/plugin-deep-link` | `deep-link:default` |
{% endif -%}
{% if tauri_window_state -%}
| Remember window size and position (desktop) | `tauri-plugin-window-state` | `@tauri-apps/plugin-window-state` | `window-state:default` |
{% endif -%}
{% if tauri_os -%}
| OS information | `tauri-plugin-os` | `@tauri-apps/plugin-os` | `os:default` |
{% endif -%}
{% if tauri_process -%}
| Exit and restart (desktop) | `tauri-plugin-process` | `@tauri-apps/plugin-process` | `process:default` |
{% endif -%}
{% if tauri_log -%}
| Frontend logs into the app's log files | `tauri-plugin-log` | `@tauri-apps/plugin-log` | `log:default` |
{% endif -%}
{% endif %}
## Develop

```sh
make setup       # once: Tauri CLI + pre-commit hooks
make dev         # run the app with hot reload
make test        # tests (no downloads)
make lint        # fmt, clippy, linters, pre-commit hooks
make self-test   # what CI checks on every installer
make help        # everything else (db-up, docker-test, build, …)
```

`compose.yaml` has an optional Postgres + pgvector (`make db-up`, then
`APP_PG_URL=postgres://app:app@localhost:5433/app`) and a Linux build container.
See [CONTRIBUTING.md](CONTRIBUTING.md) for commits, PRs and releases.
[docs/GUIDE.md](docs/GUIDE.md) explains the architecture, how to add a feature, how settings
work, and where each module is documented (`cargo doc --workspace --no-deps --open`).

Linux needs `libwebkit2gtk-4.1-dev librsvg2-dev`.
{% if frontend == "react" -%}
The UI is React in `ui-react/`; `cargo tauri dev` starts Vite with hot reload, and
`cargo tauri build` builds it first. Build it once (`npm --prefix ui-react ci --include=dev && npm --prefix ui-react run build`)
before a plain `cargo build`, which embeds `ui-react/dist`.
{% else -%}
The UI is plain HTML/JS in `ui/` (no build step); point `build.frontendDist` in
`src-tauri/tauri.conf.json` at another framework's output to swap it.
{% endif -%}
{% if mobile %}
## Mobile (iOS and Android)

The app crate is also a library (`staticlib`, `cdylib`, `rlib`) whose `run` is the
mobile entry point, so the same Rust and the same UI ship to phones and tablets.
Parts that need a desktop are compiled out there with `cfg(desktop)`: the worker process
and background agent (the backend runs inside the app), the updater (the App Store and
Google Play update the app), single instance, the stdio MCP server, Python, Claude
Desktop, folder watching and embedded Postgres (the file store is used; a saved Postgres
choice falls back to it). `app_info` reports `features.mobile`, and the UI hides what is
not there. Everything is kept in the app's private folder.
{% if background_jobs -%}
Background jobs run only while the app is in the foreground there; due jobs and missed
schedules catch up when it opens or comes back.
{% endif -%}

Once, install the Android SDK and NDK (Android Studio; set `ANDROID_HOME` and `NDK_HOME`)
and Java 17, or Xcode on a Mac, then the Rust targets and the native projects:

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
cargo tauri android init      # makes src-tauri/gen/android
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo tauri ios init          # macOS only: makes src-tauri/gen/apple
```

Run on an emulator, simulator or a connected device with hot reload (the dev server
listens on your computer's address, `TAURI_DEV_HOST`):

```sh
cargo tauri android dev
cargo tauri ios dev           # add --open to work in Xcode
```

Build for the stores:

```sh
cargo tauri android build --aab           # Google Play (an .apk too with --apk)
cargo tauri ios build --export-method app-store-connect   # an .ipa for App Store Connect
```

Store builds must be signed: an upload keystore for Android
([guide](https://v2.tauri.app/distribute/sign/android/)) and a certificate with a
provisioning profile, or automatic signing with `APPLE_DEVELOPMENT_TEAM`, for iOS
([guide](https://v2.tauri.app/distribute/sign/ios/)). The **Mobile** workflow
(`.github/workflows/mobile.yml`, by hand and on tags) builds them unsigned and says where
the signing secrets go. Android package names cannot contain `-`, so
`tauri.android.conf.json` uses `{{identifier | replace: "-", "_"}}`. The ONNX parts
(embeddings, speech) and llama.cpp need native libraries built for each phone target;
check them on a device before turning them on in a mobile release.
{% endif -%}
{% if ipc_worker %}
## Processes

```
window (Tauri, UI)  ──local socket / named pipe──▶  worker (`<app> --worker`): store, model, Postgres
       │                                                    ▲
       └─ starts the worker if none answers                 └─ {% if background_agent %}or the background agent starts it at login{% else %}stops when the window closes{% endif %}
```

Every command goes through `service::Service`: `Local` in-process, `ipc::Remote` to the
worker (one JSON request per connection, progress lines streamed back). A worker from an
older version is replaced on first contact, so updates take effect. Turn the feature off
and the same code runs in the window's process.
{% if background_agent %}
The **Background** switch installs the agent: `~/Library/LaunchAgents/{{identifier}}.worker.plist`
(launchd keeps it alive), `HKCU\...\Run\{{identifier}}.worker` on Windows (no admin rights
needed), or a systemd user unit on Linux. With it on, closing the window leaves the
worker running.
{% endif -%}
{% if daemon %}
### Daemon and service

`<app> --daemon` is the worker as a proper daemon: no window, the store opened with the
saved settings, headless tasks started (the job queue's workers and scheduler when that
feature is built in), JSON-lines log in `<data>/logs/daemon.log`, pid and health in
`<data>/daemon.json`. SIGTERM, ctrl-c and the Windows service stop control stop it
cleanly. The window and `--mcp` use it through the same socket, and leave it running when
they close.

```
<app> --service install            # user level: LaunchAgent / systemd --user / Windows Run entry
<app> --service status             # installed, running, pid, log file (--json for tools)
<app> --service logs -n 100 --follow
<app> --service stop | start | restart | uninstall
sudo <app> --service install --system   # LaunchDaemon / system unit, runs as you, starts at boot
```

On Windows `--system` installs a Windows Service (from an Administrator terminal); it runs
as LocalSystem with its own data folder. Without the rights, the app prints the command
to run instead of asking for a password. launchd and systemd restart the daemon after a
failure (systemd with a growing delay), the Windows Service after 5 s, 30 s, then 2
minutes; the daemon itself waits longer after each crash in a row (up to 5 minutes).
Settings → Service installs or removes the user-level service. Installing it replaces the
background agent{% if background_agent %} (the Background switch){% endif %}; use one or the other.
From the app's folder, `kitchen service status` (and `install`, `logs` ...) runs the
same commands on the built binary.
{% endif -%}
{% endif -%}
{% if python_example %}
## Python example

`python/worker.py` runs as a long-lived child process; `python_call` sends it one JSON
line per request (`{"id", "method", "params"}`) and reads one reply line. It uses the
standard library only. The app finds Python through `APP_PYTHON`, {% if claude_mcp %}the bundled uv
(which downloads Python itself), {% endif %}then `python3` / `python` / `py -3`. Release builds ship
the script as a bundle resource. Tests: `python3 python/test_worker.py`.
{% endif -%}

App data (store, database, model cache) lives in the OS app-data folder for
`{{identifier}}`; settings in its config folder.

### Profiles

Settings → Profiles adds separate profiles (for example Work and Personal). Each has its own
settings, documents, index, keys, backups and logs. The default profile uses the app
folders as before; the others live in `profiles/<id>/` inside them. Switching restarts the
app into the other profile. `APP_PROFILE=<id>` opens one for a single run, and the MCP
server registers with the profile it was set up from.

### Settings

Search, ingest and watch settings live under `config` in `settings.json` (the Settings
screen edits them; `desktop-core/src/config.rs` documents every key). Everything optional
is off by default; values are clamped to safe ranges.

| Key | Default | |
|---|---|---|
| `search.mode` | `auto` | `vector`, `keyword`, or `hybrid` (weighted reciprocal rank fusion) |
| `search.topK`, `search.candidates` | 10, 50 | results, and candidates per list before fusing |
| `search.vectorWeight`, `search.keywordWeight`, `search.rrfK` | 1, 1, 60 | fusion |
| `search.minVectorScore`, `search.minKeywordScore`, `search.minScore` | none | thresholds per list and on the final score |
| `search.keywordEngine` | `auto` | Postgres: `bm25` (when built in), `fulltext` |
| `search.bm25Tokenizer` | `bert_base_uncased` | pg_tokenizer model |
| `ingest.workers` | 1 | parallel read/chunk/embed; 0 = one per core |
| `ingest.chunkChars`, `ingest.chunkOverlap` | 800, 0 | chunking |
| `ingest.extensions`, `ingest.maxFileMb` | common text types + pdf, 64 | what folder ingest takes |
| `watch.enabled` | false | auto-ingest from the drop folder (`Documents/<app> Inbox`) and `watch.paths` |
| `backup.intervalHours`, `backup.keep`, `backup.folder`, `backup.encrypt` | 0, 5, app data, off | scheduled backups; a synced folder keeps a copy off the computer |
| `resources.memoryBudgetMb` | 0 (none) | caps ingest workers and embedding batches by free memory inside the budget; files that would not fit are skipped with a reason |
| `resources.gpuBudget`, `resources.gpuLayers` | off, 0 | cap the local model's GPU layers (0 = CPU only) |
| `resources.onnxProvider` | `cpu` | preferred ONNX Runtime provider (`auto`, `coreml`, `directml`, `cuda`); falls back to the CPU when not built in |
{% if ai -%}
| `llm.cache.enabled`, `llm.cache.ttlHours`, `llm.cache.maxEntries`, `llm.cache.similarity` | off, 24, 500, 0.95 | answer cache (`answer-cache.jsonl`): same question, or a near-identical one by cosine with embeddings; emptied on ingest, remove and clear |
{% endif -%}
{% if mcp_server -%}
| `mcp.policies`, `mcp.approvalTimeoutSecs` | empty (allow), 60 | `{"ask": "ask", "*": "allow"}`: per-tool `allow`, `ask` or `deny`; `allowWrite` and `tools` still apply first |
{% endif -%}
{% if email -%}
| `mail.enabled`, `mail.host`, `mail.port`, `mail.security` | off, empty, 587, `starttls` | SMTP server; `tls` for port 465; `none` only to this computer |
| `mail.username`, `mail.from`, `mail.maxMb`, `mail.timeoutSecs` | empty, empty, 25, 30 | login (empty = none), sender, largest message, timeout |
{% endif -%}
{% if oauth -%}
| `oauth.enabled`, `oauth.redirect` | off, `loopback` | allow sign-in; `deepLink` returns through `{{project-name}}://oauth/callback` (needs the deep-link plugin) |
| `oauth.providers` | none | `[{"kind": "google", "clientId": "...", "scopes": []}]`; kinds `google`, `github`, `microsoft` (+ `tenant`), `oidc` (+ `issuer`); `id` defaults to the kind |
| `oauth.timeoutSecs`, `oauth.refreshBeforeSecs` | 300, 120 | time to finish in the browser; refresh the access token this early |
{% endif -%}
{% if plugins -%}
| `plugins.enabled`, `plugins.disabled`, `plugins.settings` | off, none, `{}` | run plugins; leave some out; each plugin's settings by name |
{% endif -%}
{% if background_jobs -%}
| `jobs.enabled`, `jobs.workers`, `jobs.maxAttempts` | off, 2, 5 | run jobs and schedules; jobs at once (capped by `resources.cpuPercent`); attempts before the dead letters |
| `jobs.backoffBaseSecs`, `jobs.backoffMaxSecs`, `jobs.historyDays`, `jobs.queues` | 10, 3600, 7, `{}` | retry waits double from the base up to the max (half of each is random jitter); history kept; per-queue limits, e.g. `{"maintenance": 1}` |
{% endif -%}
{% if rclone -%}
| `rclone.enabled`, `rclone.remote`, `rclone.path` | off, empty, empty | remote name in the rclone config, folder on it |
| `rclone.folder`, `rclone.direction` | `backups`, `push` | `data` (up only); `pull`, `mirror` (deletes extras there), `bisync` |
| `rclone.intervalMinutes`, `rclone.bandwidth` | 0, empty | 0 = only when asked (else at least 5); `--bwlimit`, e.g. `2M` or `10M:1M` |
{% endif -%}
{% if daemon -%}
| Settings → Service (not a saved key) | off | installs or removes the user-level service; the system level needs `sudo <app> --service install --system` (or an Administrator terminal) |
{% endif -%}
{% if tray_menu -%}
| `tray.enabled`, `tray.items` | off, show/hide · search · clipboard · pause · settings · quit | each item `{id, label, action, shortcutHint?, enabled}`; `action` `separator` draws a line |
| `tray.leftClickOpens`, `tray.closeToTray`, `tray.showInMenuBarOnly` | on, off, off | left click shows the window; closing hides it; macOS: no Dock icon |
| `shortcuts` | none | `[{accelerator, action}]`, e.g. `CmdOrCtrl+Shift+K`; an empty accelerator turns off a code default |
| `menu.title`, `menu.extra` | `Actions`, none | items added after the ones the code defines |
{% endif -%}
{% if tailscale -%}
| `tailscale.enabled`, `tailscale.serve`, `tailscale.port`, `tailscale.httpsPort` | off, off, 0, 443 | share `127.0.0.1:<port>` at `https://<machine>.<tailnet>.ts.net` |
{% endif -%}
{% if http_api -%}
| `httpApi.enabled`, `httpApi.host`, `httpApi.port` | off, `127.0.0.1`, 0 | the local REST API; port 0 = any free port (Settings shows the URL) |
| `httpApi.corsOrigins`, `httpApi.maxBodyKb`, `httpApi.timeoutSecs` | none, 1024, 30 | browser origins allowed (`https://app.example.com`), largest body, request timeout |
{% endif -%}
{% if webview -%}
| `webview.enabled`, `webview.allow`, `webview.blocked` | off, empty, `refuse` | open remote pages in the app; hosts shown (`example.com`, `*.example.com`; never `*`); `browser` hands other pages to the system browser |
| `webview.isolation`, `webview.allowHttp`, `webview.maxWindows` | `profile`, off, 8 | cookies per profile or `incognito`; plain HTTP on allowed hosts; window limit |
| `webview.bridge`, `webview.bridgeHosts` | off, empty | `window.kitchenBridge.post(m)` for pages on these (allowed) hosts |
{% endif -%}
| `net.offline` | false | block every request that leaves the computer (a local model server still works) |
| `net.allow`, `net.deny` | empty | host lists; `*.example.com` covers subdomains; every redirect hop is checked |
| `net.proxy`, `net.noProxy` | system | empty = `HTTPS_PROXY`/`ALL_PROXY`, `direct`, or an `http://`/`socks5://` URL |
| `net.caFiles` | empty | PEM files trusted instead of the bundled Mozilla roots (company TLS inspection) |
| `net.pins`, `net.pinsOff` | none | `{"updates.example.com": ["<PEM or file>"]}`: only those CAs may vouch for the host |

Backups are one JSON-lines file each (a manifest with a SHA-256, then the chunks with their
provenance). Restoring checks the file first, backs up the index it replaces, and embeds the
chunks again with the current model, so a backup moves between models, stores and computers.
Before a Postgres schema migration the old rows are kept in `chunks_before_migration`.
With `backup.encrypt` on, backups are sealed with ChaCha20-Poly1305 under a random key kept
owner-only in the data folder (ring, already in every build). Settings shows it as a
recovery key; restoring on another computer asks for it.

{% if email -%}
Email (`desktop_core::mail::send`) speaks SMTP itself: it refuses a password over an
unencrypted link, checks certificates against the bundled Mozilla roots (or `net.caFiles` and
`net.pins`), and rejects line breaks in addresses and headers. The `send_mail` command takes
`{to, cc, bcc, subject, text, html?, attachments: [path]}`; Settings → Email has a test button.
{% endif -%}
{% if oauth -%}
<a id="sign-in-oauth"></a>
#### Sign-in (OAuth 2 / OpenID Connect)

Users sign in in their own browser (never a web view), so the app never sees a password and
saved sign-ins and passkeys work. `desktop_core::oauth` runs the authorization code flow
with PKCE (S256): the `oauth_login` command opens the provider's page, waits on
`http://127.0.0.1:<random port>/callback` (or the deep link), checks `state`, swaps the code
for tokens, verifies the ID token (signature from the provider's keys, issuer, audience,
expiry, `nonce`) and stores the tokens in the OS keychain (macOS Keychain, Windows Credential
Manager, Secret Service on Linux) under `{{identifier}}`. Nothing secret is written to
settings or logs. `oauth_status` lists who is signed in, `oauth_logout` revokes the token
where the provider can and forgets it, and `oauth_client_secret` keeps an optional client
secret in the keychain. Rust code gets a fresh token with `Oauth::access_token`, which
refreshes it shortly before it expires; a background thread does the same every minute.

Set it up in Settings → Sign-in: turn it on, add a provider with its client id, save, then
press **Sign in**. Register the app with the provider as a *desktop* (native, public) client:

- **Google**: Google Cloud console → APIs & Services → Credentials → Create credentials →
  OAuth client ID → *Desktop app*. Copy the client id; Google also gives desktop apps a client
  secret that it requires with PKCE (it is not really secret): save it with **Save secret**.
  No redirect URI needs registering (any `127.0.0.1` port works). Scopes default to
  `openid email profile`; refresh tokens come with offline access, which is requested.
- **GitHub**: Settings → Developer settings → OAuth Apps (or GitHub Apps) → New. Set the
  callback URL to `http://127.0.0.1/callback` (GitHub accepts any port for loopback URLs).
  GitHub needs the client secret at the token step, so save it with **Save secret**; it also
  lets sign-out revoke the token. GitHub is OAuth 2 only: the user comes from
  `api.github.com/user`. Scopes default to `read:user user:email`. OAuth App tokens do not
  expire; GitHub App tokens expire after 8 hours and are refreshed.
- **Microsoft** (personal and work accounts): Entra admin center → App registrations → New
  registration. Under Authentication add the platform *Mobile and desktop applications* with
  the redirect URI `http://127.0.0.1/callback` (Entra ignores the port of loopback URIs), and
  turn on *Allow public client flows*. No secret is needed. `tenant` is `common` by default
  (any account); use `organizations`, `consumers` or your tenant id to narrow it. Scopes
  default to `openid email profile offline_access`. Microsoft has no revocation endpoint, so
  signing out forgets the tokens on this computer.
- **Any OpenID Connect provider** (Okta, Auth0, Keycloak, Authentik, Zitadel, ...): create a
  native or public client with PKCE and the redirect URI `http://127.0.0.1/callback` (allow
  any port if the provider asks), then set `kind` to `oidc` and `issuer` to its issuer URL;
  the endpoints come from `<issuer>/.well-known/openid-configuration`. Sign-out revokes the
  token when the provider lists a `revocation_endpoint`.

{% if tauri_plugins %}{% if tauri_deep_link %}With `oauth.redirect` set to `deepLink`, register `{{project-name}}://oauth/callback` as the
redirect URI instead; the deep-link plugin hands the reply back to the app and an
`oauth-changed` event reports the result.
{% endif %}{% endif %}On Linux the Secret Service needs a running keyring (GNOME Keyring or KWallet); without one
Settings shows the keychain error and sign-in is not saved. Every request follows `net`.

{% endif -%}
{% if plugins -%}
Plugins (`desktop_core::plugin`) are Rust types compiled into the app, never loaded from
disk. Implement `Plugin` (only `name` and `version` are required), add it to `builtin()`,
and it can rewrite or drop chunks before they are stored, add results to every search
(interleaved with the store's), answer `plugin.<name>.<method>` (the `plugin_call`
command), and describe its settings with a JSON Schema that Settings → Plugins turns into
a form. The `glossary` example shows each hook.

{% endif -%}
{% if background_jobs -%}
### Background jobs

`desktop_core::queue` keeps a queue in `<data>/queue/` (`queue.json`, written atomically,
and `history.jsonl`). A job type implements `Job` (a name, a payload type, `run`; optionally
its queue, timeout, attempts and whether it needs the network) and is registered in
`desktop-core/src/job_types/mod.rs`; `kitchen generate job <name>` writes one with docs and a
test. Jobs run in the backend: in the app, or in the worker with `ipc_worker`; with
`background_agent` the worker starts at login and schedules keep running while the window is
closed. On iOS and Android jobs run only while the app is in the foreground (no
`BGTaskScheduler`/`WorkManager`); what came due meanwhile runs when it is back, and a missed
schedule fires once. Commands: `jobs_list`, `jobs_enqueue` {kind, payload, options},
`jobs_cancel`, `jobs_retry`, `jobs_schedules`, `jobs_set_schedule`. The queue is built on std
threads plus [croner](https://crates.io/crates/croner) for cron (apalis was considered; its
persistent stores need SQL or Redis and an async runtime, see the module docs).

{% endif -%}
{% if rclone -%}
Cloud sync downloads rclone (the version pinned in `desktop-core/src/rclone.rs`, checked by
SHA-256) into the data folder on first use, through the same network policy. Paste the
output of `rclone config show` into Settings → Cloud sync: it is kept in an owner-only file
and never synced. Data-folder sync leaves out keys, passwords, models, logs and Postgres.

{% endif -%}
{% if tray_menu -%}
### Tray, shortcuts and menus

The tray (a template image in the macOS menu bar), global shortcuts and the app menu all
name actions by id and run them through one registry (`desktop_core::actions`):
`show_hide`, `show`, `hide`, `search` (focus the search box), `ingest_clipboard`,
`ingest_drop_folder`, `pause_resume` (ingest and folder watching, for this session),
`settings` and `quit`. The app's own actions emit `app-action` to the window and call the
backend's `action` method (`src-tauri/src/service.rs`). Add one with
`kitchen generate menu export_csv --label "Export CSV" --shortcut CmdOrCtrl+Shift+E`, or
give an action a default global shortcut with `kitchen generate shortcut search CmdOrCtrl+Alt+F`.
Changes in Settings apply at once. An unknown action, a bad accelerator, two actions on the
same keys, or keys another app already holds are listed in Settings, not fatal. On macOS
the standard App, Edit and Window menus stay (copy and paste keep working); Windows and
Linux get a menu bar only when the app menu has items. Clipboard text is read with
`pbpaste`, PowerShell's `Get-Clipboard`, or `wl-paste`/`xclip`/`xsel` on Linux.

{% endif -%}
{% if tailscale -%}
Tailscale is never installed by the app: without the `tailscale` CLI, Settings → Tailnet
links to <https://tailscale.com/download>. With it, **Apply sharing** runs
`tailscale serve --bg --https=<port> http://127.0.0.1:<local port>`, reachable only inside
your tailnet; turning it off removes only that mapping.

{% endif -%}
{% if http_api -%}
### HTTP API

Turn it on under Settings → HTTP API, which shows the URL and the token (with copy buttons).
The token is made on first use and kept owner-only in `<data>/api-token`; every request
needs it, and a release build refuses to start the API without one. It listens on
`127.0.0.1` only unless you change `httpApi.host`, and browsers can call it only from the
origins listed in `httpApi.corsOrigins`. Errors always have one shape:
`{"error": {"code", "message", "requestId", "fields"?}}`, and `requestId` matches the
`x-request-id` header and the log lines.

```sh
API=http://127.0.0.1:<port>; TOKEN=$(cat "<data folder>/api-token")
curl -H "Authorization: Bearer $TOKEN" $API/health
curl -H "Authorization: Bearer $TOKEN" $API/v1/features
curl -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"source": "notes.md", "text": "The heart pumps blood."}' $API/v1/documents
curl -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"query": "heart", "page": 1, "perPage": 5}' $API/v1/search
curl -H "Authorization: Bearer $TOKEN" $API/v1/settings > settings.json   # edit, then:
curl -X PUT -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     --data @settings.json $API/v1/settings
curl -X DELETE -H "Authorization: Bearer $TOKEN" "$API/v1/documents?source=notes.md"
{% if ai -%}
curl -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"question": "What does the heart do?"}' $API/v1/ask
{% endif -%}
{% if background_jobs -%}
curl -H "Authorization: Bearer $TOKEN" $API/v1/jobs
curl -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"kind": "<job type>", "payload": {}}' $API/v1/jobs
curl -X POST -H "Authorization: Bearer $TOKEN" $API/v1/jobs/<id>/cancel     # or /retry
{% endif -%}
```

`kitchen generate route <name>` adds `POST /v1/<name>` for a backend method (one made with
`kitchen generate command <name>`, for example). Routes live in `desktop-core/src/http_api/routes.rs`.

{% endif -%}
{% if webview -%}
### Embedded web views

With `webview.enabled` on, the app can show remote pages in its own windows (`web-<name>`).
Only hosts on `webview.allow` load; every other navigation, `window.open` and (on macOS and
Linux) frame is refused, or with `webview.blocked = "browser"` opened in the system browser.
Remote pages get **no Tauri IPC**: `capabilities/webview.json` grants `web-*` windows nothing,
Tauri's ACL refuses commands from remote origins, and the app refuses any command from a
`web-*` window before it runs. The app's own window keeps its CSP (`tauri.conf.json`).

```json
"webview": { "enabled": true, "allow": ["docs.rs", "*.github.com"], "isolation": "profile" }
```

```js
const label = await invoke('webview_open', { url: 'https://docs.rs/tauri', name: 'docs', title: 'Docs' });
await invoke('webview_close', { name: 'docs' });
```

Cookies and storage: `profile` keeps a separate jar for each app profile (WebView2 user data
folder on Windows, a WebKitGTK context on Linux, a `WKWebsiteDataStore` per profile on macOS
14+; macOS 13 shares the system default, so use `incognito` there). `incognito` keeps nothing.
iOS and Android builds leave web views out.

The bridge is off by default. With `webview.bridge` on, pages on `webview.bridgeHosts` get
`window.kitchenBridge.post(message)`; the main window receives `webview-message` events
(`{label, host, data}`), and `invoke('webview_post', { name, message })` answers with a
`message` event whose `origin` is `"kitchen-app"`. Messages are JSON, at most 64 KB, and
never reach a command. Child webviews inside the main window need Tauri's `unstable`
feature, so web views open as separate windows.

{% endif -%}
Every request (model, voice and Postgres downloads, AI providers, telemetry) goes through
`desktop-core/src/net.rs` and obeys `net`. Child processes (the Python worker, uv) get the
proxy, the CA file and offline switches (`HF_HUB_OFFLINE`, `UV_OFFLINE`) in their environment.
{% if embeddings %}
`APP_EMBED_MODEL` (in `.cargo/config.toml`, or at run time) picks the model: one of
`nomic-embed-text-v1.5`, `bge-small-en-v1.5`, `all-MiniLM-L6-v2`, `multilingual-e5-small`.
Changing it re-embeds stored chunks on the next start.
{% endif -%}
{% if postgres %}
`APP_PG_URL` uses an existing Postgres with pgvector instead of the embedded one. The
embedded server (Postgres 16.16) runs on Apple Silicon Macs and x86-64 Windows and Linux.
On first use it installs one extension bundle (pgvector, pg_tokenizer, VectorChord-BM25)
for that platform, from `APP_PG_EXT_URL` in `.cargo/config.toml`, after checking its SHA-256.
By default that is the public [pg-extensions](https://github.com/adeelahmad/pg-extensions)
release, with its hashes pinned in `desktop-core/pg-ext.sha256`. To build your own, run
**Actions → Postgres extensions → Run workflow** with *publish* on: it builds the bundle on
macOS, Windows and Linux, smoke-tests it on a real server and attaches it to this repo's
`pg-ext-16.16.0-r1` release (versions in `scripts/pg-ext/versions.env`). Then point
`APP_PG_EXT_URL` at it and paste its `SHA256SUMS` into `desktop-core/pg-ext.sha256`. The
release must download without a login. `APP_PG_EXT_DIR` installs it from a local folder instead.
pgvector there is built without `-march=native`, so no AVX-512 is needed.
The first server download calls the GitHub API; set `GITHUB_TOKEN` if you hit its rate limit.
{% if bm25 %}
VectorChord-BM25 is dual-licensed AGPLv3 / Elastic License 2.0 (pg_tokenizer is Apache-2.0).
If you ship it, follow one of them: ELv2 forbids offering it as a managed service; AGPLv3
needs a source offer for it. Not legal advice.
{% endif %}
{% endif %}
## Versions and changelog

Commits follow [Conventional Commits](https://www.conventionalcommits.org) (checked by a
commit-msg hook and on PR titles). [release-please](https://github.com/googleapis/release-please)
turns them into a release PR that bumps the version (semver) everywhere it appears and
writes `CHANGELOG.md`.
{% if installers %}
## Release

1. Merge the release PR. That tags `v<version>` and creates the GitHub release; the
   **Release** workflow then builds each platform, installs it on a fresh runner, runs
   `--self-test`, and attaches the installers.
2. A manual tag `v<version>` (matching `tauri.conf.json`) does the same without release-please.
3. Running the Release workflow by hand makes a pre-release `v<version>-test.<n>`.

Repository **Settings > Actions > General > Workflow permissions** must be
**Read and write**, or the publish step gets a 403.

Builds are unsigned (no Apple or Windows certificates needed). `docs/install-unsigned.md`
is the release text that tells users how to open them{% if macos_pkg %}; the `.pkg` clears
macOS's quarantine flag so the app opens without prompts{% endif %}.
{% endif -%}
{% if updater %}
## Auto-update

```sh
cargo tauri signer generate -w ~/.tauri/{{project-name}}.key
```

1. Paste the printed **public** key into `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`.
2. Add repository secrets `TAURI_SIGNING_PRIVATE_KEY` (the file's contents) and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Tagged releases then carry signed update bundles and `latest.json`; installed apps check
`https://github.com/{{github_repo}}/releases/latest/download/latest.json` on start and
offer **Install and restart**. Without a key, builds still work and the app says updates
are not set up.
{% endif -%}
{% if claude_mcp %}
## Claude Desktop

Release builds bundle [uv](https://github.com/astral-sh/uv) as a sidecar. **Connect Claude
Desktop** copies it to the app-data folder and adds `mcpServers.{{mcp_server_name}}`
(running `uv tool run --from "{{mcp_package}}" {{mcp_server_name}}`) to
`claude_desktop_config.json`, backing the file up first. `APP_MCP_PACKAGE` at run time
overrides the package, e.g. to test a branch archive.
{% endif -%}
