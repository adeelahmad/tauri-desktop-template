# Tauri desktop app template

A [cargo-generate](https://github.com/cargo-generate/cargo-generate) template for
cross-platform desktop apps (macOS, Windows, Linux) in Rust + Tauri 2, extracted from the
AnkiPlus desktop app. Every part is a yes/no question when you generate, and a cargo
feature afterwards.

| Option | What you get |
|---|---|
| `frontend` | React 19 + Vite (`ui-react/`) or plain HTML/JS (`ui/`) |
| `embeddings` | Local ONNX text embeddings via fastembed-rs; pick Nomic v1.5, BGE small, MiniLM or multilingual E5 |
| `postgres` | Embedded Postgres 16 + pgvector (downloads on first run, HNSW search, our own extension builds: no AVX-512 needed) |
| `bm25` | BM25 keyword ranking in Postgres (VectorChord-BM25), for hybrid BM25 + vector search. Default off |
| `watch_folders` | A drop folder plus watch folders that ingest new files automatically. Default off |
| `ai` | Ask questions over your documents with Claude, OpenAI, Gemini, Ollama or any OpenAI-compatible server (llama.cpp, LM Studio, vLLM): streaming, citations, retries, fallback providers, token budgets, prompt caching. Keys stay in an owner-only file in the app data folder. No extra dependencies. Default off |
| `local_llm` (with `ai`) | llama.cpp compiled in and linked statically (Metal on Apple Silicon): GGUF models run inside the app with no server. Provider `local`, a one-click model download (resumable), context, GPU layers and thread settings. Default off |
| `tts` | Text to speech in pure Rust over ONNX Runtime with tiny-voca (about 25 MB, two voices, speed and pitch, own lexicon-based phonemizer, no espeak). One-click voice download with SHA-256 check; "Read aloud" on answers. Default off |
| `stt` | Speech to text with OpenAI Whisper (tiny 40 MB, base 75 MB, small 250 MB; 8-bit ONNX, pinned and checksummed) on the same ONNX Runtime: no whisper.cpp, no second ggml next to llama.cpp. Log-mel features, resampling, WAV reading and greedy decoding are plain Rust. Microphone dictation into search and Ask, audio files, language detection, translate to English, a length limit. macOS microphone prompt and entitlement included. Default off |
| `email` | Native SMTP in Rust: SSL/TLS (465) and STARTTLS (587), AUTH PLAIN/LOGIN, HTML, attachments, certificate pins; the password stays in an owner-only file. Default off |
| `telemetry` | Opt-in crash reports (Sentry envelopes), anonymous analytics (PostHog or Aptabase) and traces (OTLP/HTTP JSON to any collector), all over the existing HTTP client with no SDK crates. A panic hook writes crashes locally with the home folder scrubbed and sends them at the next start only if allowed; every backend call is timed in one place; only call names, timings, counts, OS/arch and a random install id can leave. Keys come from release secrets. Default off |
| `background_jobs` | A persistent background job queue on std threads: named job types with typed JSON payloads, delayed and cron/interval schedules, per-queue concurrency, retries with exponential backoff and jitter, dead letters, cancel, timeouts and a pruned run history; respects the resource and network settings; runs in the worker with the background agent; foreground only on mobile. `kitchen generate job <name>` adds job types. Default off |
| `single_instance` | A second launch brings the running window to the front and exits, so two copies never share the index or embedded Postgres. Standard library only (file lock + loopback knock). Default off |
| `mcp_server` | A Model Context Protocol server written in Rust (stdio, `app --mcp`), so Claude Desktop, Claude Code and other MCP clients can search the documents. Read-only tools by default; write tools, a tool allowlist and an audit log are settings. One click adds it to Claude Desktop. No extra dependencies. Default off |
| `pdf` | PDF text via docling.rs, tesseract OCR for pages without a text layer |
| `ipc_worker` | Backend in its own process (`app --worker`) over a Unix socket / named pipe |
| `background_agent` | Starts the worker at login: macOS launchd LaunchAgent, Windows Run entry, Linux systemd user unit |
| `python_example` | A Python child process over stdio JSON, with tests |
| `wizard` | First-run setup: choose the store, download the model with progress |
| `updater` | Signed auto-update from GitHub releases (`latest.json` built by the release workflow) |
| `installers` | Release workflow: unsigned .dmg, .exe (and optional .pkg, .AppImage/.deb), each installed and `--self-test`ed on CI |
| `macos_pkg` | A .pkg whose postinstall clears the quarantine flag, so the unsigned app opens without prompts |
| `install_scripts` | `curl … \| sh` (macOS, no Gatekeeper prompt) and PowerShell one-liners |
| `claude_mcp` | Bundles uv and registers a Python MCP server in Claude Desktop (config backed up first) |
| `app_shell` | Custom titlebar (Windows and Linux buttons and drag region, macOS traffic lights over the content), six named themes in light, dark and system mode with no flash of the wrong theme, interface and editor fonts, and Settings in a second window (Cmd/Ctrl+,) with its own minimal capability. React and plain UI. Default off |
| `tauri_plugins` | Official Tauri plugins, one answer each: dialog, fs, clipboard, notification, opener, store, global shortcut, autostart, deep link, window state, os, process, log, http (URL allowlist), positioner, shell (explicit command list only). Default off |

Always included: a file store with BM25, vector and hybrid (RRF) search, configurable
chunking and parallel ingest workers (one settings file, everything optional off by default),
`--self-test` for CI, CI for macOS/Windows/Linux, generic icons, `.env`/`.env.local` loading in
development builds only (a tiny parser, no dependency, never bundled), and Rust-first git
hooks in `.githooks/` (`kitchen hooks install`: fmt and clippy, plus prettier and eslint for React).
Security defaults are always on: navigation limited to the app's own pages, a strict CSP,
frozen JS prototypes, no global Tauri object with React, and dev tools only in debug builds
(`APP_DEVTOOLS=1` opens them). Each process writes JSON-lines logs, rotated by size, to the
app data `logs/` folder (`APP_LOG=warn,desktop_core=debug`). Rust 1.89 or newer.
Ingest is crash-safe: each multi-file import is journaled, and one cut short by a crash or
quit resumes at the next start. Indexing threads run at background priority and use a
configurable share of the CPU (macOS QoS, Windows background mode, Linux nice, with no crates).
Every chunk carries its provenance (file SHA-256, size and time, extraction method including
PDF OCR, position, embedding model, when it was indexed) into search hits, citations and MCP
results. Stores are versioned: Postgres runs numbered migrations in transactions, and a store
from a newer app version is refused instead of being damaged.

## Feature dependencies

Some answers only work with others. `cargo generate` refuses a combination that does not
work (the template's pre-hook) and Kitchen does the same before it changes anything;
`kitchen enable <feature> --with-deps` turns on what a feature needs.

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

Warnings (it still generates):

| Feature | Without | What is missing |
|---|---|---|
| `oauth` | `tauri_deep_link` | the deep-link redirect (oauth.redirect = deepLink) needs the deep-link plugin; without it sign-in uses the loopback redirect |
| `ipc_worker`, `background_agent`, `daemon`, `updater`, `release_channels`, `single_instance`, `mcp_server`, `python_example`, `claude_mcp`, `watch_folders`, `postgres`, `http_api`, `webview`, `tray_menu`, `app_shell`, `tauri_global_shortcut`, `tauri_autostart`, `tauri_window_state`, `tauri_process`, `tauri_positioner`, `tauri_shell` | a desktop target (`mobile` on) | desktop only: compiled out on iOS and Android |

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
| `http_api` | axum: MIT; tokio: MIT | none (MIT / Apache-2.0) |
<!-- /kitchen:feature-dependencies -->

## Use

With the [Kitchen CLI](kitchen/README.md) (recommended: turn features on and off later,
generate code, check your machine, MCP server for agents):

```sh
cargo install --git https://github.com/<owner>/<this-repo> desktop-kitchen
cargo install cargo-generate
kitchen new my-app --enable email,ai
cd my-app && kitchen doctor && kitchen dev
```

Or with cargo-generate alone:

```sh
cargo install cargo-generate
cargo generate --git https://github.com/<owner>/<this-repo> template --name my-app
cd my-app && cargo tauri dev
```

For a private repository, cargo-generate uses your git credentials (the repository's SSH
clone URL works too). Non-interactive: pass answers with
`-d key=value` (see `.github/workflows/test-template.yml`).

The generated project's README covers developing, releasing, signing keys for the
updater, and the process layout.
