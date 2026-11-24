# AGENTS.md

Instructions for AI agents (and people) working in **{{app_name}}**, a Tauri 2 desktop app
made with Kitchen. Read this first, then [docs/GUIDE.md](docs/GUIDE.md) and the `//!` doc of
the module you are changing. [TESTING.md](TESTING.md) has the test commands for each OS, and
[BRAND.md](BRAND.md) has the name, icon and colours.

## Layout

```text
Cargo.toml              workspace: desktop-core + src-tauri
kitchen.toml            the answers this app was generated with (change them with `kitchen`)
desktop-core/           the engine: plain Rust, no Tauri, one module per part
  src/lib.rs            module list (marker: kitchen:modules)
  src/config.rs         every setting, off by default (markers: kitchen:config, kitchen:validate)
  src/net.rs            every outgoing request goes through here
src-tauri/              the desktop app
  src/lib.rs            Tauri commands: thin, arguments -> service call (kitchen:mods, kitchen:handlers)
  src/service.rs        one dispatch by method name (kitchen:methods)
  src/guard.rs          navigation guard: only the app's own pages
  tauri.conf.json       window, bundle and the content security policy
  capabilities/         which Tauri permissions the window has
{%- if frontend == "react" %}
ui-react/               React 19 + Vite frontend (built into ui-react/dist)
{%- else %}
ui/                     plain HTML/JS frontend, no build step
{%- endif %}
{%- if python_example %}
python/                 Python worker example (JSON over stdio)
{%- endif %}
docs/GUIDE.md           architecture, the wiring checklist, module index
.claude/skills/         task recipes for agents (feature, generate, port an app, release)
.mcp.json               MCP servers for agents: `kitchen mcp`{% if mcp_server %} and this app's own{% endif %}
```

## Features in this build

Features are chosen when the app is generated (`kitchen.toml` records them). The ones that
run inside the app are also cargo features, with a switch in Settings that starts **off**.
This build has:

