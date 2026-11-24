# Developer guide

This guide is for people and AI agents who adopt this app and build on it. It explains how
the parts fit together, how to add a feature, how settings work, and where each module is
documented. The code is the reference: every module has a rustdoc page with a runnable
example, and this guide links to them.

```sh
cargo doc --workspace --no-deps --open                           # desktop-core and the app's public API
cargo doc --workspace --no-deps --document-private-items --open  # also the app's internal modules
```

The pages land in `target/doc/desktop_core/` and `target/doc/app_lib/`.

## Contents

- [Kitchen for web developers](#kitchen-for-web-developers)
- [Port an existing app (Electron or web)](#port-an-existing-app-electron-or-web)
- [Architecture](#architecture)
- [Processes and folders](#processes-and-folders)
- [How to add a feature](#how-to-add-a-feature)
- [How settings work](#how-settings-work)
- [Build-time app config](#build-time-app-config)
- [Command errors](#command-errors)
- [Tests](#tests)
- [Module index](#module-index)
- [Docs and checks](#docs-and-checks)
- [Feature dependencies](#feature-dependencies)
- [Kitchen CLI](#kitchen-cli)
- [Notes for AI agents](#notes-for-ai-agents)

## Kitchen for web developers

If you have built a web app, you already know how this one works. Here is how the pieces map:

| On the web | Here |
|---|---|
| `fetch('/api/search?q=...')` | `invoke('search', { query })` from `@tauri-apps/api/core`: your API call, but in-process and typed |
| a route / controller | a Tauri command in `src-tauri/src/lib.rs` (`kitchen generate command export_csv`) |
| your backend service | `desktop_core`: a plain Rust library with no Tauri in it, tested with `cargo test` |
| `.env` / `config/*.json` | `config/` (build-time, per environment and OS) and the user's `settings.json` (run time) |
| Postgres, embeddings, email, background jobs | one `kitchen enable postgres` / `embeddings` / `email` / `background_jobs` each; nothing to install or host |
| `npm run dev` / `npm test` / deploy | `kitchen start` / `kitchen test` / `kitchen package` + `kitchen publish` |
| Vite, ESLint, Prettier | the same, in `ui-react/`; `kitchen eject vite` shows where |

The UI is a normal Vite + React app in `ui-react/`, served with hot reload during
development and bundled into the binary for release. There is no Node at run time: the
backend is Rust, and the UI talks to it only through commands you register.

### Five-minute quickstart

```sh
cargo install --git https://github.com/adeelahmad/tauri-desktop-template desktop-kitchen
kitchen new notes --enable email          # a new app in ./notes
cd notes
kitchen doctor                            # what this machine is missing, with the fix for each
kitchen start --install                   # installs the Tauri CLI and npm packages if needed, then runs with hot reload
```

Edit `ui-react/src/App.jsx` and the window updates in place; edit Rust and the app
rebuilds and restarts. Then add a backend call:

```sh
kitchen generate command word_count       # src-tauri/src/word_count.rs, wired in, with a test
```

```js
const reply = await invoke('word_count', { input: { text } });
```

```sh
kitchen test                              # unit tests, then the real app over WebDriver
kitchen package --release                 # .dmg / .msi + .exe / .AppImage + .deb in ./packages
kitchen publish                           # shows the GitHub release it would make; --yes makes it
```

Nothing is hidden: `kitchen eject vite|tauri|cargo` prints where each tool's own config
lives (`ui-react/vite.config.js`, `src-tauri/tauri.conf.json`, `Cargo.toml`,
`.cargo/config.toml`) and exactly what Kitchen adds on top (mostly `KITCHEN_ENV` and the
bundle list). Every step Kitchen runs is printed first, so you can run it yourself.

## Port an existing app (Electron or web)

**Secure by default.** A Kitchen app starts locked down, and you open only what you need:

- The UI cannot reach the OS. There is no Node in the renderer and no `require`; it can
  call only the commands you register, and plugins only through the allowlist in
  `src-tauri/capabilities/*.json`.
- A strict Content Security Policy (`tauri.conf.json`): scripts only from the app,
  `object-src 'none'`, no framing, no remote navigation (`guard.rs` locks it).
- The backend is Rust: memory safe, no `eval`, no prototype pollution, and one network
  policy (`desktop_core::net`) for every outgoing request.
- Secrets stay out of the UI and out of `settings.json` (owner-only files in the data folder).
- A production build refuses to start without its required config (`requiredInProduction`).

**Map the parts.**

| Electron | Kitchen (Tauri 2) |
|---|---|
| main process (`main.js`) | `src-tauri/src/lib.rs` (`run`, the window, commands) + `desktop-core` (logic) |
| `ipcMain.handle('search', ...)` | `#[tauri::command] fn search(...)`, registered in `generate_handler!` |
| `preload.js` + `contextBridge.exposeInMainWorld` | not needed: the UI calls `invoke('search', ...)`; capabilities decide what is allowed |
| `ipcRenderer.invoke('search', q)` | `invoke('search', { query: q })` |
| `webContents.send('progress', x)` | `app.emit("progress", x)`; the UI uses `listen('progress', ...)` |
| `nodeIntegration`, `contextIsolation`, `sandbox` | nothing to turn off: the renderer never has Node |
| Node modules in main (`fs`, `child_process`, SQLite) | Rust crates or `desktop-core` modules; Tauri plugins (`fs`, `dialog`, `shell`) with scoped capabilities |
| electron-builder config | `bundle` in `tauri.conf.json`; `kitchen package` |
| electron-updater | the `updater` feature (signed updates from GitHub releases) |

**Steps.**

1. `kitchen new myapp --set frontend=react`.
2. Move the frontend over as it is: copy your `src/` into `ui-react/src/` and your
   dependencies into `ui-react/package.json`. Any Vite-compatible app works; for webpack or
   CRA, point the `beforeDevCommand` / `beforeBuildCommand` and `devUrl` / `frontendDist`
   in `tauri.conf.json` at your own scripts instead.
3. Replace each `ipcRenderer.invoke` with `invoke`, and move each `ipcMain.handle` body
   into a command (`kitchen generate command <name>`), with the logic in `desktop-core`.
4. Delete `preload.js`. For each plugin the UI needs, add its permission to a capability
   file, and nothing more.
5. Move `process.env` reads to `config/` (build-time) or Settings (run time).
6. `kitchen test`, then `kitchen package --release`.

On the roadmap: `kitchen migrate`, which will do steps 2 to 4 for a typical Electron app.

## Architecture

```text
 UI: ui-react/ (React) or ui/ (plain JS)
   │  invoke("search", { query, k })                 events: setup-progress, update-progress, ...
   ▼
 Tauri commands: src-tauri/src/lib.rs                (thin: arguments → JSON params)
   │  service.call("search", {"query": ..., "k": ...}, progress)
   ▼
 service::Service: src-tauri/src/service.rs          (one dispatch by method name)
   ├── Local: runs the backend in this process (default)
   └── ipc::Remote ──local socket──▶ `<app> --worker` ──▶ Local   (feature ipc-worker)
   ▼
 desktop_core::Engine: desktop-core/src/engine.rs    (store + embedder + config + journal)
   ├── chunk → embed → store                          ingest
   ├── store (+ fuse in hybrid mode)                  search
   ├── llm, tts, stt, mail, watch, backup, ...        optional parts, one module each
   └── net                                            every outgoing request, under one policy
```

Four layers, each with one job:

1. **UI.** Calls Tauri commands with `invoke` and listens to events. It knows which parts
   the build has from `app_info().features`, and hides what is missing.
2. **Tauri commands** (`src-tauri/src/lib.rs`). Each command turns its arguments into a JSON
   object and calls the service by method name. Commands hold no logic, so the window, the
   worker, the MCP server and `--self-test` all share one backend.
3. **Service** (`src-tauri/src/service.rs`). `Local::dispatch` maps method names to
   `desktop_core` calls and owns the long-lived state: the engine, the folder watcher, and
   loaded voice and Whisper models. With feature `ipc-worker`, the window uses
   `ipc::Remote` instead. It sends the same calls to a separate `<app> --worker` process
   over a local socket, as newline-delimited JSON. The worker survives a crash of the
   window, and the background agent can start it at login. Because the service has one
   trait method (`call`), a command does not know which one it talks to.
4. **Engine** (`desktop-core`). A plain Rust library with no Tauri in it. It can be tested
   and used on its own: `cargo test -p desktop-core`, or call it from a CLI.

### Processes and folders

| Run as | What it is | Entry |
|---|---|---|
| `<app>` | the window | `app_lib::run` |
| `<app> --worker` | the backend in its own process (feature `ipc-worker`) | `app_lib::worker_main` → `ipc::serve` |
| `<app> --mcp` | MCP server over stdio for Claude Desktop, Claude Code and other clients (feature `mcp`) | `app_lib::mcp_main` → `mcp::main` |
| `<app> --self-test [out.json]` | indexes and searches a sample with every built-in store; CI runs it on the installed app | `app_lib::self_test` |

On iOS and Android (the `mobile` option) the same crate is the app, with only the first
row: the desktop-only parts (worker, agent, updater, single instance, `--mcp`) are compiled
out with `cfg(desktop)`.

Each process reads the same `settings.json` and data folder (`paths::app_dirs`). Each one
writes its own log in `<data>/logs/<process>.log`, as JSON lines. A profile (Settings →
Profiles, or `APP_PROFILE=<id>`) moves both folders to `profiles/<id>/`.

| Folder | Holds |
|---|---|
| config (`~/Library/Application Support/<id>`, `%APPDATA%\<id>`, `~/.config/<id>`) | `settings.json`, `profiles.json` |
| data (same on macOS and Windows; `~/.local/share/<id>` on Linux) | the store (`chunks.json` or Postgres), `models/`, `jobs/`, `backups/`, `logs/`, `keys.json`, `mail-password`, `backup.key`, `mcp-audit.jsonl`, `crashes/` |

## How to add a feature

Every feature is optional and **off by default**, in two ways. At build time it is a cargo
feature, chosen when the app is generated. At run time it is a switch in Settings that
starts off.

### With the Kitchen CLI (fastest)

```sh
kitchen generate module invoices --config                            # desktop-core/src/invoices.rs + settings section
kitchen generate setting invoices.due_days int 30 --label "Days until due"
kitchen generate command export_invoices                             # Tauri command + service method
kitchen generate page Invoices                                       # React page (ui-react)
kitchen generate route export_invoices                               # POST /v1/export_invoices on the HTTP API (feature http-api)
```

Each generator writes a documented, tested file and wires it in at the `// kitchen:...`
marker comments. Then fill in the logic, and run the checks in [Docs and checks](#docs-and-checks).

### By hand: the wiring checklist

Follow an existing feature end to end. `mail` (email over SMTP) is the reference: it has
a config section, a password kept outside the settings, service methods, commands and a
Settings section.

1. **Cargo feature.** Add `myfeat = [...]` to `desktop-core/Cargo.toml`, with its optional
   dependencies (`dep:...`). In `src-tauri/Cargo.toml`, add `myfeat = ["desktop-core/myfeat"]`,
   and add it to `default = [...]` if this build should have it. Add the name to the
   `unexpected_cfgs` values in the root `Cargo.toml`.
2. **Module.** Create `desktop-core/src/myfeat.rs` with a `//!` doc, a runnable example and
   unit tests. Declare it in `desktop-core/src/lib.rs` as
   `#[cfg(feature = "myfeat")] pub mod myfeat;` above `// kitchen:modules`.
3. **Settings.** Add a `MyfeatConfig` with `#[serde(default, rename_all = "camelCase")]`,
   `enabled: false` in its `Default`, and a `validated()` method that clamps values. Add
   the field to `Config` in `config.rs`, behind the same `#[cfg]`, and call
   `self.myfeat = self.myfeat.validated();` in `Config::validated`. See
   [How settings work](#how-settings-work).
4. **Engine (optional).** If the feature works on the index, add an `Engine` method.
   Otherwise the service can call the module directly.
5. **Service method.** In `src-tauri/src/service.rs`, add a
   `#[cfg(feature = "myfeat")] "myfeat_do" => ...` arm to `Local::dispatch`. Read its
   arguments with `arg::<T>(&params, "key")`. Refuse with a clear message when the
   feature's switch is off. List the method in the module doc.
6. **Tauri command.** In `src-tauri/src/lib.rs`, add a `#[tauri::command(async)]` function
   that calls `call(&state, "myfeat_do", json!({...}))`. Register it in
   `generate_handler!` behind `#[cfg(feature = "myfeat")]`, and add `"myfeat"` to
   `features()` so the UI can tell it is built in.
7. **MCP tool (optional).** To offer it to MCP clients, add a `Tool` in
   `src-tauri/src/mcp.rs`. Set `read_only: false` if it changes data. Write tools are only
   offered when the user allows them.
8. **UI.** In `ui-react/src/Settings.jsx`, add `FIELDS` rows
   (`['myfeat', 'enabled', 'Turn on ...', 'bool', { needs: 'myfeat' }]`), an entry in the
   `needs` map, and the section in `sections`. Add any screen of its own the same way the
   Ask, Speak and Dictate panels do.
9. **Docs.** Add a row to the README's feature table, and one for each new key in the
   settings table. Then add a line to the module index below.
10. **Network.** Any HTTP request goes through `desktop_core::net` (`get`, `get_bytes`,
    `download_to`, `post_json`, `agent`). That way, offline mode, the allow and deny lists,
    the proxy and the pins apply to it. A raw TCP or TLS connection (as in `mail`) calls
    `net::check_host` first.
11. **Secrets.** Never put keys or passwords in `settings.json`. Keep them owner-only in
    the data folder with `desktop_core::jobs::write_private`, the way `KeyFile`,
    `PasswordFile` and `BackupKey` do.

Keep the `// kitchen:...` marker comments where they are, because the generators insert
code at them.

## How settings work

The window keeps one JSON file per profile, `<config>/settings.json`:

```json
{
  "setupDone": true,
  "backend": "file",
  "embeddings": true,
  "config": {
    "search": { "mode": "hybrid", "topK": 10 },
    "ingest": { "workers": 0 },
    "llm": { "enabled": true, "provider": "ollama" }
  }
}
```

- `backend` and `embeddings` are chosen in the first-run wizard and fixed until the
  next setup. `config` is [`desktop_core::Config`](../desktop-core/src/config.rs): one
  section per area (`search`, `ingest`, `watch`, `resources`, `log`, `net`, `backup`), plus
  one per feature built in (`llm`, `mcp`, `tts`, `stt`, `mail`, `telemetry`).
- **Missing keys take their defaults** (`#[serde(default)]` on every struct). An old or
  hand-written file therefore always loads, and a new setting needs no migration. Keys are
  camelCase in JSON and snake_case in Rust.
- **`validated()` clamps every value** into a safe range when settings are loaded or
  changed. A bad value becomes a usable one, never an error.
- **Flow of a change.** The Settings screen calls `set_config({ config })`. The command
  validates the config and sends it to the service as `configure`. That runs
  `Engine::configure`, restarts the folder watcher, and applies the new log level and
  telemetry settings. The command then saves the applied config. Before setup, the saved
  config applies at the next setup. `set_config({ config: {} })` resets everything to the
  defaults.
- **Other processes** (`--worker`, `--mcp`) read the same file at start. The worker gets
  live changes through `configure`, like the in-process backend.
- **Secrets stay out of it.** API keys are in `keys.json`, the SMTP password is in
  `mail-password` and the backup key is in `backup.key`, all owner-only in the data folder.
  Environment variables such as `ANTHROPIC_API_KEY` override the key file.
- **Environment overrides** for a single run: `APP_PROFILE` (profile), `APP_LOG` (log level,
  for example `warn,desktop_core=debug`), `APP_EMBED_MODEL` and `APP_EMBED_DIM` (embedding
  model), `APP_PG_URL` and `APP_PG_EXT_DIR` (Postgres), `APP_DEVTOOLS=1` (debug builds).

The README's settings table lists every key with its default. The rustdoc of each config
struct documents every field.

## Build-time app config

Next to the user's settings, an app has its own configuration as the developer wrote it:
support URLs, defaults, per-OS shortcuts, staging versus production endpoints. It lives
in `config/` and is merged when the app is built:

```text
config/settings.json                      shared by every build
config/environments/development.json      KITCHEN_ENV (or RUN_MODE): development | production | test
config/environments/production.json
config/environments/test.json
config/platforms/mac.json                 the target OS: mac | linux | windows
config/platforms/linux.json
config/platforms/windows.json
config/local.json                         this machine only (gitignored)
APP_<SECTION>__<KEY>=value                at run time, e.g. APP_SEARCH__DEFAULT_TOP_K=5
```

Each layer overrides the one before it: objects merge key by key, and any other value
(arrays too) replaces. `src-tauri/build.rs` does the merge with the code in
[`config_layers`](../src-tauri/src/config_layers.rs) and compiles the result in. The
environment is `KITCHEN_ENV`: `kitchen start` sets `development`, `kitchen test` sets
`test`, `kitchen package --release` sets `production`. Without it, release builds are
`production` and debug builds `development`.

- **Rust:** `app_lib::app_config::get::<usize>("search.defaultTopK")`, `get_or(path, default)`,
  `app_config::env()`, or the whole value as `app_config::CONFIG` (`serde_json::Value`).
- **UI:** `app_info().config`; `ui-react/src/config.js` keeps it in `CONFIG` and reads a key
  with `configGet('ui.showEnvBadge', false)`. The sample shows an "development build" badge.
- **Environment variables** use `APP_` and `__` between levels; a key matches ignoring case and
  `_`, and the value is read as JSON when it parses. Only sections that already exist in
  `config/` can be overridden, so the app's other `APP_*` variables never collide.
- **Production safety:** list required paths in `requiredInProduction`, e.g.
  `["mail.host", "api.url"]`. A production build that lacks one exits at start with
  `missing required config: mail.host (set it in config/ or as APP_MAIL__HOST)`.
  Development and test only log a warning.

**Build-time config versus settings.** `config/` is yours: it is fixed per build, shared by
every user, and compiled into the binary, so never put secrets in it. `settings.json`
([How settings work](#how-settings-work)) belongs to the user: it changes at run time in the
Settings screen, one file per profile.

## Command errors

Commands reject the UI's `invoke` with one JSON shape:

```json
{ "code": "not_found", "message": "no document named notes.txt" }
```

`code` is a short, stable, snake_case word the UI can branch on: `invalid` (bad input),
`not_found`, `unavailable` (the feature is off or not set up), `denied` (a policy said no),
`failed` (anything else). `message` is for people. In Rust, return
`Result<T, app_lib::error::CommandError>`; `?` turns a `String` or `desktop_core::Error`
into code `failed`, and `CommandError::new("invalid", "...")` sets a code. In the UI,
`errorText(e)` and `errorCode(e)` from `ui-react/src/errors.js` read it (and the plain
strings that older commands still return).

## Tests

```sh
kitchen test              # both kinds below (feature tests are skipped, with the reason, if the machine can't run them)
kitchen test --unit       # cargo test --workspace (cargo nextest + doc tests when nextest is installed)
kitchen test --feature    # the real app, driven through its window
```

Both set `KITCHEN_ENV=test`, so the build embeds `config/environments/test.json`. If
`ui-react/package.json` has a `test` script, `--unit` runs it too; the template ships none,
so Node stays a build tool for the UI.

**Feature tests** live in [`e2e/`](../e2e): Rust tests that start the built app and drive
it with [tauri-driver](https://v2.tauri.app/develop/tests/webdriver/) over WebDriver, with
a small client in `e2e/tests/webdriver/mod.rs` (plain HTTP and JSON on `ureq`, a
dev-dependency only). Each test gets its own tauri-driver on free ports and its own scratch
home, so its own settings, data folder and store: tests run in parallel and never touch
your data. The sample adds a note, searches for it, and checks the hit.

Setup, once per machine:

| OS | Install |
|---|---|
| Linux | `cargo install tauri-driver --locked` and `sudo apt install webkit2gtk-driver` (plus `xvfb` on a machine without a display; Kitchen wraps the run in `xvfb-run`) |
| Windows | `cargo install tauri-driver --locked` and msedgedriver matching your Edge (`cargo install msedgedriver-tool && msedgedriver-tool`), on PATH |
| macOS | nothing: there is no WebDriver for WKWebView, so `kitchen test --feature` builds the app and runs its `--self-test` instead, and says so. Run the WebDriver tests in Linux or Windows CI |

`kitchen doctor` checks for tauri-driver and WebKitWebDriver (or msedgedriver). By hand,
`kitchen test --feature` is:

```sh
KITCHEN_ENV=test cargo tauri build --debug --no-bundle
APP_BINARY=target/debug/<app> cargo test -p e2e -- --ignored
```

## Module index

Paths are relative to this file. The rustdoc page of `desktop_core::X` is
`target/doc/desktop_core/X/index.html`, after `cargo doc`.

### desktop-core (the engine)

| Module | Feature | What it is for |
|---|---|---|
| [`lib`](../desktop-core/src/lib.rs) | always | Crate root: `Error`, `Result`, the `Embedder` trait, vector helpers, re-exports |
| [`engine`](../desktop-core/src/engine.rs) | always | `Engine`: open, ingest (text, files, folders, resumable), search, ask, backup and restore |
| [`config`](../desktop-core/src/config.rs) | always | `Config` and its sections; `validated()` |
| [`chunk`](../desktop-core/src/chunk.rs) | always | Text into chunks along paragraphs and sentences; `Provenance` |
| [`store`](../desktop-core/src/store.rs) | always | The `Store` trait, `FileStore` (JSON file), BM25 |
| [`fuse`](../desktop-core/src/fuse.rs) | always | Weighted reciprocal rank fusion for hybrid search |
| [`jobs`](../desktop-core/src/jobs.rs) | always | Crash-safe ingest journal; `write_atomic`, `write_private` |
| [`backup`](../desktop-core/src/backup.rs) | always | Portable backups (optionally encrypted) and restores |
| [`logging`](../desktop-core/src/logging.rs) | always | JSON-lines logs with rotation, through the `log` facade |
| [`net`](../desktop-core/src/net.rs) | always | The network policy and the one HTTP client |
| [`governor`](../desktop-core/src/governor.rs) | always | CPU share and background priority for ingest |
| [`audio`](../desktop-core/src/audio.rs) | always | WAV, PCM, resampling, base64 |
| [`embed`](../desktop-core/src/embed.rs) | `embed` | Local ONNX embeddings (fastembed) |
| [`pdf`](../desktop-core/src/pdf.rs) | `pdf` | PDF text, with OCR for pages without text |
| [`pgstore`](../desktop-core/src/pgstore.rs) | `postgres` (`bm25`) | Embedded Postgres + pgvector (+ BM25) store |
| [`watch`](../desktop-core/src/watch.rs) | `watch` | Drop folder and watched folders |
| [`llm`](../desktop-core/src/llm.rs) | `ai` | Chat client for cloud and local models: retries, fallback chain, budgets, prompt caching; API keys; RAG prompt |
| [`meter`](../desktop-core/src/meter.rs) | `ai` | AI cost meter: price table, per-day and per-month usage, budgets |
| [`cache`](../desktop-core/src/cache.rs) | `ai` | Semantic answer cache for `Engine::ask` |
| [`local`](../desktop-core/src/local.rs) | `llama` | llama.cpp built in (GGUF) |
| [`mcp`](../desktop-core/src/mcp.rs) | `mcp` | MCP protocol, tool policy and audit log |
| [`tts`](../desktop-core/src/tts.rs) | `tts` | Text to speech (tiny-voca) |
| [`stt`](../desktop-core/src/stt.rs) | `stt` | Speech to text (Whisper) |
| [`plugin`](../desktop-core/src/plugin.rs) | `plugins` | Compiled-in plugins: ingest hooks, extra methods and settings |
| [`rclone`](../desktop-core/src/rclone.rs) | `rclone` | Sync backups (or the data folder) to a cloud with rclone |
| [`tailscale`](../desktop-core/src/tailscale.rs) | `tailscale` | Tailscale status and `tailscale serve` for one local port |
| [`http_api`](../desktop-core/src/http_api/mod.rs) | `http-api` | Local REST API (Axum): bearer token, one error shape, validated bodies, pagination |
| [`mail`](../desktop-core/src/mail.rs) | `mail` | SMTP email with TLS, STARTTLS and attachments |
| [`oauth`](../desktop-core/src/oauth.rs) | `oauth` | OAuth 2 / OpenID Connect sign-in (PKCE, browser, keychain) |
| [`webview`](../desktop-core/src/webview.rs) | `webview` | Embedded web view policy: allowlist, isolation, message bridge |
| [`telemetry`](../desktop-core/src/telemetry.rs) | `telemetry` | Opt-in crash reports, analytics and traces |

A module only exists in your app if its feature was chosen when the app was generated
(`kitchen enable <feature>` adds it later).

### src-tauri (the app)

| Module | Feature | What it is for |
|---|---|---|
| [`lib`](../src-tauri/src/lib.rs) | always | Tauri commands, settings file, entry points (`run`, `worker_main`, `mcp_main`, `self_test`) |
| [`main`](../src-tauri/src/main.rs) | always | Command-line switch between the entry points |
| [`service`](../src-tauri/src/service.rs) | always | The `Service` trait and `Local`, the backend by method name |
| [`paths`](../src-tauri/src/paths.rs) | always | App folders and profiles |
| [`guard`](../src-tauri/src/guard.rs) | always | Navigation lock and dev tools policy for the window |
| [`app_config`](../src-tauri/src/app_config.rs) | always | Build-time app config (`config/`), `get::<T>("a.b")`, production safety check |
| [`config_layers`](../src-tauri/src/config_layers.rs) | always | The layer merge `build.rs` runs; `APP_A__B` overrides |
| [`error`](../src-tauri/src/error.rs) | always | `CommandError`, the `{ code, message }` shape of command errors |
| [`ipc`](../src-tauri/src/ipc.rs) | `ipc-worker` | The worker process and its local-socket protocol |
| [`agent`](../src-tauri/src/agent.rs) | `background-agent` | Start the worker at login (launchd, Run key, systemd) |
| [`mcp`](../src-tauri/src/mcp.rs) | `mcp` | `--mcp`: tools mapped to the service; client registration |
| [`update`](../src-tauri/src/update.rs) | `updater` | Signed auto-update from GitHub releases |
| [`single`](../src-tauri/src/single.rs) | `single-instance` | One window at a time |
| [`python`](../src-tauri/src/python.rs) | `python-example` | A Python child process behind a command |
| [`claude`](../src-tauri/src/claude.rs) | `claude-mcp` | Register a Python MCP server in Claude Desktop through bundled uv |
| [`webview`](../src-tauri/src/webview.rs) | `webview` | Remote pages in app windows: allowlist hook, isolation, no IPC, bridge |

## Docs and checks

The bar for every module:

- a `//!` doc saying what the module is for, how it fits in, and its main items;
- at least one runnable example. Use `no_run` only when the example needs the network, a
  model download or a device;
- `///` docs on every public item. `desktop-core` has `#![warn(missing_docs)]`, so clippy
  with `-D warnings` fails on a gap;
- unit tests for the logic.

Run these before a commit. Add `--features ...` to cover the parts you changed:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace                  # unit tests and doctests
cargo test -p desktop-core --doc        # doctests only
```

If you changed `ui-react`, also run `npx prettier --check src`, `npx eslint src` and
`npm run build` in that folder.

## Feature dependencies

Some features only work with others. `kitchen enable`, `disable` and `set` check these rules
before they render anything (add `--with-deps` to switch the dependencies too), and
`kitchen features --check` checks this app's `kitchen.toml`.

<!-- kitchen:feature-dependencies (generated; see kitchen/src/compat.rs) -->
Generated from `kitchen/src/compat.rs`; the template's pre-hook (`hooks/compat.rhai`) and
`kitchen enable` / `disable` / `set` / `features --check` apply the same rules.

| Feature | Needs | Why |
|---|---|---|
| `background_agent` | `ipc_worker` | it starts the worker process at login |
| `daemon` | `ipc_worker` | the daemon is the worker process run headless |
| `bm25` | `postgres` | VectorChord-BM25 is a Postgres extension |
| `local_llm` | `ai` | llama.cpp is the local provider of AI chat |
| `tauri_dialog`, `tauri_fs`, `tauri_clipboard`, `tauri_notification`, `tauri_opener`, `tauri_store`, `tauri_global_shortcut`, `tauri_autostart`, `tauri_deep_link`, `tauri_window_state`, `tauri_os`, `tauri_process`, `tauri_log`, `tauri_http`, `tauri_positioner`, `tauri_shell` | `tauri_plugins` | each Tauri plugin is wired in only with tauri_plugins |
| `macos_pkg` | `installers` | the release workflow builds it |
| `install_scripts` | `installers` | they download what the release workflow builds |
| `linux` | `installers` | the release workflow builds them |

Warnings (it still generates):

| Feature | Without | What is missing |
|---|---|---|
| `oauth` | `tauri_deep_link` | the deep-link redirect (oauth.redirect = deepLink) needs the deep-link plugin; without it sign-in uses the loopback redirect |
| `ipc_worker`, `background_agent`, `daemon`, `updater`, `single_instance`, `mcp_server`, `python_example`, `claude_mcp`, `watch_folders`, `postgres`, `http_api`, `webview`, `tray_menu`, `app_shell`, `tauri_global_shortcut`, `tauri_autostart`, `tauri_window_state`, `tauri_process`, `tauri_positioner`, `tauri_shell` | a desktop target (`mobile` on) | desktop only: compiled out on iOS and Android |

Licences checked for each feature (the pre-hook prints a notice for anything beyond MIT / Apache-2.0):

| Feature | Brings in | Notice |
|---|---|---|
| `postgres` | PostgreSQL 16 server and pgvector: PostgreSQL; postgresql_embedded: (MIT OR Apache-2.0) AND PostgreSQL | PostgreSQL and pgvector are under the PostgreSQL Licence (permissive, BSD-like): keep their copyright notice with the app |
| `bm25` | VectorChord-BM25: AGPL-3.0 OR Elastic-2.0; pg_tokenizer: Apache-2.0 | VectorChord-BM25 is AGPL-3.0 or Elastic License 2.0: pick one before you ship (ELv2 forbids offering it as a managed service; AGPL asks for your source) |
| `embeddings` | fastembed, hf-hub: Apache-2.0; ONNX Runtime: MIT; Oniguruma: BSD-2-Clause; models: Apache-2.0 (nomic, MiniLM) or MIT (BGE, E5) | the tokenizer's Oniguruma regex library is BSD-2-Clause: keep its copyright notice with the app |
| `pdf` | docling: MIT; docling-pdf: MIT AND BSD-3-Clause (Adobe mapping resources and glyph list); tesseract (optional, not bundled): Apache-2.0 | docling-pdf embeds Adobe's PDF mapping resources and glyph list (BSD-3-Clause): keep their licence text with the app |
| `watch_folders` | notify: CC0-1.0; notify-debouncer-full: MIT OR Apache-2.0 | notify is CC0-1.0 (public domain dedication): nothing to do |
| `local_llm` | llama.cpp: MIT; llama-cpp-2: MIT OR Apache-2.0 | none (MIT / Apache-2.0) |
| `tts` | ONNX Runtime: MIT; ort: MIT OR Apache-2.0; tiny-voca model: Apache-2.0 | none (MIT / Apache-2.0) |
| `stt` | ONNX Runtime: MIT; ort: MIT OR Apache-2.0; Whisper models: MIT | none (MIT / Apache-2.0) |
| `rclone` | rclone (downloaded on first use): MIT | none (MIT / Apache-2.0) |
| `daemon` | service-manager, signal-hook, windows-service: MIT OR Apache-2.0 | none (MIT / Apache-2.0) |
| `oauth` | keyring: MIT OR Apache-2.0 | none (MIT / Apache-2.0) |
| `ipc_worker` | interprocess: 0BSD OR Apache-2.0 | none (MIT / Apache-2.0) |
| `background_jobs` | croner: MIT; chrono: MIT OR Apache-2.0 | none (MIT / Apache-2.0) |
| `http_api` | axum: MIT; tokio: MIT | none (MIT / Apache-2.0) |
<!-- /kitchen:feature-dependencies -->

## Kitchen CLI

`kitchen` is the command-line companion to this template. It is a single Rust binary that
lives in [`kitchen/`](https://github.com/adeelahmad/tauri-desktop-template/tree/main/kitchen)
at the root of the template repository. It drives cargo-generate (Liquid templates), so it
needs neither Node nor a second template format. `kitchen.toml` in your app records the
answers it was generated with.

| Command | What it does |
|---|---|
| `kitchen new <name> --enable email,ai --disable postgres` | Create an app from the template |
| `kitchen enable <feature>` / `kitchen disable <feature>` | Turn a feature on or off later. Your edits are kept by a three-way merge; `--dry-run` shows what would change |
| `kitchen generate module\|setting\|command\|page\|route ...` | Generate documented, tested code, wired in at the `// kitchen:...` markers |
| `kitchen doctor` | Check the toolchain (Rust, cargo-generate, Tauri CLI, Node, WebKitGTK or Xcode, tauri-driver and WebKitWebDriver) and the app's markers |
| `kitchen start` (`dev`) | `cargo tauri dev` with `KITCHEN_ENV=development`: Vite HMR and Rust rebuilds. `--install` installs a missing Tauri CLI or npm packages |
| `kitchen test [--unit\|--feature]` | Unit tests and feature tests ([Tests](#tests)), with `KITCHEN_ENV=test` |
| `kitchen package [mac\|windows\|linux] [--release]` | `cargo tauri build` with this OS's bundles, copied into `./packages`. Other OSes are refused |
| `kitchen publish [--tag v1.2.3] [--yes]` | A GitHub release with `./packages` through `gh`; a dry run without `--yes` |
| `kitchen eject vite\|tauri\|cargo` | Where that tool's own config is, and what Kitchen sets on top |
| `kitchen mcp` | Offer the same commands to AI agents as an MCP server over stdio (`claude mcp add kitchen -- kitchen mcp`) |

Add `--json` to any command for machine-readable output. The CLI's own README has the
details, including `kitchen set`, `kitchen upgrade`, and how to override the generator
templates.

## Notes for AI agents

- [AGENTS.md](../AGENTS.md) is the short version of this guide for this build, and
  `.claude/skills/` has recipes for turning features on, generating code, porting an
  Electron or web app, and releasing.
- Start with this guide, then the `//!` doc of the module you are changing. Run
  `cargo doc --document-private-items` to read the app's internal modules.
- New work follows the [wiring checklist](#by-hand-the-wiring-checklist), or uses
  `kitchen generate`. Keep features off by default, both the cargo feature and the
  run-time switch.
- Keep commands thin: logic goes in `desktop-core`, dispatch goes in `service.rs`.
- Every request leaves through `desktop_core::net`. Secrets never go in `settings.json`.
- Reuse the crates already in the tree (`ureq` with rustls, `serde_json`, `thiserror`,
  `log`, `sha2`, `ring`) before you add a dependency.
- Do not remove the `// kitchen:...` markers.
- Finish with the checks in [Docs and checks](#docs-and-checks).
