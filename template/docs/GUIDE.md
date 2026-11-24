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

- [Architecture](#architecture)
- [Processes and folders](#processes-and-folders)
- [How to add a feature](#how-to-add-a-feature)
- [How settings work](#how-settings-work)
- [Module index](#module-index)
- [Docs and checks](#docs-and-checks)
- [Kitchen CLI](#kitchen-cli)
- [Notes for AI agents](#notes-for-ai-agents)

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
| [`llm`](../desktop-core/src/llm.rs) | `ai` | Chat client for cloud and local models; API keys; RAG prompt |
| [`cache`](../desktop-core/src/cache.rs) | `ai` | Semantic answer cache for `Engine::ask` |
| [`local`](../desktop-core/src/local.rs) | `llama` | llama.cpp built in (GGUF) |
| [`mcp`](../desktop-core/src/mcp.rs) | `mcp` | MCP protocol, tool policy and audit log |
| [`tts`](../desktop-core/src/tts.rs) | `tts` | Text to speech (tiny-voca) |
| [`stt`](../desktop-core/src/stt.rs) | `stt` | Speech to text (Whisper) |
| [`plugin`](../desktop-core/src/plugin.rs) | `plugins` | Compiled-in plugins: ingest hooks, extra methods and settings |
| [`rclone`](../desktop-core/src/rclone.rs) | `rclone` | Sync backups (or the data folder) to a cloud with rclone |
| [`tailscale`](../desktop-core/src/tailscale.rs) | `tailscale` | Tailscale status and `tailscale serve` for one local port |
| [`mail`](../desktop-core/src/mail.rs) | `mail` | SMTP email with TLS, STARTTLS and attachments |
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
| [`ipc`](../src-tauri/src/ipc.rs) | `ipc-worker` | The worker process and its local-socket protocol |
| [`agent`](../src-tauri/src/agent.rs) | `background-agent` | Start the worker at login (launchd, Run key, systemd) |
| [`mcp`](../src-tauri/src/mcp.rs) | `mcp` | `--mcp`: tools mapped to the service; client registration |
| [`update`](../src-tauri/src/update.rs) | `updater` | Signed auto-update from GitHub releases |
| [`single`](../src-tauri/src/single.rs) | `single-instance` | One window at a time |
| [`python`](../src-tauri/src/python.rs) | `python-example` | A Python child process behind a command |
| [`claude`](../src-tauri/src/claude.rs) | `claude-mcp` | Register a Python MCP server in Claude Desktop through bundled uv |

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
| `kitchen generate module\|setting\|command\|page ...` | Generate documented, tested code, wired in at the `// kitchen:...` markers |
| `kitchen doctor` | Check the toolchain (Rust, cargo-generate, Tauri CLI, Node, WebKitGTK or Xcode) and the app's markers |
| `kitchen mcp` | Offer the same commands to AI agents as an MCP server over stdio (`claude mcp add kitchen -- kitchen mcp`) |

Add `--json` to any command for machine-readable output. The CLI's own README has the
details, including `kitchen set`, `kitchen upgrade`, and how to override the generator
templates.

## Notes for AI agents

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
