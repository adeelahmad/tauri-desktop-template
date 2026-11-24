# Kitchen CLI

`kitchen` creates cross-platform desktop apps from the Kitchen template. After that it can:

- turn features on and off later, without losing your edits;
- generate code in the app's own style;
- check your machine for what a build needs;
- offer all of the above to AI agents as a local MCP server.

It is a single Rust binary.

```sh
cargo install --git https://github.com/adeelahmad/tauri-desktop-template desktop-kitchen   # installs `kitchen`
cargo install cargo-generate                                                               # the template engine it drives
```

## Commands

```sh
kitchen new notes --enable email,ai --disable postgres   # create ./notes
cd notes
kitchen features                  # every feature, on/off, and what it needs
kitchen enable tts                # turn on later: a 3-way merge keeps your edits
kitchen disable tts --dry-run     # see what would change first
kitchen set frontend=vanilla      # change any other answer
kitchen upgrade                   # move to the newest template, keeping your edits

kitchen generate module invoices --config                     # desktop-core module + settings section
kitchen generate setting invoices.due_days int 30 --label "Days until due"
kitchen generate command export_csv                           # Tauri command + backend method
kitchen generate page Reports                                 # React page on the main screen
kitchen generate job send_digest                              # background job type (needs background_jobs)
kitchen generate menu export_csv --label "Export CSV" --shortcut CmdOrCtrl+Shift+E   # app action in the app menu (tray_menu)
kitchen generate shortcut search CmdOrCtrl+Alt+F              # default global shortcut for an action (tray_menu)
kitchen generate route export_csv                             # POST /v1/export_csv on the HTTP API (http_api)

kitchen doctor                    # Rust, cargo-generate, Tauri CLI, Node, WebKitGTK/Xcode, tauri-driver, the app's markers and AGENTS.md

kitchen start                     # cargo tauri dev, KITCHEN_ENV=development: Vite HMR + Rust rebuilds (`dev` works too)
kitchen start --install           # first installs a missing Tauri CLI / npm packages
kitchen test                      # unit tests, then the real app over WebDriver (KITCHEN_ENV=test)
kitchen test --unit               # cargo test --workspace (nextest + doc tests if nextest is installed)
kitchen test --feature            # e2e/: tauri-driver on Linux/Windows; --self-test on macOS
kitchen package --release         # this OS's installers (dmg/app, nsis/msi, appimage/deb) into ./packages
kitchen publish                   # the `gh release create` it would run; --yes runs it
kitchen eject vite|tauri|cargo    # where that tool's own config lives, and what Kitchen adds
kitchen build                     # cargo tauri build, as is
kitchen mcp                       # MCP server over stdio
```

## The lifecycle

One path from a new app to a release, like `rails new` → `rails s` → `rails test` → deploy:

| Command | Runs | Environment |
|---|---|---|
| `kitchen start` | `cargo tauri dev` | `KITCHEN_ENV=development` |
| `kitchen test --unit` | `cargo test --workspace`, or `cargo nextest run --workspace` + `cargo test --doc` | `KITCHEN_ENV=test` |
| `kitchen test --feature` | `cargo tauri build --debug --no-bundle`, then `cargo test -p e2e -- --ignored` (under `xvfb-run` without a display) | `KITCHEN_ENV=test` |
| `kitchen package [os] [--release]` | `cargo tauri build --bundles <this OS's formats> [--debug]`, then copies to `./packages` | `production` with `--release`, else `development` |
| `kitchen publish --yes` | `gh release create v<version> packages/* --generate-notes` | |

Each command prints the step before it runs it, so nothing is hidden. Packaging for another
OS is refused with the reason: Tauri builds installers for the OS it runs on. `KITCHEN_ENV`
picks the layer of the app's build-time config (`config/environments/<env>.json`; see the
app's `docs/GUIDE.md`).

## Kitchen for web developers

| On the web | In a Kitchen app |
|---|---|
| `fetch('/api/...')` | `invoke('command', { ... })`: your API call |
| routes / controllers | Tauri commands (`kitchen generate command <name>`) |
| your backend service | `desktop_core`, a plain Rust library |
| `.env`, `config/*.json` | `config/` per environment and OS (build time) + the user's settings (run time) |
| Postgres, embeddings, email, jobs | one `kitchen enable <feature>` each, nothing to host |
| `npm run dev` / `test` / deploy | `kitchen start` / `test` / `package` + `publish` |

Five minutes from nothing to an installer:

