# Common tasks. `make help` lists them.
APP := {{project-name}}
.DEFAULT_GOAL := help
{% if frontend == "react" %}
UI_DEPS := ui-react/node_modules
$(UI_DEPS): ui-react/package-lock.json
	npm --prefix ui-react ci --include=dev
	@touch $(UI_DEPS)
ui: $(UI_DEPS) ## Build the React UI (the app embeds ui-react/dist)
	npm --prefix ui-react run build
{% else %}
ui: ## Nothing to build: the UI is plain files in ui/
	@true
{% endif %}
.PHONY: help setup dev build test lint check-windows fmt audit self-test fuzz {% if evals %}eval {% endif %}{% if bundle_embedding_model or bundle_stt_model or bundle_tts_voice or bundle_llm %}models models-verify {% endif %}ui db-up db-down docker-test hooks clean

help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

setup: hooks ## Install the Tauri CLI and git hooks
	cargo install tauri-cli --version '^2' --locked

hooks: ## Install pre-commit (and commit-msg) hooks
	pre-commit install --hook-type pre-commit --hook-type commit-msg

dev: ## Run the app with hot reload
	cargo tauri dev

{% if bundle_embedding_model or bundle_stt_model or bundle_tts_voice or bundle_llm %}models: ## Fetch the models the installer bundles (src-tauri/resources/models/models.lock) and check their SHA-256s
	cargo run -q -p desktop-core --example models -- fetch

models-verify: ## Check the fetched models against models.lock (sizes and SHA-256s)
	cargo run -q -p desktop-core --example models -- verify

{% endif %}build: ui{% if bundle_embedding_model or bundle_stt_model or bundle_tts_voice or bundle_llm %} models{% endif %} ## Build installers for this machine (signed updater archives only with TAURI_SIGNING_PRIVATE_KEY set)
	cargo tauri build $(if $(TAURI_SIGNING_PRIVATE_KEY),,--config '{"bundle":{"createUpdaterArtifacts":false}}')

test: ui ## Tests: Rust{% if python_example %}, Python{% endif %}
	cargo test --workspace
{%- if python_example %}
	python3 python/test_worker.py
{%- endif %}

lint: ui ## Formatting, clippy{% if frontend == "react" %}, eslint{% endif %} and the pre-commit hooks
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
{%- if frontend == "react" %}
	npm --prefix ui-react run lint
{%- endif %}
	pre-commit run --all-files

# From Linux or macOS: cargo-xwin fetches the MSVC CRT and Windows SDK headers (C deps such
# as ring need them); nothing is linked. On Windows itself, plain `cargo clippy` does this.
WIN_TARGET := x86_64-pc-windows-msvc
check-windows: ui ## Type-check and clippy the Windows build from here (cargo-xwin, no linking)
	rustup target add $(WIN_TARGET)
	@cargo xwin --version >/dev/null 2>&1 || cargo install --locked cargo-xwin
	cargo xwin clippy --workspace --all-targets --target $(WIN_TARGET) -- -D warnings

fmt: ## Format everything
	cargo fmt --all
{%- if frontend == "react" %}
	npm --prefix ui-react run format
{%- endif %}

# Each tool runs when it is installed; a missing one is skipped with the command that installs it.
audit: ## Supply chain: cargo-deny (deny.toml), cargo-audit{% if frontend == "react" %}, npm audit{% endif %}{% if python_example %}, pip-audit{% endif %}
	@fail=0; \
	if cargo deny --version >/dev/null 2>&1; then echo "== cargo deny check"; cargo deny check || fail=1; \
	else echo "skip cargo-deny: cargo install --locked cargo-deny"; fi; \
	if cargo audit --version >/dev/null 2>&1; then echo "== cargo audit"; cargo audit || fail=1; \
	else echo "skip cargo-audit: cargo install --locked cargo-audit"; fi; \
{%- if frontend == "react" %}
	if command -v npm >/dev/null 2>&1; then echo "== npm audit (ui-react, shipped packages)"; npm --prefix ui-react audit --omit=dev || fail=1; \
	else echo "skip npm audit: install Node.js (https://nodejs.org)"; fi; \
{%- endif %}
{%- if python_example %}
	if command -v pip-audit >/dev/null 2>&1; then echo "== pip-audit (python/requirements.txt)"; pip-audit -r python/requirements.txt --progress-spinner off || fail=1; \
	else echo "skip pip-audit: pipx install pip-audit"; fi; \
{%- endif %}
	exit $$fail

FUZZ_SECONDS ?= 60
# Outside the workspace (fuzz/Cargo.toml): nightly only. One target per parser of outside input.
fuzz: ## Fuzz every parser for FUZZ_SECONDS each (needs nightly and cargo-fuzz)
	@cargo +nightly --version >/dev/null 2>&1 || { echo "needs nightly: rustup toolchain install nightly"; exit 1; }
	@cargo fuzz --version >/dev/null 2>&1 || { echo "needs cargo-fuzz: cargo install cargo-fuzz"; exit 1; }
	@for t in $$(cargo +nightly fuzz list); do \
		echo "== fuzz $$t ($(FUZZ_SECONDS) s)"; \
		cargo +nightly fuzz run $$t -- -max_total_time=$(FUZZ_SECONDS) -rss_limit_mb=4096 || exit 1; \
	done

self-test: ui ## What CI checks on every installer
	cargo run -p $(APP) -- --self-test

{% if evals -%}
eval: ui ## Search evals on evals/dataset.jsonl, compared with evals/baseline.json (fails on a regression)
	cargo run -p $(APP) -- --eval evals/dataset.jsonl --baseline evals/baseline.json

{% endif -%}
db-up: ## Postgres + pgvector in Docker (then: export APP_PG_URL=postgres://app:app@localhost:5433/app)
	docker compose up -d db

db-down: ## Stop it
	docker compose down

docker-test: ## Run the tests in the Linux build container
	docker compose run --rm build make test

clean: ## Remove build output
	cargo clean
	rm -rf ui-react/dist
