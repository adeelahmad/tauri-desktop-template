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
- [How long things take](#how-long-things-take)
- [The UI in a plain browser](#the-ui-in-a-plain-browser)
- [Port an existing app (Electron or web)](#port-an-existing-app-electron-or-web)
- [Architecture](#architecture)
- [Processes and folders](#processes-and-folders)
- [How to add a feature](#how-to-add-a-feature)
- [How settings work](#how-settings-work)
- [Build-time app config](#build-time-app-config)
- [Command errors](#command-errors)
- [Untrusted input](#untrusted-input)
- [Typed IPC](#typed-ipc)
- [Accessibility](#accessibility)
- [Translations](#translations)
- [Tests](#tests)
- [Search evals](#search-evals)
- [Postgres upgrades](#postgres-upgrades)
- [Bundling models](#bundling-models)
- [Key-value store and cache](#key-value-store-and-cache)
- [Wrap a web app](#wrap-a-web-app)
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
| `localStorage` / IndexedDB, Redis as a store | `kitchen enable kv`: `desktop_core::kv`, one file, namespaces, TTLs ([Key-value store and cache](#key-value-store-and-cache)) |
| Redis as a cache, `lru-cache` | `kitchen enable cache`: `desktop_core::cache::Cache`, in memory, TTL, single-flight |
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
kitchen start                             # installs the UI's npm packages on the first run, then runs with hot reload
                                          # (add --install if `kitchen doctor` says the Tauri CLI is missing)
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

## How long things take

Measured on Linux (4 cores, 15 GB RAM, SSD) with every feature off except the ones named.
The crates were already downloaded once; on a new machine add the time cargo and npm need
to download them. "Cold" means an empty cargo `target/` folder.

| Step | Wall time | Notes |
|---|---|---|
| `cargo install` the Kitchen CLI | 13 s | once per machine |
| `kitchen new demo` | 0.5 s | 1.8 s the first time, when it downloads the template |
| `kitchen new demo2 --enable email,background_jobs,tray_menu` | 0.5 s | |
| `kitchen doctor` | 0.3 s | |
| `kitchen enable http_api --dry-run`, then for real | 0.9 s, 0.8 s | 14 files; `git diff` shows them (`new` makes a first commit) |
| `kitchen generate command hello`, `kitchen generate page Hello` | < 0.1 s each | |
| `cargo build` (cold) | 14 s | builds `desktop-core` only: the workspace's default member |
| `cargo build -p demo` (the whole app, after the line above) | 86 s | about 3.9 GB in `target/` |
| `kitchen test --unit` | 70 s | builds the test profile, then runs the Rust tests |
| `kitchen start`, first run, cold, until the window is up | 101 s | `npm install` (3 s) + the Rust build + Vite |
| `kitchen start` after a build | 23 s | |
| Edit a `.jsx` file while `kitchen start` runs | instant | Vite reloads it in place |
| Edit a `.rs` file while `kitchen start` runs | seconds | rebuilds the app crate and restarts the window |

Turning on heavy features changes the first build most. With the earlier defaults
(`embeddings`, `postgres`, `pdf`, `ipc_worker`, `python_example`, `updater` and friends on),
`cargo build -p demo` took 115 s even with every dependency already compiled once, and a
cold build downloads ONNX Runtime and compiles docling. That is why every feature now
starts off; `kitchen enable` adds them when you need them.

Things that look like errors but are not:

- `libEGL warning: DRI3 error` when the window opens under `xvfb-run` or in a VM: WebKitGTK
  falls back to software rendering, and the app works.
- `npm warn deprecated` lines during the first `kitchen start`: they come from npm
  packages and do not stop anything.

## The UI in a plain browser

The React UI runs in any browser without the Rust side, so you can work on it the way you
work on a web app:

```sh
cd ui-react
npm install
npm run dev            # open http://localhost:5173 (and /settings.html with the app shell)
```

Outside Tauri, `ui-react/src/dev-mock.js` stands in for the backend: it answers `invoke()`
with sample data and a small in-memory state (settings save, search finds the sample
documents, profiles can be added), and `listen()` / `emit()` work. Each call is logged to the
console as `[dev-mock]`. A command it does not know rejects with
`dev-mock: no mock for "<command>"`; add a line to `commands` in that file when you add a
Tauri command. URL options: `?mode=dark`, `?theme=nord`, `?os=macos|windows|linux` and
`?wizard=1` (start at the setup wizard), and `?pseudo=1` (the pseudo-locale: every text
accented and 30% longer, see [Accessibility](#accessibility)).

It is loaded only when `import.meta.env.DEV` is set and `window.__TAURI_INTERNALS__` is
missing, so `cargo tauri dev` and release builds never contain it. For screenshots or
browser tests of the built pages, build it in on purpose:

```sh
VITE_DEV_MOCK=1 npm run build && npx vite preview   # http://localhost:4173
```

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

| Electron | Kitchen (Tauri 3) |
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
  model), `APP_MODELS` and `APP_MODELS_DIR` (bundled models), `APP_PG_URL` and
  `APP_PG_EXT_DIR` (Postgres), `APP_DEVTOOLS=1` (debug builds).

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

Every Tauri command returns `Result<T, app_lib::error::CommandError>`, so a failed command
rejects the UI's `invoke` with one JSON shape:

```json
{ "code": "not_found", "message": "No such file or directory (os error 2)", "details": { "kind": "NotFound" } }
```

`code` is short, snake_case and **stable**: the UI may branch on it, so a code never changes
meaning once released (new codes go at the end of the table). `message` is for people and
may change. `details` is optional data for code and is left out when there is none.

| Code | Meaning | Comes from |
|------|---------|------------|
| `invalid` | The arguments are wrong: missing, the wrong type, or not allowed. | a bad argument, an unknown method, `serde_json::Error` |
| `not_found` | A file, profile, web view or other named thing does not exist. | `desktop_core::Error::Io` / `io::Error` of kind `NotFound` |
| `denied` | The OS or a policy said no (file permissions, the web view allowlist). | `Io` of kind `PermissionDenied`, policy checks |
| `not_ready` | The backend is still starting; try again in a moment. | the engine is not open yet |
| `unavailable` | Turned off in Settings, or not set up or downloaded yet. | the command itself |
| `not_built` | The part is not compiled into this build (`details.part` names it). | `Error::NotBuilt` |
| `io` | Reading or writing a file failed (`details.kind` has the I/O error kind). | other `Error::Io` |
| `bad_data` | A file or a message was not the JSON expected. | `Error::Json` |
| `embedding` | The embedding model could not load or run. | `Error::Embed` |
| `store` | The store (file or Postgres) refused or failed. | `Error::Store` |
| `source` | A file, folder, recording or URL could not be used. | `Error::Source` |
| `ai` | The language model call failed, or AI is switched off. | `Error::Llm` |
| `mail` | Sending email or logging in to the mail server failed. | `mail::MailError` |
| `oauth` | Signing in with a provider failed. | `oauth::OauthError` |
| `cloud_sync` | rclone could not be installed, configured or run. | `rclone::RcloneError` |
| `tailscale` | Tailscale could not be reached or changed. | `tailscale::TailscaleError` |
| `failed` | Anything else. | a `String` or `&str` |

**In Rust**, `?` does the mapping: a `desktop_core::Error` (or one of the part errors above)
gets its code, a `String` from an older helper becomes `failed`. Set a code yourself with
`CommandError::invalid("...")`, `CommandError::unavailable("...")` or
`CommandError::new(codes::DENIED, "...")` (the constants are in `app_lib::error::codes`).
The backend behind the commands (`service::Service`) returns the same type and the worker
process sends it over its socket unchanged, so a code set there reaches the UI. The MCP
server and the HTTP API still answer with the message only. Keep commands thin: the mapping
lives in `error.rs`, not in each command.

**In the UI**, call `errorText(e)` (exported by {% if frontend == "react" %}`ui-react/src/ipc.js`{% else %}`ui/errors.js`{% endif %}): it reads
`e.message`, falls back to `String(e)` for anything else, and adds a next step for a few codes
(`denied`, `not_built`, `mail`, `oauth`, `tailscale`). `errorCode(e)` gives the code
(`failed` for a plain string), for branching. The typed bindings include `CommandError`
(`bindings/types.d.ts`).{% if frontend == "react" %} The browser dev mock (`dev-mock.js`) rejects with the same shape;
call `fail(code, message)` in a mock that should fail.{% endif %}

## Untrusted input

Files the user drops in, watched folders, recordings from the web view, MCP clients, HTTP
requests, backups picked for restore and downloads are all untrusted: a parser must never
panic, hang or allocate without bound on them. The rules:

1. **Cap the size first.** Each kind of input has a cap in `desktop_core::untrusted::limits`
   (`WAV_BYTES`, `PDF_BYTES`, `CONFIG_BYTES`, `MCP_LINE_BYTES`, `ZIP_UNPACKED_BYTES`, ...).
   Read files with `untrusted::read_capped`, and lines with `untrusted::read_line_capped`,
   never `std::fs::read` or `BufRead::lines` on outside data. Ingest also refuses files over
   `ingest.maxFileMb` before reading them.
2. **Never trust a length in the input.** Clamp a header's length to the bytes really there,
   use `get` and checked or saturating arithmetic, and count what a decompressor really
   produces (`pgstore::unzip_into`), not what the header claims.
3. **No super-linear work on input.** Globs, chunking and resampling are linear in their
   input; a sample rate or a count from a file is range-checked before it sizes an allocation.
4. **Panics become errors.** `untrusted::guard` wraps a parser in `catch_unwind`, so one bad
   file fails alone. The MCP server answers a panicking message with error -32603 and goes on;
   the HTTP API's backend calls run in `spawn_blocking`, whose panic becomes a 500.
{%- if pdf %}
5. **Parse risky formats in a child.** With `ingest.pdf.isolate` (on by default) a PDF is
   parsed by `<app> --parse` (`pdf::serve_parse`): the bytes go in on stdin, JSON comes out on
   stdout, and the child is killed after `ingest.pdf.timeoutSecs` and limited to
   `ingest.pdf.memoryMb` (`setrlimit` on Linux and macOS, a Job Object on Windows; see
   `desktop_core::sandbox`). macOS does not enforce the memory limit, so the time limit
   and the output cap are the bounds there.
{%- endif %}

Every parser has a fuzz target in `fuzz/` (`make fuzz`, [TESTING.md](../TESTING.md#fuzzing)).
When you add a parser of outside input, add its cap, a fuzz target, and a unit test for
each bug the fuzzer finds.

## Typed IPC

The UI calls the Rust commands with `invoke`. Their argument and result types are generated
from the Rust code, so the editor knows what each command takes and returns, and a renamed
argument or a changed struct shows up as an error in the UI code instead of at run time.

```text
src-tauri/src/lib.rs   generate_handler![...] + each #[tauri::command] fn's signature
desktop-core, src-tauri   #[derive(TS)] on the structs they take and return (Config, Hit, AppInfo, ...)
        |  cargo test -p <app> export_bindings
        v
{% if frontend == "react" %}ui-react/src/bindings/{% else %}ui/bindings/{% endif %}types.d.ts      every struct and enum (the settings, the feature flags, ...)
{% if frontend == "react" %}ui-react/src/bindings/{% else %}ui/bindings/{% endif %}commands.d.ts   Commands (args and result of each command), Invoke, Api
```

{% if frontend == "react" %}In the React UI, call commands through `src/ipc.js`:

```js
import { api } from './ipc.js';

const hits = await api.search({ query: 'heart', k: 5 }); // Hit[]
const info = await api.app_info(); // info.features.ai, info.settings.config.search.topK, ...
```

`api.<command>(args)` is `invoke('<command>', args)` with the types attached; `invoke` from
`./ipc.js` is the same typed `invoke` for a command chosen at run time. The UI stays plain
JavaScript and JSX: the types reach it through `.d.ts` files and JSDoc `@type` comments, so
there is no TypeScript build step. Editors (VS Code and others) show them as you type; for a
full check run `npx tsc --noEmit --allowJs --checkJs --jsx preserve --moduleResolution bundler src/*.js`.
{% else %}The plain UI gets the same types through JSDoc: `ui/app.js` declares
`/** @type {import('./bindings/commands').Invoke} */ const invoke = ...`, so editors know each
command's arguments and result while the files stay plain JavaScript with no build step.
{% endif %}
**How it works.** [`desktop_core::bindings`](../desktop-core/src/bindings.rs) (cargo feature
`ts`, which the app turns on for its tests only) reads the `generate_handler![...]` list in
`src-tauri/src/lib.rs` with `syn`, follows each entry's `#[cfg(...)]` and finds the command's
signature (Tauri's own parameters, such as `AppHandle` and `State`, are left out; argument
names become camelCase, as Tauri expects). The types come from
[ts-rs](https://docs.rs/ts-rs), a derive that follows the serde attributes the types already
have. ts-rs was picked over specta/tauri-specta because it is only a derive: commands stay
ordinary `#[tauri::command]` functions registered with Tauri's own `generate_handler!`, and
tauri-specta is still a release candidate for Tauri 2. The derives are behind
`cfg_attr(test, ...)` and `cfg_attr(feature = "ts", ...)`, so release builds contain none of it.

**Keeping them fresh.**

```sh
cargo test -p {{project-name}} export_bindings                          # writes the files (commit them)
KITCHEN_BINDINGS=check cargo test -p {{project-name}} export_bindings   # only compares
```

With `CI` set (GitHub Actions sets it) the test only compares, and fails with
`stale TypeScript bindings` when the Rust commands or types changed and the files were not
regenerated; `KITCHEN_BINDINGS=write` or `=check` overrides that. Run it with the default
features: the bindings describe the build the app was generated with. A missing file is
written in either mode, so a new app gets its first bindings from its first test run.
`kitchen generate command <name>` runs the test after adding the command (`--no-bindings`
skips it).

**A new command** needs nothing extra when it takes and returns plain types, `Value` or types
already listed. When it returns a new struct, derive the type and register it in the test:

```rust
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]          // in desktop-core: #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Export { rows: usize, file: String }

// src-tauri/src/lib.rs, fn export_bindings, above `// kitchen:bindings`:
b.add::<export_csv::Export>("export_csv::Export");
```

The test names any type it cannot write (`` `Export` has no TypeScript type ``), and any two
types that would share a TypeScript name (rename one with `#[ts(rename = "...")]`).
Commands that return `serde_json::Value` are typed `JsonValue`; give them a struct to type
the result.

## Accessibility

{% if frontend == "react" %}`npm run a11y` (in `ui-react/`, and part of `kitchen test` when Node is installed) checks
the UI in a real browser: it builds the UI with the browser dev mock, serves it, and runs
[axe-core](https://github.com/dequelabs/axe-core) on the main screen, the first-run wizard,
the Settings window (with the app shell) and every Settings section, in light and dark mode.
Serious and critical issues fail it (`A11Y_LEVEL=moderate` is stricter), and so does a Tab
stop that cannot be seen. It also renders each screen in the **pseudo-locale** (every text
accented and 30% longer, `src/pseudo.js`) and fails when anything then sticks out past the
window, so a longer translation will not break the layout. Turn the pseudo-locale on by hand
with `?pseudo=1` in `npm run dev`.

It uses Playwright, which is large and so not a dependency of the app: install it once with
`npm i -g playwright && npx playwright install chromium` (CI installs it on the runner). Without
it the check says so and passes, unless `A11Y_REQUIRED=1`.

What the UI does for this, and what new screens should keep doing:

- every input, select and textarea has a label: a wrapping `<label>`, or `aria-label` when
  the placeholder is the only visible text;
- status lines are live regions (`role="status"`, errors `role="alert"`), so screen readers
  read the outcome of an action;
- buttons are real `<button>`s with text, in reading order, with a visible focus ring
  (`:focus-visible` in `style.css`); the Settings sections are a `nav` with `aria-current`;
- colours come from the theme tokens, which keep text contrast at WCAG AA in light and dark.
{% else %}The plain UI has no build step and no browser test; keep labels on every input, `role="status"`
on status lines and visible focus rings (see the React UI's `npm run a11y` for the checks).
{% endif %}

## Translations

{% if i18n %}This app has the `i18n` feature.{% else %}This needs the `i18n` feature (`kitchen enable i18n`); this app does not have it yet.{% endif %}
Every message the app shows comes from one set of [Fluent](https://projectfluent.org/fluent/guide/)
files, `locales/<code>/app.ftl`, shared by the backend and the UI. English is the source;
German (`de`) and Arabic (`ar`, right to left) ship as examples.

```ftl
action-done-imported = Imported the drop folder: { $files ->
    [one] one file
   *[other] { $files } files
}.
setting-ui-language = Language
    .help = Menus and messages switch at once.
```

- **Backend** (`desktop_core::i18n`): `t!("action-done-paused")`, or with arguments
  `t!("error-store", detail = e.to_string())`. It translates the native menus and tray
  (`src-tauri/src/tray.rs`), the test email, and the `message` of command errors; the
  error `code` never changes, so the UI and scripts can still match on it. The `.ftl` files
  are compiled in by `desktop-core/build.rs`.
- **UI**: {% if frontend == "react" %}`t('add-text-done', 'Added { $count } chunks.', { count })` from
  `ui-react/src/i18n.js`, with the files embedded by Vite (`ui-react/src/locales.js`,
  `@fluent/bundle`). A component that shows text calls `useLocale()` so it redraws when the
  language changes.{% else %}`t('add-text-done', 'Added { $count } chunks.', { count })` from
  `ui/i18n.js`, and `data-i18n="id"` (or `data-i18n-placeholder`, `-title`, `-aria-label`) on
  static HTML. The messages come from the backend (`i18n_bundle`) already parsed.{% endif %}
  The English text sits next to the id, so the UI reads well with the feature off and a
  missing translation shows English.
- **Language**: `ui.language` is `system` by default (the OS language through `sys-locale`,
  matched to the closest locale, then English). Settings → Language switches it at once in
  every window and the tray. Arabic, Hebrew, Persian and Urdu set `<html dir="rtl">`; the
  CSS uses logical properties (`margin-inline-start`, `padding-inline-start`, `inset-inline`,
  `text-align: start`), so layouts mirror by themselves. Keep using them.
- **Plurals, numbers, dates**: plural forms follow each language's CLDR rules (Arabic has
  six); `NUMBER($n, minimumFractionDigits: 1)` and plain numbers use the locale's
  separators; `DATETIME($ms, dateStyle: "long")` formats dates from a small table of month
  names and orders (en, de, fr, es, ar; others fall back to ISO dates).
- **Digits: Latin by default.** Every number and date is written with `0123456789`, in
  Arabic too (`1,234.5`, not `١٬٢٣٤٫٥`). WebViews disagree about Arabic (newer ICU data
  writes `ar` with Latin digits, older WebKit with Arabic-Indic ones), and the backend's
  menus and messages must match the UI, so the backend decides and the UIs follow:
  `ui.digits = "native"` (Settings → Language → Digits) switches to the language's own
  digits where it has them (Arabic-Indic for Arabic, Persian digits for Persian; Arabic in
  the Maghreb stays Latin, as CLDR says). The backend formats with
  `desktop_core::i18n::format_number`; the UIs format with `Intl` under the tag
  `number_locale` gives (`ar-u-nu-latn` or `ar-u-nu-arab`): the React UI builds it in
  `ui-react/src/locales.js` (`numberLocale`, the same rule), the plain UI gets it from
  `i18n_bundle` (`numbers`). Use `dateTime()` from `ui-react/src/i18n.js` for dates in React
  rather than a bare `toLocaleString()`. Pass ids such as ports and process ids to `t()` as
  text (`String(port)`), so they get no group separators.

Why Fluent and not ICU4X: Fluent is a message format (variables, plurals, attributes,
fallback between locales) with one Rust crate (`fluent-bundle`) and one JS package
(`@fluent/bundle`) that read the same files. ICU4X formats numbers, dates and plurals with
the full CLDR data, but has no message format and adds megabytes of data to the binary;
Fluent plus a small date table covers what a desktop app's messages need. If you need full
CLDR date formats, put `icu` behind the same `format_date` function.

### A new language

```sh
kitchen generate locale fr          # locales/fr/app.ftl: English, every message marked "# untranslated"
# translate it, starting with `language-name = Français`, and delete each "# untranslated" line
kitchen i18n check                  # missing or extra messages, Fluent syntax, the UI's ids
```

Rebuild, and the language appears in Settings. `kitchen generate locale en-XA --pseudo`
makes an accented, longer pseudo-locale for finding hard-coded text and layouts that cut
text off{% if frontend == "react" %}; `npm run dev` with `?locale=ar` shows right to left in the browser, and `?pseudo=1` the built-in pseudo-locale{% endif %}.

### A new message

Add it to `locales/en/app.ftl` and every other locale (English text is fine until someone
translates it), then use it. `kitchen i18n check` (also in `kitchen test` and `make lint`)
fails when a locale is missing a message English has, has one English does not, uses a
variable the English message never passes, or when the UI asks for an id English lacks.
Settings fields are translated by id: `setting-<section>-<key>` for the label (a camelCase
or dotted key becomes kebab-case), `setting-<section>-<key>-<option>` for a choice and
`section-<id>` for a section title.

## Tests

```sh
kitchen test              # both kinds below (feature tests are skipped, with the reason, if the machine can't run them)
kitchen test --unit       # cargo test --workspace (cargo nextest + doc tests when nextest is installed)
kitchen test --feature    # the real app, driven through its window
```

Both set `KITCHEN_ENV=test`, so the build embeds `config/environments/test.json`. If
`ui-react/package.json` has a `test` script, `--unit` runs it too; the template ships none,
so Node stays a build tool for the UI. With Node installed, `--unit` also runs the UI's
accessibility check (`npm run a11y`, see [Accessibility](#accessibility)). The unit tests
include `export_bindings`, which keeps the [typed IPC](#typed-ipc) bindings in step.

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

## Search evals

With the `evals` answer (cargo feature `evals`, off by default), the app can score its own
search on a labelled dataset and fail when a change makes it worse. The logic is
`desktop_core::eval` ([`desktop-core/src/eval.rs`](../desktop-core/src/eval.rs)); the
command line is `src-tauri/src/eval.rs`.

```sh
make eval                                               # evals/dataset.jsonl against evals/baseline.json
cargo run -p <app> -- --eval                            # the bundled sample, JSON report on stdout
cargo run -p <app> -- --eval my/dataset.jsonl --baseline my/baseline.json --threshold 0.02
cargo run -p <app> -- --eval --out evals/baseline.json  # save a new baseline (on purpose)
cargo run -p <app> -- --eval --answers                  # also the AI answer path (feature ai)
kitchen eval [dataset] [--baseline file]                # the same from Kitchen (and the MCP tool `eval`)
```

- **Data.** A dataset is JSON lines, `{"query", "relevant_doc_ids": [...], "expected_answer"?}`.
  The corpus is `corpus.jsonl` next to it (or `--corpus`), JSON lines of `{"id", "text"}`;
  each document is indexed with its id as the source, so the ids in `relevant_doc_ids` are
  what search returns. `evals/` ships a sample: 20 short cooking notes and 15 queries.
- **What runs.** The corpus goes into a fresh temporary store (`--backend file|postgres`)
  with the saved search and AI settings. Keyword search (BM25, in the file store or with
  `bm25` in Postgres) always runs; with `embed`, vector and hybrid run too.
- **Metrics**, per mode, with binary relevance over the top `k` (`--k`, default 5):
  recall@k, MRR, nDCG@k, and p50/p95/mean latency per query. Each query's results are in
  `per_query`.
- **Answers** (`--answers`, feature `ai`). For each case with an `expected_answer` the app
  asks through `Engine::ask` and reports the word-overlap F1 with it, plus a faithfulness
  score from an LLM judge that goes through `desktop_core::llm` (budgets and the network
  policy apply; `--no-judge` leaves it out). Skipped, with the reason in the report, when AI
  is off or the provider has no API key, so CI without keys still passes.
- **Baseline.** `--baseline` compares with a saved report: a drop in recall@k, MRR,
  nDCG@k or faithfulness of more than `--threshold` (default 0.02), or a mode the baseline
  has that is gone, sets `"ok": false` and exits 1. Latency is only checked with
  `--latency-ratio` (e.g. 1.5), since it depends on the machine.
- **Extending it.** Anything that ranks document ids is a `Retriever` (a closure is
  enough), and anything that grades an answer is a `Judge`, so a new search source or
  judge needs no changes to the metrics or the report.

## Postgres upgrades

With the `postgres` answer, the app runs its own Postgres server on a data folder in the
profile (`<data>/postgres/data`). Postgres cannot start on files made by another major
version, so when an app update brings a newer major (say 16 to 17), the folder is upgraded
with `pg_upgrade`. The logic is `desktop_core::pgupgrade`
([`desktop-core/src/pgupgrade.rs`](../desktop-core/src/pgupgrade.rs)); `pgstore` calls it
when it starts the server.

| `postgres.upgrade` | What happens at start when `PG_VERSION` is older |
|---|---|
| `ask` (default) | setup stops with a `pg-upgrade-needed: ...` error; the UI shows **Upgrade database** / **Not now** (`run_setup` with `pgUpgrade: true` or `false`) |
| `auto` | upgrades now; the setup screen shows "Upgrading database: <step> (n/m)…" |
| `off` | keeps running on the old major's server; Settings shows a notice (`Status.notice`) |

`postgres.upgradeCopy` (off) uses `pg_upgrade --copy` instead of `--link`: slower and twice
the disk space, but the old folder can always be started again.

The steps, each a progress line and a log entry:

1. Get the old server: the copy the previous version left in `install/<old version>`, else a
   download from a pinned release URL checked against a pinned SHA-256 (`OLD_SERVERS`; the
   server only). When you bump `PG_VERSION`, move the old version and its hashes there.
2. Start the old server once to read its encoding and locale, and stop it cleanly.
3. `initdb` the new cluster in `data.pg<new>.new` with the same encoding, locale and password.
4. `pg_upgrade --check`, then `pg_upgrade --link` (or `--copy`). The extension bundle for the
   new major (pgvector, and pg_tokenizer + VectorChord-BM25 with `bm25`) is installed first.
5. `data` becomes `data.pg<old>.bak`, the new cluster becomes `data`.
6. Start, a smoke query, `ALTER EXTENSION ... UPDATE`, `ANALYZE`.
7. Remove `data.pg<old>.bak`.

**When it fails.** Up to and including `pg_upgrade`, the old folder is put back as it was,
the error goes to `pg-upgrade-failed.txt` and the app keeps running on the old server, with
the error as a notice. `auto` then asks instead of retrying at every start. If the new server
fails its smoke test, a `--copy` upgrade is rolled back the same way (the new cluster is kept
as `data.pg<new>.failed`); after `--link` the old folder shares files with a server that has
started, so it is kept as `data.pg<old>.bak` and the error says where. User data is never
deleted on failure, and a crash part-way is finished or undone on the next start.

```sh
<app> --pg-upgrade --check   # pg_upgrade --check only: nothing changes
<app> --pg-upgrade           # upgrade now, whatever postgres.upgrade says (also retries a failed one)
```

```rust
use desktop_core::pgupgrade::{need, Need};

match need(&data_dir.join("postgres").join("data"), desktop_core::pgstore::bundled_major())? {
    Need::Upgrade { from, to } => println!("Postgres {from} -> {to} needed"),
    other => println!("{other:?}"),
}
```

**Tests.** `cargo test -p desktop-core --features postgres pgupgrade` runs the plan and the
rollback against a fake `Runner`, with no Postgres. The real upgrade (downloads Postgres 15
and 16; `initdb` and `pg_upgrade` refuse to run as root) is ignored by default:
`APP_PG_UPGRADE_E2E=1 cargo test -p desktop-core --features postgres -- --ignored upgrade_e2e`.

## Bundling models

By default every model downloads on first use into `<data>/models`: the installer stays
small, and the first use of each feature needs the network. When the app is generated
(question `models`, asked only when a feature with a model is on) it can ship them inside
the installer instead:

| `models` | What happens |
|---|---|
| `download` (default) | Every model downloads on first use, as before |
| `bundle` | The chosen models are inside the installer; the first start works offline. A bundled model missing from the install is an error that says to reinstall |
| `bundle_with_fallback` | Inside the installer, and any that is missing (or fails its checksum) downloads instead |

Then one question per feature that is on picks what goes in:

| Answer | Model | Size | Licence (model card) |
|---|---|---|---|
| `bundle_embedding_model` | the chosen `embedding_model`: BGE small en v1.5 / all-MiniLM-L6-v2 / multilingual E5 small / Nomic embed v1.5 | 134 / 91 / 487 / 548 MB | MIT / Apache-2.0 / MIT / Apache-2.0 |
| `bundle_stt_model` | Whisper base (8-bit ONNX) | 78 MB | MIT |
| `bundle_tts_voice` | tiny-voca | 25 MB | Apache-2.0 |
| `bundle_llm` (default no) | Qwen3 0.6B Q4_K_M GGUF | 397 MB | Apache-2.0 |

`kitchen models add whisper-tiny` / `whisper-small` (42 / 249 MB) bundles other Whisper
sizes. `kitchen models list` shows every model with its size and licence.

How it works:

- `src-tauri/resources/models/models.lock` lists each file: kind, model, path, bytes, SHA-256
  and a URL pinned to a Hugging Face revision. Only the lock is committed (`.gitignore`).
- `make models` (or `kitchen models fetch`) downloads the files next to the lock and checks
  every checksum; `make build` runs it first. `make models-verify` / `kitchen models verify`
  checks what is there. Both run `desktop-core/examples/models.rs`, the same code as the app.
- `tauri.conf.json` has `"bundle.resources": {"resources/models/": "models/"}`, so the folder
  is copied into the installer (and next to the binary in development builds).
- At run time `desktop_core::models::locate` picks the folder: the bundled copy when every
  file is there (its SHA-256 is checked once, then a marker in `<data>/models/.bundled`
  remembers it), else the user's cache, downloading only if `net` allows. The embedding
  model is kept in the Hugging Face cache layout fastembed reads, so it loads with no
  network call. The setup wizard says "built into the app" instead of offering a download,
  and `app_info` lists the bundled models (`models`).
- `APP_MODELS` (`download`, `bundle`, `bundle_with_fallback`; set in `.cargo/config.toml`)
  and `APP_MODELS_DIR` (another bundled folder) override them for a run.

Things to know before you ship:

- **Size.** The pre-hook and `kitchen features --check` print what the models add; past
  about 200 MB they warn. GitHub refuses release files over 2 GB, and macOS notarization
  uploads the whole installer to Apple (slow for large ones), so an LLM is a warning too.
- **Updates.** The updater downloads the whole installer, so every update carries the models
  again. Bundle small models, or use `bundle_with_fallback` with fewer models.
- **Licences.** Bundling redistributes the models: ship each model's licence (above) with
  the app, as you do for code. `kitchen features --check` lists them.

## Wrap a web app

With `web_wrapper` on, the app is one remote site in its own native window instead of the
boilerplate UI: a PiKVM, a NAS, a router or any web tool you want in the Dock with its own
icon. `kitchen new <name> --web <url>` asks the questions, reads the site's title, icon and
certificate, and generates the app:

```sh
kitchen new kvm --web https://pikvm.local --insecure-tls --capture-keys
#   site certificate SHA-256 3A:7F:...:C2 (self-signed or untrusted: pinned in the app; compare it with the device's own)
#   site icon https://pikvm.local/share/apple-touch-icon.png (180px)
cd kvm && kitchen start
```

On a terminal, plain `kitchen new kvm` first asks "Desktop app (boilerplate)" or "Web app
(wrap a URL)", then the URL, "Ignore TLS certificate errors for this site?", "Capture all
keys?", the release shortcut (default Ctrl+Alt+Shift+K), microphone, start muted, webcam
and screen sharing (all default no). The answers go into `config/settings.json`
(`wrapper`) and are built into the app (see "Developer settings" below).
`kitchen icon fetch <url> [--insecure-tls]` remakes the icon set from the site.

**Developer settings.** Every wrapper setting is the developer's choice, made before the
build: the `wrapper` section of `config/settings.json` is compiled in (`app_config`) and
read-only at run time. Change it by editing the file, or for the generation answers with
`kitchen set` (`web_url`, `web_insecure_tls`, `web_cert_sha256`, `capture_keys`,
`release_shortcut`, `web_microphone`, `web_camera`, `web_screen`, `web_muted`), then
rebuild. `wrapper.userEditable` names the only fields the user may change in Preferences
(default `["releaseShortcut", "zoom", "muted", "startCaptured", "dockIcon", "menuBarIcon",
"runAtStartup", "rememberWindow"]`; `captureKeys`,
`microphone`, `camera` and `screen` may be added; `[]` hides them all). Preferences shows
only those, and `wrapper_set` refuses any other field in Rust (`UserOverrides::change`),
so a page cannot change the address, TLS, headers or anything else. The user's changes
are saved apart, as `config.wrapper` in the user's `settings.json` (`UserWrapper`: the
overrides and the certificate pinned on first use), and merged over the built-in settings;
an override the developer no longer allows is ignored. The Zoom and Mute menu items always
work, and are remembered only when `zoom` or `muted` is editable.

**Window, Dock, menu bar and startup** (`src-tauri/src/wrapper/presence.rs`). Four user
preferences, with the developer's defaults in `config/settings.json`:

| Preference | Default | Built on | Applies |
|---|---|---|---|
| `rememberWindow`: size, position, maximized and full screen come back on the next launch | on | `tauri_window_state` (the plugin tracks and saves the `web-app` window; the wrapper restores it, and `desktop_core::wrapper::screen::fit` moves it back onto a display when the one it was on is gone) | next launch |
| `menuBarIcon`: a menu bar icon (system tray on Windows and Linux) with Show/Hide, Release Keys (with key capture), Mute, Preferences and Quit | off | `tray_menu` (`src-tauri/src/tray.rs`; the wrapper keeps its own app menu) | at once |
| `dockIcon` (macOS): off makes a menu-bar-only app, and closing the window hides it | on | the tray's "menu bar only" switch | at once |
| `runAtStartup`: open at login (a LaunchAgent on macOS, the Run key on Windows, an autostart entry on Linux); with the Dock icon off it starts hidden in the menu bar | off | `tauri_autostart`, started with `--autostart` | at once |

`kitchen new --web` turns on `tray_menu`, `tauri_plugins`, `tauri_window_state` and
`tauri_autostart` for these. In a build without one of them, its preference is not shown
and `wrapper_set` refuses it. `WrapperConfig::check` refuses the Dock icon and the menu bar
icon both off, so Preferences cannot save that and a hand-edited file falls back to the
built-in settings: the app always stays reachable.

**What the site can and cannot do.** The site's window (`web-app`) has no Tauri IPC: it is
in no capability, and the command guard refuses every call from a `web-*` window. Only the
wrapped origin loads in it; links, `target=_blank` and `window.open` to anything else open
in the default browser, and other schemes are refused. Cookies and storage persist in the
app's data folder (`<data>/wrapper`), so a login survives restarts. Downloads go to the
Downloads folder under a unique name, with a "Show in Finder/Explorer" button.

**Open in Browser….** A site that breaks in the web view (WebKit on macOS and Linux) can
still be used: Page → Open in Browser… (Cmd/Ctrl+Shift+O) opens the page the window shows
in a real browser. It opens only that page, only when it is `http(s)` on the wrapped
origin (through the proxy, the site's own URL, never the loopback one with its token).
`wrapper.openInBrowserApp` picks the browser: empty (the default) for the user's default
browser, or an app name such as `"Google Chrome"`, started on macOS with `open -a <app>
<url>` and elsewhere as a program on `PATH` (`"firefox"`), with an argument list and never
a shell; if it cannot be started, the default browser opens instead. `WrapperConfig::check`
allows plain app name characters only (letters, digits, spaces, `.`, `-`, `_`, `+`). It is
a developer setting, not one the user can change (`desktop_core::wrapper::browser`).

**Self-signed certificates.** With "ignore TLS errors" on, a certificate error is accepted
for the wrapped origin only, and only for the certificate pinned by its SHA-256: the one
`kitchen new` saw (it prints it; compare it with the device's), or the first one the app sees.
A different certificate is refused until you press Forget in Preferences, which is there
only when `insecureTls` is on and `pinnedSha256` is empty (a certificate pinned on first
use, kept with the user's settings, apart from the built-in ones). A pin you set in
`config/` is fixed: change it there and rebuild. No local CA is installed and the system
trust store is never touched.

**Key capture.** Keys the OS would keep (Cmd+Tab, the Windows key, Alt+Tab, media keys) go
to the page while capture is on. The release shortcut, or the window losing focus, gives
them back; an overlay says how. `kitchen doctor` checks the macOS permission.

**The web engine.** Tauri 3 takes its engine when the app starts (`Builder::runtime` in
`run()`), and the wrapper's code is generic over that runtime (`R: tauri::Runtime`). It uses
Tauri's own API for navigation, new windows, downloads, permissions, page scripts and zoom.
The few hooks that need the engine's native web view (mute, the certificate hook in direct
mode, WebView2's headers, WebKitGTK's settings) go through the `Engine` trait in
`src-tauri/src/wrapper/engine.rs`, which today is the system web view through wry. The
Chromium engine implements the same trait (see "Chromium engine"); the rest of the wrapper
does not change.

**Customise.** `config/settings.json` (`wrapper`) sets everything at build time, and it is
checked in one place (`WrapperConfig::check`, with the user's changes merged):
the User-Agent, request headers per URL pattern (`/api/*`, a full URL, or `re:` + a regex;
values are literal, `${ENV}`, `config:<key>` or `keychain:<name>`, never logged and never
sent to Preferences), form auto-fill (pattern → CSS selector → value, on the wrapped origin
only; on load or from the Page menu; Preferences can store the secrets the rules name as
`keychain:<name>` in the OS keychain, and no others), and switches for WebRTC,
service workers and WebAssembly. A custom URL scheme (`deep_link_scheme`, needs
`tauri_deep_link`; `kitchen enable deep_link --scheme kvm`) maps `kvm://path?x` to
`<site>/path?x`, same origin only, and reaches a running instance with `single_instance`.

**The proxy.** Headers on every request (scripts, `fetch`, WebSocket handshakes) need a hook
the web view lacks on macOS and Linux, so there the app can run a loopback proxy
(`proxy`: `auto` turns it on when a header rule exists, `on`, `off`): plain
`http://127.0.0.1:<port>` with a per-launch token cookie, upstream TLS through rustls with
the same pin, WebSockets proxied, `Set-Cookie` and `Location` rewritten for loopback, HTML
untouched. WebRTC media does not go through it (fine for PiKVM, whose video still works).

| | macOS (WKWebView) | Windows (WebView2) | Linux (WebKitGTK) |
|---|---|---|---|
| Pinned self-signed TLS | `didReceiveAuthenticationChallenge` | `ServerCertificateErrorDetected` | `load-failed-with-tls-errors` + `allow_tls_certificate_for_host` |
| Key capture | CGEventTap; needs Accessibility and Input Monitoring (a first-run screen opens the panes) | `WH_KEYBOARD_LL`; **Ctrl+Alt+Del and Win+L stay with Windows** | keyboard grab: X11 `XGrabKeyboard`; Wayland only with the shortcuts-inhibit protocol (GNOME asks once), else the compositor keeps its shortcuts |
| Microphone, webcam | prompts with the usage strings and entitlements (only when on) | allowed per origin | allowed per origin |
| Screen sharing | needs Screen Recording (System Settings; on the first-run screen) | picker | portal (PipeWire) |
| Mute | `_setPageMuted:` | `IsMuted` | `set_is_muted` |
| Headers on every request | the loopback proxy | `WebResourceRequested` (no proxy) | the loopback proxy |
| WebRTC / WASM / service workers off | page script | browser switches (`--disable-features=WebRtc`, `--noexpose_wasm`) + page script | WebKit setting (WebRTC) + page script |

## Chromium engine

`web_engine = chromium` (`kitchen new <name> --engine chromium`, or `kitchen set
web_engine=chromium` later) runs the app on Chromium instead of WKWebView **on macOS**, through
[`tauri-runtime-cef`](https://crates.io/crates/tauri-runtime-cef) (CEF, Tauri 3 alpha). Windows
keeps WebView2, which is Chromium already, and Linux keeps WebKitGTK (CEF there needs GTK 4,
which the rest of the app does not use yet). Use it when a site you wrap breaks in Safari's
engine; try a User-Agent override and the proxy first (both cost nothing).

It is a cargo feature, `chromium`, that is on only in apps generated with it. On macOS the
build script then sets `cfg(cef)`: `src-tauri/src/engine.rs` hands `tauri_runtime_cef::Cef` to
`Builder::runtime`, `main()` gets `#[cef_entry_point]` (CEF starts the executable again for
its child processes), and the web wrapper's engine hooks (`src-tauri/src/wrapper/engine.rs`)
switch to CEF. Every other part of the app uses Tauri's runtime-erased types, so it does not
change.

| Web wrapper hook | On Chromium |
|---|---|
| Navigation, `target=_blank`, `window.open` | The same `on_navigation` / `on_new_window` and the same origin guard: other origins open in the system browser |
| Downloads | The same `on_download`: the Downloads folder under a free name, no save dialog |
| Cookies and storage | A Chromium profile in `<data>/cef`, with session cookies kept across restarts; cookies are encrypted with the macOS Keychain |
| Mute | `CefBrowserHost::SetAudioMuted`: the whole page, iframes and WebRTC included (no private API) |
| Camera, microphone, screen | The same `on_permission_request` and `desktop_core::wrapper::perm`; kinds Tauri has no name for get Chromium's own answer, so storage access and sign-in flows work; notifications, geolocation, USB, serial and HID are blocked |
| Pinned self-signed site (`insecureTls`) | Always through the loopback proxy (rustls with the pin, as on the other platforms); the window itself refuses every certificate error, so there is nothing to click through |
| Request headers | Through the proxy |
| Key capture | Unchanged (the `CGEventTap` does not depend on the engine) |

**Build.** Install the Tauri 3 CLI (`kitchen doctor` says which), `cmake` and `ninja`
(`brew install cmake ninja`). The first build downloads the CEF distribution (about 130 MB,
340 MB unpacked) into `CEF_PATH`, by default `~/Library/Caches/tauri-cef`; set `CEF_PATH`
to share one copy between apps and CI runs. `cargo run` does not work on macOS: CEF starts
its renderer and GPU processes from helper apps inside the bundle. `kitchen start`
(`cargo tauri dev`) builds and runs a `.app` for you.

**Bundle and sign.** `kitchen package mac` runs `cargo tauri build`; the 3.x CLI sees
`tauri-runtime-cef` in the manifest and does the CEF steps itself:

1. copies `Chromium Embedded Framework.framework` into `Contents/Frameworks/`;
2. builds the `tauri-cef-helper` executable against the app's own `cef` crate and writes it
   as five helper apps: `<App> Helper.app`, `Helper (GPU)`, `Helper (Renderer)`,
   `Helper (Plugin)` and `Helper (Alerts)` (bundle id `<identifier>.helper`, `LSUIElement`);
3. signs from the inside out (helpers and framework, then the executable, then the `.app`)
   with the hardened runtime, using `src-tauri/Entitlements.plist` merged with what CEF needs
   (`com.apple.security.cs.allow-jit`, `allow-unsigned-executable-memory`,
   `disable-library-validation`) for every signed item;
4. notarizes and staples when the usual `APPLE_*` variables are set (as for any build).

Expect a `.app` of about 340 MB and a `.dmg` of about 150 MB (today: about 10 MB). Every
update through `tauri-plugin-updater` is the full archive, about 150 MB. Chromium has security
fixes every two to four weeks, and macOS no longer patches the engine for you: plan releases
on that schedule. `tauri-runtime-cef` pins one CEF build, so a fix reaches you through a
runtime release.

**Not in the alpha CLI yet** (`tauri-cli` 3.0.0-alpha.6, `tauri-bundler` 3.0.0-alpha.5):

- No way to trim CEF's locales: all 220 `.lproj` folders ship (about 50 MB).
- One entitlements file for the app and every helper, so the renderer also gets the camera
  and microphone entitlements. A per-helper split needs a bundler change.
- No privacy strings (`NSCameraUsageDescription`, `NSMicrophoneUsageDescription`) in the helper
  `Info.plist`s. macOS normally asks on behalf of the main app, which has them; if a prompt
  names a helper, the strings have to be added before signing, which the CLI cannot do yet.

**Check on a real Mac first:** the helpers start and are signed, the camera and microphone
prompts, deep links through CEF's `NSApplication` (`tauri_deep_link`), the pinned site through
the proxy, and notarization of the larger bundle.

## Key-value store and cache

Two features, both off by default: `kitchen enable kv` and `kitchen enable cache`.

**Which one?**

| You need | Use |
|---|---|
| Small state that must survive a restart: UI preferences, drafts, flags, tokens, counters, "last synced" marks | `kv` |
| A faster answer for something you can always rebuild: a slow call, a parsed file, a remote lookup | `cache` |
| Queries over many records, joins, full text or vector search, large data | Postgres (`postgres`) or the document store |
| Big files | plain files in the data folder |

Web-dev mapping: `localStorage` (and Redis used as a store) → `kv`; Redis used as a cache
(or `lru-cache` with a TTL) → `cache`.

**kv** (`desktop_core::kv`) keeps everything in `<data>/kv.redb`, opened on first use by
`Engine::kv()`. Values are bytes or JSON in namespaces (`kv.ns("settings")`), each key can
have a TTL (expired keys read as missing at once and are swept every `kv.sweepMinutes`), and
`compare_and_swap`, `incr` and `transaction` change several keys atomically. Values are
capped (`kv.maxValueKb`), and so is the file (`kv.maxSizeMb`). Namespaces in
`kv.encryptedNamespaces` are sealed with ChaCha20-Poly1305 under a key in `<data>/kv.key`
(owner-only) or, with `kv.keySource = keychain` and the `oauth` feature, the OS keychain. A
backup copies the store next to the backup file (`<backup>.kv`, sealed with the backup key
when backups are encrypted); restoring brings it back.

```rust
let settings = engine.kv()?.ns("settings")?;
settings.set_json_ttl("draft", &draft, std::time::Duration::from_secs(3600))?;
let n = settings.incr("launches", 1)?;
```

From the UI: `api.kv_set({ ns: 'ui', key: 'theme', value: 'dark' })`, `api.kv_get`,
`api.kv_list`, `api.kv_delete`. The window, the HTTP API (`GET/PUT/DELETE /v1/kv/{ns}/{key}`,
`GET /v1/kv/{ns}`) and the MCP tools (`kv_get`, `kv_set`, `kv_list`, offered only when named
in `mcp.tools` or `mcp.policies`) reach only the namespaces in `kv.uiNamespaces` (default
`ui`); anything else is refused with `denied` (403 over HTTP). Rust code can use any
namespace. The file is opened by one process at a time: with `ipc_worker` the window, MCP
and HTTP share the worker's; without it, `<app> --mcp` cannot use the store while the
window has it open.

**cache** (`desktop_core::cache::Cache`) wraps moka: W-TinyLFU eviction by weight, TTL and
time to idle, hit and miss counts, and `get_or_insert_with`, which runs the loader once
however many threads ask for the same key. `Engine::cache()` is the app's own JSON cache,
sized by `cache.maxMb`; with `cache.persist` (and `kv`) it is written through to kv
namespace `cache`. Settings → Cache shows the stats and clears it (`cache_stats`,
`cache_clear`). The AI answer cache (`llm.cache`) is separate: it finds answers by question
similarity, which a key lookup cannot do.

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
| [`models`](../desktop-core/src/models.rs) | always | Models built into the installer (`models.lock`), and where each model loads from |
| [`bindings`](../desktop-core/src/bindings.rs) | `ts` (tests only) | Typed IPC: TypeScript types for the Tauri commands ([Typed IPC](#typed-ipc)) |
| [`governor`](../desktop-core/src/governor.rs) | always | CPU share and background priority for ingest |
| [`audio`](../desktop-core/src/audio.rs) | always | WAV, PCM, resampling, base64 |
| [`i18n`](../desktop-core/src/i18n.rs) | `i18n` | Translations: Fluent catalogue, `t!`, plurals, numbers and dates per locale, the OS language, right-to-left ([Translations](#translations)) |
| [`untrusted`](../desktop-core/src/untrusted.rs) | always | Size caps for outside input, capped reads, panic guard ([Untrusted input](#untrusted-input)) |
| [`embed`](../desktop-core/src/embed.rs) | `embed` | Local ONNX embeddings (fastembed) |
| [`pdf`](../desktop-core/src/pdf.rs) | `pdf` | PDF text, with OCR for pages without text; parsed in `<app> --parse` |
| [`sandbox`](../desktop-core/src/sandbox.rs) | `pdf` | Run a parser in a child process with a time and memory limit |
| [`pgstore`](../desktop-core/src/pgstore.rs) | `postgres` (`bm25`) | Embedded Postgres + pgvector (+ BM25) store |
| [`pgupgrade`](../desktop-core/src/pgupgrade.rs) | `postgres` | Upgrading the data folder to a new major Postgres with `pg_upgrade` ([Postgres upgrades](#postgres-upgrades)) |
| [`watch`](../desktop-core/src/watch.rs) | `watch` | Drop folder and watched folders |
| [`llm`](../desktop-core/src/llm.rs) | `ai` | Chat client for cloud and local models: retries, fallback chain, budgets, prompt caching; API keys; RAG prompt |
| [`meter`](../desktop-core/src/meter.rs) | `ai` | AI cost meter: price table, per-day and per-month usage, budgets |
| [`cache`](../desktop-core/src/cache.rs) | `ai`, `cache` | [`semantic`](../desktop-core/src/cache/semantic.rs) (`ai`): the answer cache of `Engine::ask`; [`memory`](../desktop-core/src/cache/memory.rs) (`cache`): the in-memory cache ([Key-value store and cache](#key-value-store-and-cache)) |
| [`kv`](../desktop-core/src/kv.rs) | `kv` | Embedded key-value store (redb): namespaces, TTLs, CAS, transactions, encryption per namespace |
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
| [`wrapper`](../desktop-core/src/wrapper/mod.rs) | `web-wrapper` | The web app's settings and their one check, origin guard, deep links, certificate pinning, header and auto-fill rules, media permissions, the page script, the loopback proxy ([Wrap a web app](#wrap-a-web-app)) |
| [`keymap`](../desktop-core/src/keymap.rs) | `web-wrapper` | Key codes from macOS, Windows and Linux to DOM `code`/`key`, modifiers, the release shortcut |
| [`telemetry`](../desktop-core/src/telemetry.rs) | `telemetry` | Opt-in crash reports, analytics and traces |
| [`eval`](../desktop-core/src/eval.rs) | `evals` | Search and RAG evals: recall@k, MRR, nDCG@k, latency, baselines, LLM judge |

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
| [`error`](../src-tauri/src/error.rs) | always | `CommandError`, the `{ code, message, details? }` shape of command errors, and the stable `codes` |
| [`ipc`](../src-tauri/src/ipc.rs) | `ipc-worker` | The worker process and its local-socket protocol |
| [`agent`](../src-tauri/src/agent.rs) | `background-agent` | Start the worker at login (launchd, Run key, systemd) |
| [`mcp`](../src-tauri/src/mcp.rs) | `mcp` | `--mcp`: tools mapped to the service; client registration |
| [`update`](../src-tauri/src/update.rs) | `updater` | Signed auto-update from GitHub releases |
| [`single`](../src-tauri/src/single.rs) | `single-instance` | One window at a time |
| [`python`](../src-tauri/src/python.rs) | `python-example` | A Python child process behind a command |
| [`claude`](../src-tauri/src/claude.rs) | `claude-mcp` | Register a Python MCP server in Claude Desktop through bundled uv |
| [`webview`](../src-tauri/src/webview.rs) | `webview` | Remote pages in app windows: allowlist hook, isolation, no IPC, bridge |
| [`wrapper`](../src-tauri/src/wrapper/mod.rs) | `web-wrapper` | The web app: its window, menu, Preferences, the engine hooks (`engine.rs`), and the per-OS TLS, key capture, media and mute hooks |
| [`eval`](../src-tauri/src/eval.rs) | `evals` | `--eval`: the command line of the search evals |

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
| `release_channels` | `updater` | channels, staged rollout and rollback steer the updater |
| `tauri_dialog`, `tauri_fs`, `tauri_clipboard`, `tauri_notification`, `tauri_opener`, `tauri_store`, `tauri_global_shortcut`, `tauri_autostart`, `tauri_deep_link`, `tauri_window_state`, `tauri_os`, `tauri_process`, `tauri_log`, `tauri_http`, `tauri_positioner`, `tauri_shell` | `tauri_plugins` | each Tauri plugin is wired in only with tauri_plugins |
| `macos_pkg` | `installers` | the release workflow builds it |
| `install_scripts` | `installers` | they download what the release workflow builds |
| `linux` | `installers` | the release workflow builds them |
| `web_insecure_tls`, `capture_keys`, `web_microphone`, `web_muted`, `web_camera`, `web_screen` | `web_wrapper` | it is a choice about the wrapped web app |
| `bundle_embedding_model` | `embeddings` | it puts that feature's model inside the installer |
| `bundle_stt_model` | `stt` | it puts that feature's model inside the installer |
| `bundle_tts_voice` | `tts` | it puts that feature's model inside the installer |
| `bundle_llm` | `local_llm` | it puts that feature's model inside the installer |

Warnings (it still generates):

| Feature | Without | What is missing |
|---|---|---|
| `oauth` | `tauri_deep_link` | the deep-link redirect (oauth.redirect = deepLink) needs the deep-link plugin; without it sign-in uses the loopback redirect |
| `ipc_worker`, `background_agent`, `daemon`, `updater`, `release_channels`, `single_instance`, `mcp_server`, `python_example`, `claude_mcp`, `watch_folders`, `postgres`, `http_api`, `webview`, `web_wrapper`, `tray_menu`, `app_shell`, `tauri_global_shortcut`, `tauri_autostart`, `tauri_window_state`, `tauri_process`, `tauri_positioner`, `tauri_shell` | a desktop target (`mobile` on) | desktop only: compiled out on iOS and Android |

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
| `background_jobs` | croner: MIT; jiff: Unlicense OR MIT | none (MIT / Apache-2.0) |
| `web_wrapper` | rustls: Apache-2.0 OR ISC OR MIT; webpki-roots: CDLA-Permissive-2.0; regex, keyring: MIT OR Apache-2.0 | webpki-roots (Mozilla's root certificates) is CDLA-Permissive-2.0: keep its licence text with the app |
| `http_api` | axum: MIT; tokio: MIT | none (MIT / Apache-2.0) |
| `i18n` | fluent-bundle, fluent-syntax, intl-memoizer, unic-langid, sys-locale: MIT OR Apache-2.0; @fluent/bundle (React UI): Apache-2.0; jiff: Unlicense OR MIT | none (MIT / Apache-2.0) |
| `kv` | redb: MIT OR Apache-2.0 | none (MIT / Apache-2.0) |
| `cache` | moka: (MIT OR Apache-2.0) AND Apache-2.0 | none (MIT / Apache-2.0) |
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
| `kitchen start` (`dev`) | `cargo tauri dev` with `KITCHEN_ENV=development`: Vite HMR and Rust rebuilds. Installs the UI's npm packages on the first run; `--install` also installs a missing Tauri CLI |
| `kitchen test [--unit\|--feature]` | Unit tests and feature tests ([Tests](#tests)), with `KITCHEN_ENV=test` |
| `kitchen package [mac\|windows\|linux] [--release]` | `cargo tauri build` with this OS's bundles, copied into `./packages`. Other OSes are refused |
| `kitchen publish [--tag v1.2.3] [--yes]` | A GitHub release with `./packages` through `gh`; a dry run without `--yes` |
| `kitchen eval [dataset] [--baseline file]` | Run the app's search evals (`<app> --eval`, feature `evals`) and fail on a regression ([Search evals](#search-evals)) |
| `kitchen models list\|add\|remove\|fetch\|verify` | Models built into the installer ([Bundling models](#bundling-models)); `kitchen set models=bundle` (or `kitchen enable bundled_models`) turns bundling on |
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
