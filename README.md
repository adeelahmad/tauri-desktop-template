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
| `telemetry` | Opt-in crash reports (Sentry envelopes), anonymous analytics (PostHog or Aptabase) and traces (OTLP/HTTP JSON to any collector), all over the existing HTTP client with no SDK crates. A panic hook writes crashes locally with the home folder scrubbed and sends them at the next start only if allowed; every backend call is timed in one place; only call names, timings, counts, OS/arch and a random install id can leave. Keys come from release secrets. Default off |
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

Always included: a file store with BM25, vector and hybrid (RRF) search, configurable
chunking and parallel ingest workers (one settings file, everything optional off by default),
`--self-test` for CI, CI for macOS/Windows/Linux, generic icons.
Security defaults are always on: navigation limited to the app's own pages, a strict CSP,
frozen JS prototypes, no global Tauri object with React, and dev tools only in debug builds
(`APP_DEVTOOLS=1` opens them). Each process writes JSON-lines logs, rotated by size, to the
app data `logs/` folder (`APP_LOG=warn,desktop_core=debug`). Rust 1.89 or newer.

## Use

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
