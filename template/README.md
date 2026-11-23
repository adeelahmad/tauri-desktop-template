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
| Chunking, file store, keyword search | `desktop-core/src/{chunk,store,engine}.rs` | always |
{% if embeddings -%}
| Local ONNX embeddings (fastembed-rs, `{{embedding_model}}`) | `desktop-core/src/embed.rs` | `embed` |
{% endif -%}
{% if postgres -%}
| Embedded Postgres 17 + pgvector (HNSW cosine, full-text fallback) | `desktop-core/src/pgstore.rs` | `postgres` |
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
| `--self-test` (CI runs it on every installed build) | `self_test` in `src-tauri/src/lib.rs` | always |
{% if installers -%}
| Release workflow: unsigned .dmg{% if macos_pkg %}, .pkg{% endif %}, .exe{% if linux %}, .AppImage, .deb{% endif %}, tested after install | `.github/workflows/release.yml` | |
{% endif -%}
{% if install_scripts -%}
| One-line installers (`curl … \| sh`, PowerShell) | `scripts/` | |
{% endif %}
The default features are the parts you picked when generating. Turn one off for a build
with `--no-default-features --features ...`.

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

Linux needs `libwebkit2gtk-4.1-dev librsvg2-dev`.
{% if frontend == "react" -%}
The UI is React in `ui-react/`; `cargo tauri dev` starts Vite with hot reload, and
`cargo tauri build` builds it first. Build it once (`npm --prefix ui-react ci --include=dev && npm --prefix ui-react run build`)
before a plain `cargo build`, which embeds `ui-react/dist`.
{% else -%}
The UI is plain HTML/JS in `ui/` (no build step); point `build.frontendDist` in
`src-tauri/tauri.conf.json` at another framework's output to swap it.
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
{% if embeddings %}
`APP_EMBED_MODEL` (in `.cargo/config.toml`, or at run time) picks the model: one of
`nomic-embed-text-v1.5`, `bge-small-en-v1.5`, `all-MiniLM-L6-v2`, `multilingual-e5-small`.
Changing it re-embeds stored chunks on the next start.
{% endif -%}
{% if postgres %}
`APP_PG_URL` uses an existing Postgres with pgvector instead of the embedded one. The
embedded server with prebuilt pgvector runs on macOS and Windows (x86-64 needs AVX-512);
elsewhere, including Linux, the wizard offers only the file store unless `APP_PG_URL` is set.
The first download calls the GitHub API; set `GITHUB_TOKEN` if you hit its rate limit.
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
