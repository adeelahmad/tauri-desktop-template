# Contributing to {{app_name}}

Thanks for helping. This file is the short version of how work flows here.

## Set up

```sh
git clone https://github.com/{{github_repo}} && cd {{project-name}}
make setup      # Tauri CLI + git hooks (needs `pipx install pre-commit` or `brew install pre-commit`)
make dev        # run the app with hot reload
```

Linux also needs `libwebkit2gtk-4.1-dev librsvg2-dev` (or use `make docker-test`).

## Making a change

1. Branch from `main`: `feat/short-name`, `fix/short-name`.
2. Keep `make test` and `make lint` green; add a test with every fix.
3. Write commits as [Conventional Commits](https://www.conventionalcommits.org):
   - `feat: add PDF drag and drop` → minor version
   - `fix: keep page numbers on empty slides` → patch version
   - `feat!: change the store format` or a `BREAKING CHANGE:` footer → major version
   - `docs:`, `chore:`, `refactor:`, `test:`, `ci:`, `build:`, `perf:` → no release on their own
   The commit-msg hook checks this.
4. Open a PR using the template. PRs are squash-merged, so the **PR title** is the commit
   that lands on `main` and the line in `CHANGELOG.md`.

## Releases

Nobody bumps versions by hand. release-please keeps a release PR open that collects
the changes since the last version, bumps it (semver) in `tauri.conf.json`, `Cargo.toml`
and the UI's `package.json`, and updates `CHANGELOG.md`. Merging that PR tags
`v<version>`, publishes the GitHub release and builds the installers into it.

## Code style

- Rust: `cargo fmt`, clippy with `-D warnings`.
- Frontend: ESLint + Prettier (`npm --prefix ui-react run format`).
- Python: ruff.
- Shell: shellcheck. Workflows: actionlint.

## Reporting bugs and security issues

Use the issue templates. For anything security-related see [SECURITY.md](SECURITY.md).
By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
