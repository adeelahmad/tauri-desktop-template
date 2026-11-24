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

kitchen doctor                    # Rust, cargo-generate, Tauri CLI, Node, WebKitGTK/Xcode, the app's markers and AGENTS.md
kitchen service install           # the app's daemon as a user service (template feature `daemon`)
kitchen service status            # installed? running? pid, log file
kitchen service logs --lines 100 --follow
kitchen service stop | start | restart | uninstall [--system]
kitchen dev | kitchen build       # cargo tauri dev / build
kitchen mcp                       # MCP server over stdio
```

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
`service` and `new`. Each one takes an optional `project` folder, and the ones that change files also take
`dry_run`. `service` only runs `status` and `logs` by itself; for `install`, `uninstall`,
`start`, `stop` and `restart` it reports the command it would run unless `allow_changes` is
true.

## Services

`kitchen service <action> [--system]` runs `<app> --service <action>` on the app's built
binary (`target/debug/<name>` or `target/release/<name>`, the newer one; `CARGO_TARGET_DIR`
is honoured), and builds it with `cargo build -p <name>` first when there is none. The app
needs the `daemon` answer (`kitchen enable daemon`, which needs `ipc_worker`). The user
level is a LaunchAgent, a `systemctl --user` unit or a Windows Run entry; `--system` is a
LaunchDaemon, a system unit or a Windows Service and needs administrator rights (the app
prints the `sudo` or Administrator command instead of asking). `--dry-run` shows the command
without running it.

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
