---
name: kitchen-release
description: Package this app into installers and publish a release - local builds with `kitchen build`, versioning with release-please, and the GitHub Release workflow. Use when asked to build installers, cut or publish a release, bump the version, or set up auto-update signing.
---

# Package and publish

## Before anything

```sh
kitchen doctor                       # toolchain, Tauri CLI, WebView libraries
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p {{project-name}} -- --self-test
```

## Build installers on this machine

```sh
kitchen build                        # = cargo tauri build, from the app's root
```

The installers land in `target/release/bundle/` (.dmg/.app on macOS, .exe (NSIS) on
Windows, .AppImage/.deb/.rpm on Linux). A build only makes installers for the OS it runs on.
Builds are unsigned unless you add certificates; `docs/install-unsigned.md` tells users how
to open them.

## Version and changelog

Commits follow Conventional Commits (`feat:`, `fix:`, `feat!:`). On `main`,
release-please opens a release PR that bumps the version in `tauri.conf.json`,
`Cargo.toml` and `package.json` and writes `CHANGELOG.md`. Do not bump versions by hand.
{% if installers %}
## Publish (GitHub Release workflow)

1. Merge the release PR. That tags `v<version>`, creates the GitHub release, and runs
   `.github/workflows/release.yml`: it builds every platform, installs each build on a fresh
   runner, runs `--self-test`, and attaches the installers.
2. Or push a tag `v<version>` that matches `src-tauri/tauri.conf.json`.
3. Or run the **Release** workflow by hand for a pre-release `v<version>-test.<n>`.

The repository needs **Settings → Actions → General → Workflow permissions: Read and write**,
or the publish step gets a 403.
{% else %}
## Publish

This app was generated without the Release workflow. `kitchen enable installers --dry-run`
shows what turning it on adds (`.github/workflows/release.yml` builds, installs and
self-tests every platform). Otherwise build on each OS and upload the files from
`target/release/bundle/` to a GitHub release by hand (`gh release create v<version> <files>`).
{% endif %}{% if updater %}
## Auto-update signing

```sh
cargo tauri signer generate -w ~/.tauri/{{project-name}}.key
```

Put the **public** key in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. Add the
private key and its password as repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit the private key or paste it into a file
in the repo.
{% endif %}
## Never

- Never commit signing keys, certificates or tokens; they are CI secrets.
- Never publish a build whose self-test failed.
