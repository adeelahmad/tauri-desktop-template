# Testing

Three kinds of tests, from fastest to slowest:

1. **Unit tests and doctests** in `desktop-core` (the engine). Nothing to download, no
   WebView needed. Run these all the time.
2. **The self-test**: `{{project-name}} --self-test` indexes and searches a sample with
   every store the build has, and prints a JSON report. CI runs it on every installed build.
3. **Feature tests**: the tests of one optional part, run with its cargo feature on. Some
   need a model, a server or a device, and skip with a note when it is missing.

## Every OS

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p desktop-core                  # unit tests and doctests, default features off
cargo test -p desktop-core --doc            # doctests only
cargo test --workspace                      # also the app crate (needs the WebView libraries below)
{%- if python_example %}
python3 python/test_worker.py               # the Python worker
{%- endif %}
```

`make test` and `make lint` wrap these.
{% if frontend == "react" %}
The React UI must be built before the app crate compiles (it embeds `ui-react/dist`):

```sh
cd ui-react
npm ci --include=dev
npx prettier --check src
npx eslint src
npm run build
```
{% endif %}
## Self-test

```sh
cargo run -p {{project-name}} -- --self-test               # report on stdout
cargo run -p {{project-name}} -- --self-test report.json   # report in a file too
```

It exits non-zero when any store fails to find the right chunk.{% if embeddings %} The first
run downloads the embedding model; CI retries it, since a download can fail midway.{% endif %}

## Feature tests

Turn on the cargo feature of the part you changed (the names are in `src-tauri/Cargo.toml`
and [AGENTS.md](AGENTS.md)):

```sh
cargo test -p desktop-core --features <cargo feature>
cargo clippy -p desktop-core --all-features -- -D warnings   # every optional part compiles
```
{% if email %}
- Email: `cargo test -p desktop-core --features mail -- --ignored mail` talks to real SMTP
  servers (465 and 587).
{%- endif %}
{%- if postgres %}
- Postgres: the embedded server downloads on first use. `APP_PG_URL=postgres://...` uses an
  existing server with pgvector instead (`make db-up` starts one in Docker).
{%- endif %}
{%- if tts %}
- Text to speech: set `APP_TEST_TTS_DIR` to a folder with `model.onnx` and `lexicon.tsv`.
{%- endif %}
{%- if stt %}
- Speech to text: set `APP_TEST_STT_DIR` to the Whisper base model, and `APP_TEST_STT_WAV`
  to an English recording (or `APP_TEST_TTS_DIR`, to have the built-in voice read one).
{%- endif %}
{%- if local_llm %}
- llama.cpp: set `APP_TEST_GGUF` to a small `.gguf` file (CI uses stories260K, about 1 MB).
{%- endif %}

Tests that need something missing print `skipped: ...` and pass.

## Linux

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

`cargo test -p desktop-core` needs none of these; the app crate does. The self-test opens
no window.

## macOS

Install the Xcode command line tools (`xcode-select --install`). CI also links the clang
runtime when it builds the app (see `.github/workflows/ci.yml`); do the same if linking
fails with undefined clang runtime symbols:

```sh
export RUSTFLAGS="-C link-arg=$(xcrun clang --print-runtime-dir)/libclang_rt.osx.a"
```

## Windows

Install the Visual Studio Build Tools (C++ workload) and use the MSVC Rust toolchain.
WebView2 ships with Windows 10 and 11. Release builds have no console, so pass a file to
the self-test to see its report: `{{project-name}}.exe --self-test report.json`.

## Before a commit

`kitchen doctor` checks the toolchain and the generator markers. Then run the
[Every OS](#every-os) block, plus the feature tests of the parts you touched.