{% if embeddings -%}
- `embeddings` (cargo feature `embed`): local ONNX text embeddings, `desktop-core/src/embed.rs`.
{% endif -%}
{% if postgres -%}
- `postgres` (cargo feature `postgres`): embedded Postgres + pgvector store, `desktop-core/src/pgstore.rs`.
{% if bm25 -%}
- `bm25` (cargo feature `bm25`): BM25 keyword ranking in Postgres for hybrid search.
{% endif -%}
{% endif -%}
{% if watch_folders -%}
- `watch_folders` (cargo feature `watch`): drop folder and watch folders, `desktop-core/src/watch.rs`.
{% endif -%}
{% if ai -%}
- `ai` (cargo feature `ai`): chat over the documents with cloud or local LLMs, `desktop-core/src/llm.rs`.
{% if local_llm -%}
- `local_llm` (cargo feature `llama`): llama.cpp built in, `desktop-core/src/local.rs`.
{% endif -%}
{% endif -%}
{% if tts -%}
- `tts` (cargo feature `tts`): text to speech, `desktop-core/src/tts.rs`.
{% endif -%}
{% if stt -%}
- `stt` (cargo feature `stt`): speech to text with Whisper, `desktop-core/src/stt.rs`.
{% endif -%}
{% if email -%}
- `email` (cargo feature `mail`): SMTP email, `desktop-core/src/mail.rs`. The password is kept outside `settings.json`.
{% endif -%}
{% if plugins -%}
- `plugins` (cargo feature `plugins`): compiled-in plugin API, `desktop-core/src/plugin.rs` (marker: kitchen:plugins).
{% endif -%}
{% if rclone -%}
- `rclone` (cargo feature `rclone`): cloud sync of backups, `desktop-core/src/rclone.rs`.
{% endif -%}
{% if tailscale -%}
- `tailscale` (cargo feature `tailscale`): tailnet status and `tailscale serve`, `desktop-core/src/tailscale.rs`.
{% endif -%}
{% if telemetry -%}
- `telemetry` (cargo feature `telemetry`): opt-in crash reports, analytics and traces, `desktop-core/src/telemetry.rs`.
{% endif -%}
{% if mcp_server -%}
- `mcp_server` (cargo feature `mcp`): this app as an MCP server (`<app> --mcp`), `desktop-core/src/mcp.rs`, `src-tauri/src/mcp.rs`.
{% endif -%}
{% if pdf -%}
- `pdf` (cargo feature `pdf`): PDF text extraction with OCR fallback, `desktop-core/src/pdf.rs`.
{% endif -%}
{% if wizard -%}
- `wizard` (cargo feature `wizard`): first-run setup wizard.
{% endif -%}
{% if single_instance -%}
- `single_instance` (cargo feature `single-instance`): a second launch focuses the running window, `src-tauri/src/single.rs`.
{% endif -%}
{% if ipc_worker -%}
- `ipc_worker` (cargo feature `ipc-worker`): the backend runs as `<app> --worker` over a local socket, `src-tauri/src/ipc.rs`.
{% if background_agent -%}
- `background_agent` (cargo feature `background-agent`): starts the worker at login, `src-tauri/src/agent.rs`.
{% endif -%}
{% endif -%}
{% if python_example -%}
- `python_example` (cargo feature `python-example`): Python child process over stdio JSON, `python/`, `src-tauri/src/python.rs`.
{% endif -%}
{% if updater -%}
- `updater` (cargo feature `updater`): signed auto-update from GitHub releases, `src-tauri/src/update.rs`.
{% if release_channels -%}
- `release_channels` (cargo feature `release-channels`): stable/beta update channels, staged rollout and rollback, `desktop-core/src/release.rs`, `src-tauri/src/update.rs`.
{% endif -%}
{% endif -%}
{% if supply_chain -%}
- `supply_chain` (cargo feature `supply-chain`): cargo-deny, cargo-audit and SBOMs in `.github/workflows/supply-chain.yml`; licence notices made by `src-tauri/build.rs` (`desktop-core/src/notices.rs`) and shown in Settings → About.
{% endif -%}
{% if claude_mcp -%}
- `claude_mcp` (cargo feature `claude-mcp`): bundled uv registers a Python MCP server in Claude Desktop, `src-tauri/src/claude.rs`.
{% endif -%}
{% if mobile -%}
- `mobile`: iOS and Android builds; desktop-only code is behind `cfg(desktop)`.
{% endif -%}
{% if installers -%}
- `installers`: the Release workflow builds and tests installers, `.github/workflows/release.yml`.
{% if macos_pkg -%}
- `macos_pkg`: also a macOS .pkg that clears the quarantine flag.
{% endif -%}
{% if install_scripts -%}
- `install_scripts`: one-line install scripts in `scripts/`.
{% endif -%}
{% if linux -%}
- `linux`: also Linux installers (.AppImage, .deb).
{% endif -%}
{% endif -%}
{% if tauri_plugins -%}
- `tauri_plugins`: official Tauri plugins, permissions in `src-tauri/capabilities/`:
{%- if tauri_dialog %} `tauri_dialog`{% endif %}
{%- if tauri_fs %} `tauri_fs`{% endif %}
{%- if tauri_clipboard %} `tauri_clipboard`{% endif %}
{%- if tauri_notification %} `tauri_notification`{% endif %}
{%- if tauri_opener %} `tauri_opener`{% endif %}
{%- if tauri_store %} `tauri_store`{% endif %}
{%- if tauri_global_shortcut %} `tauri_global_shortcut`{% endif %}
{%- if tauri_autostart %} `tauri_autostart`{% endif %}
{%- if tauri_deep_link %} `tauri_deep_link`{% endif %}
{%- if tauri_window_state %} `tauri_window_state`{% endif %}
{%- if tauri_os %} `tauri_os`{% endif %}
{%- if tauri_process %} `tauri_process`{% endif %}
{%- if tauri_log %} `tauri_log`{% endif %}
{%- if tauri_http %} `tauri_http` (only the URLs in `capabilities/default.json`){% endif %}
{%- if tauri_positioner %} `tauri_positioner`{% endif %}
{%- if tauri_shell %} `tauri_shell` (only the commands in `capabilities/desktop.json`; never add a shell, an interpreter or `"args": true`){% endif %}.
{% endif -%}
{% if app_shell -%}
- `app_shell` (cargo feature `app-shell`): custom titlebar, themes (light/dark/system) and fonts, Settings in its own window (Cmd/Ctrl+,), `src-tauri/src/shell.rs`, `desktop-core/src/appearance.rs`, `capabilities/{shell,settings}.json`. Settings values reach CSS and a pre-load script, so validate new ones in `AppearanceConfig::validated`; add a theme in `THEMES` and `shell.css`.
{% endif -%}
- Always on: security defaults (`src-tauri/src/guard.rs`, CSP), JSON-lines logs, `--self-test`,
  dependency policy in `deny.toml` (`make audit` / `kitchen audit`),
  `.env`/`.env.local` in debug builds only (`desktop_core::dotenv`), git hooks in `.githooks/`
  (`kitchen hooks install`).

`kitchen features` lists every other feature you can turn on.

## Golden rules

1. **Secure defaults.** New settings start off or at the safest value. Every outgoing
   request goes through `desktop_core::net`. Secrets live in their own owner-only files,
   never in `settings.json`.
2. **Everything behind features.** New optional code gets a cargo feature that is off by
   default, plus a run-time switch in Settings that starts off. Desktop-only code goes
   behind `cfg(desktop)`.
