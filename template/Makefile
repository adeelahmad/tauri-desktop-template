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
.PHONY: help setup dev build test lint fmt self-test ui db-up db-down docker-test hooks clean

help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

setup: hooks ## Install the Tauri CLI and git hooks
	cargo install tauri-cli --version '^2' --locked

hooks: ## Install pre-commit (and commit-msg) hooks
	pre-commit install --hook-type pre-commit --hook-type commit-msg

dev: ## Run the app with hot reload
	cargo tauri dev

build: ui ## Build installers for this machine
	cargo tauri build

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

fmt: ## Format everything
	cargo fmt --all
{%- if frontend == "react" %}
	npm --prefix ui-react run format
{%- endif %}

self-test: ui ## What CI checks on every installer
	cargo run -p $(APP) -- --self-test

db-up: ## Postgres + pgvector in Docker (then: export APP_PG_URL=postgres://app:app@localhost:5433/app)
	docker compose up -d db

db-down: ## Stop it
	docker compose down

docker-test: ## Run the tests in the Linux build container
	docker compose run --rm build make test

clean: ## Remove build output
	cargo clean
	rm -rf ui-react/dist
