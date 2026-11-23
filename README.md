# Tauri desktop app template

A [cargo-generate](https://github.com/cargo-generate/cargo-generate) template for
cross-platform desktop apps (macOS, Windows, Linux) in Rust + Tauri 2, extracted from the
AnkiPlus desktop app. Every part is a yes/no question when you generate, and a cargo
feature afterwards.

| Option | What you get |
|---|---|
| `frontend` | React 19 + Vite (`ui-react/`) or plain HTML/JS (`ui/`) |
| `embeddings` | Local ONNX text embeddings via fastembed-rs; pick Nomic v1.5, BGE small, MiniLM or multilingual E5 |
| `postgres` | Embedded Postgres 16 + pgvector (downloads on first run, HNSW search, AVX-512 guard) |
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

Always included: a file store with keyword search (works with everything off),
`--self-test` for CI, CI for macOS/Windows/Linux, generic icons.

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
