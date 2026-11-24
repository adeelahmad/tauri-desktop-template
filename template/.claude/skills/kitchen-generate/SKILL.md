---
name: kitchen-generate
description: Add a new backend module, a setting, a Tauri command or a React page to this app with `kitchen generate`, wired in at the `// kitchen:` markers. Use when asked to add new functionality, a new command or API the UI calls, a new setting, or a new screen.
---

# Generate a command, module, setting or page

Each generator writes a documented, tested file and wires it in at the `// kitchen:` marker
comments. Run `kitchen doctor` first: it reports missing markers. Add `--dry-run` to see the
files a generator would write, and `--json` for machine-readable output.

| To add | Command | Writes and wires |
|---|---|---|
| Engine logic (plain Rust, testable) | `kitchen generate module <name>` | `desktop-core/src/<name>.rs`, `kitchen:modules` |
| ...with a settings section | `kitchen generate module <name> --config` | also `kitchen:config`, `kitchen:validate`, and the Settings screen with React |
| A setting in that section | `kitchen generate setting <module>.<key> <bool\|int\|float\|text\|list> [default] --label "<text>"` | `kitchen:fields`, `kitchen:defaults`, `kitchen:settings` |
| Something the UI calls | `kitchen generate command <name>` | `src-tauri/src/<name>.rs`, `kitchen:mods`, `kitchen:handlers`, `kitchen:methods` |
| A screen (React only) | `kitchen generate page <Name>` | `ui-react/src/<Name>.jsx`, `kitchen:imports`, `kitchen:pages` |

Names are snake_case (`export_csv`, `invoices`); pages are PascalCase (`Reports`).
`kitchen g` is short for `kitchen generate`.

## Typical flow

```sh
kitchen generate module invoices --config
kitchen generate setting invoices.due_days int 30 --label "Days until due"
kitchen generate command export_invoices
kitchen generate page Invoices
```

Then:

1. Put the logic in the module (`desktop-core`), not in the command. The command file's
   `handle`/`run` functions call into it; the `#[tauri::command]` only forwards the call.
2. Call it from the UI: `invoke('export_invoices', { input: { ... } })`.
3. New optional behaviour stays off by default: a `bool` setting that defaults to `false`,
   checked before the work runs. A new cargo feature follows the wiring checklist in
   `docs/GUIDE.md`.
4. Keep the generated `//!` example and test, and extend them to cover the real logic.
5. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test -p desktop-core`.

Never remove a marker; if one is missing, `kitchen upgrade` or the template restores it.
To change what a generator writes, put your own copy of `module.rs`, `module_config.rs`,
`command.rs` or `page.jsx` in `.kitchen/templates/`.