```sh
cargo install --git https://github.com/adeelahmad/tauri-desktop-template desktop-kitchen
kitchen new notes && cd notes
kitchen doctor
kitchen start --install           # edit ui-react/src/App.jsx: the window updates in place
kitchen generate command word_count
kitchen test
kitchen package --release
```

The app's `docs/GUIDE.md` has the longer version, and a walkthrough for porting an
Electron or web app.

Add `--json` to any command for machine-readable output. `kitchen g` is short for `kitchen generate`.

## How `enable` keeps your changes

`kitchen.toml` records the answers the app was generated with and the template commit
it came from. When you change an answer, Kitchen renders the template twice: once with the
old answers (*base*) and once with the new ones (*next*). It then applies the difference to
your app as a three-way merge:

| base → next | your file       | result |
|-------------|-----------------|--------|
| added       | missing         | created |
| added       | exists          | conflict: the template's version is saved as `<file>.kitchen-new` |
| changed     | untouched       | updated |
| changed     | edited          | merged with `git merge-file`; clashing lines get `<<<<<<<` markers |
| removed     | untouched       | deleted |
| removed     | edited          | kept, and listed |

Files the template doesn't touch are never read or written.

## Generators

Each generator writes a new file that already has docs and a test. It wires that file in at
the `// kitchen:…` marker comments in the app (`kitchen doctor` checks they are there).

To change what a generator writes, put your own copy of the file template in
`.kitchen/templates/`. The files are `module.rs`, `module_config.rs`, `command.rs`,
`page.jsx` and `job.rs`; copy the originals from [`templates/`](templates).

`menu` and `shortcut` need the `tray_menu` feature. `menu` registers the action and its app
menu item at `// kitchen:actions` in `src-tauri/src/tray.rs` (the `--shortcut` keys become
its accelerator) and adds a backend arm at `// kitchen:action-methods` in
`src-tauri/src/service.rs`; `shortcut` adds a default global shortcut at `// kitchen:actions`.
Users can rebind or clear it in Settings → Tray & shortcuts. They use the variables
`{{name}}`, `{{Name}}`, `{{camelName}}` and `{{Title}}`.

### Background jobs

`kitchen generate job <name>` needs the `background_jobs` feature (`kitchen enable
background_jobs`). It writes `desktop-core/src/job_types/<name>.rs`: a job type with a typed
payload, a doc example that runs it through a queue, and a test. It registers the type at the
`// kitchen:job-mods` and `// kitchen:jobs` markers in `desktop-core/src/job_types/mod.rs`.
After that the job can be enqueued from the UI (`invoke('jobs_enqueue', { kind: '<name>', payload })`),
from Rust (`queue.enqueue_job::<Name>(&payload, Enqueue::default())`), or put on a cron or
interval schedule in Settings → Jobs. Failed attempts retry with exponential backoff and
jitter, then wait in the dead letters until retried.

## MCP

```sh
claude mcp add kitchen -- kitchen mcp
```

The `generate` tool takes the kinds `module`, `setting`, `command`, `page` and `job`. The tools are `features`, `enable`, `disable`, `set`, `upgrade`, `generate`, `doctor`,
`start`, `test`, `package`, `publish`, `eject` and `new`. Each one takes an optional
`project` folder, and the ones that change files also take `dry_run`. `start` keeps
running, so it answers with the command to run in a terminal instead of blocking; `test`
and `package` run and return the report (the commands, the test results, and the last
lines of output of a step that failed); `publish` only shows the release unless `yes` is
true.

## For AI agents

Every app comes with `AGENTS.md` (layout, golden rules, the features that are on, tests,
markers, and what never to do), `CLAUDE.md` pointing to it, `TESTING.md`, `BRAND.md`,
skills in `.claude/skills/` (`kitchen-feature`, `kitchen-generate`, `kitchen-port-app`,
`kitchen-release`) and a `.mcp.json` that registers `kitchen mcp` (and the app's own MCP
server when `mcp_server` is on). `kitchen doctor` warns when `AGENTS.md` is missing.

## Why cargo-generate and not Yeoman

The template engine is [cargo-generate](https://github.com/cargo-generate/cargo-generate),
which uses Liquid. It does Yeoman's job, and it is native to Rust, so Kitchen needs neither
Node nor a second template format. Feature switching reuses the same renders, with
`git merge-file` doing the merge.

## As a library

Every command is a function that returns a `Report`. The CLI, the MCP server and your own
tools all share them. Run `cargo doc --open` for the API, which includes examples.