3. **Docs with examples.** Every module gets a `//!` doc with a runnable example (doctest;
   `no_run` only for network or devices), `///` docs on public items, and unit tests.
4. **Thin commands.** Logic goes in `desktop-core`; Tauri commands only turn arguments into
   a service call. Reuse the crates already in the tree before adding a dependency. A new
   command or a changed struct it returns means new typed IPC bindings: run
   `cargo test -p {{project-name}} export_bindings` and commit them (docs/GUIDE.md, "Typed IPC").
5. **Run `kitchen doctor`** before you start and after big changes: it checks the toolchain
   and that the generator markers are still in place.

## Adding things

```sh
kitchen features                                   # what is on and off
kitchen enable <feature> --dry-run                 # see the files it would change
kitchen enable <feature>                           # turn it on (3-way merge keeps your edits)
kitchen disable <feature>                          # turn it off
kitchen set <name>=<value>                         # change another answer, e.g. frontend

kitchen generate module <name> --config            # desktop-core/src/<name>.rs + settings section
kitchen generate setting <module>.<key> <bool|int|float|text|list> [default] --label "<text>"
kitchen generate command <name>                    # Tauri command + service method
{%- if frontend == "react" %}
kitchen generate page <Name>                       # React page on the main screen
{%- endif %}
```

Add `--json` for machine-readable output. Every generator writes a documented, tested file
and wires it in at the markers below. The skills in `.claude/skills/` walk through each task.

## MCP servers

`.mcp.json` registers `kitchen mcp` (install the CLI with
`cargo install --git https://github.com/adeelahmad/tauri-desktop-template desktop-kitchen`),
so an agent can run `features`, `enable`, `disable`, `set`, `upgrade`, `generate` and
`doctor` as tools{% if mcp_server %}, and this app's own MCP server (`cargo run -p {{project-name}} -- --mcp`).
That server stays off until it is switched on in the app under Settings → MCP server, after
setup is finished{% if frontend == "react" %}; build the UI once first (`make ui`), since the
app embeds `ui-react/dist`{% endif %}{% endif %}.

## Tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p desktop-core                         # engine tests, nothing to download
cargo test --workspace                             # also the app (needs the system WebView libraries)
cargo test -p desktop-core --features <cargo feature>   # one optional part
cargo test -p {{project-name}} export_bindings     # typed IPC: rewrite the UI's TypeScript types (CI: fails if stale)
cargo run -p {{project-name}} -- --self-test       # index and search a sample, JSON report
{%- if python_example %}
python3 python/test_worker.py
{%- endif %}
{%- if frontend == "react" %}
(cd ui-react && npx prettier --check src && npx eslint src && npm run build)
(cd ui-react && npm run a11y)                      # axe-core on every screen + pseudo-locale layout (needs Playwright)
{%- endif %}
```

`make test` and `make lint` run the usual set. [TESTING.md](TESTING.md) has the details.

## Markers

The generators insert code at these comments. Never remove or reword them.

| Marker | File |
|---|---|
| `// kitchen:modules` | `desktop-core/src/lib.rs` |
| `// kitchen:config`, `// kitchen:validate` | `desktop-core/src/config.rs` |
| `// kitchen:fields`, `// kitchen:defaults` | each generated module with `--config` |
| `// kitchen:mods`, `// kitchen:handlers`, `// kitchen:bindings` | `src-tauri/src/lib.rs` |
| `// kitchen:methods` | `src-tauri/src/service.rs` |
{%- if plugins %}
| `// kitchen:plugins` | `desktop-core/src/plugin.rs` |
{%- endif %}
{%- if frontend == "react" %}
| `// kitchen:imports`, `{/* kitchen:pages */}` | `ui-react/src/App.jsx` |
| `// kitchen:settings`, `// kitchen:titles`, `// kitchen:sections` | `ui-react/src/Settings.jsx` |
{%- endif %}

## Never

- Never put secrets (API keys, passwords, tokens) in `settings.json`, `tauri.conf.json`,
  `kitchen.toml`, `.cargo/config.toml` or any committed file. Use the owner-only key files
  the app already has, or build-time CI secrets.
- Never give remote content IPC: the window only shows the app's own pages, no capability
  lists a remote URL, and external pages open in the system browser, not in the app's
  window. Fetch remote data in Rust through `desktop_core::net` instead.
- Never disable or loosen the CSP in `src-tauri/tauri.conf.json` (no `'unsafe-eval'`, no
  remote `script-src`, never `"csp": null`), and never turn off `freezePrototype`.
- Never remove the `// kitchen:` markers, or edit `kitchen.toml` by hand.
- Never turn a feature on by default.
