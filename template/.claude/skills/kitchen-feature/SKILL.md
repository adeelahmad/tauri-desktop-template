---
name: kitchen-feature
description: Turn a Kitchen feature (email, ai, tts, postgres, mcp_server, ...) on or off in this app safely, with a dry run first. Use when asked to add, enable, remove or disable a built-in feature, or to change a generation answer such as the frontend.
---

# Enable or disable a feature

Features are the template's yes/no answers, recorded in `kitchen.toml`. `kitchen` renders
the template with the old and new answers and applies the difference as a three-way merge,
so the app's own edits are kept. Never edit `kitchen.toml` by hand.

1. **Start clean.** `git status` should show nothing to commit, so the change is easy to
   review and undo. Commit or stash first.
2. **Find the name.** `kitchen features` lists every answer, on/off, and what each one
   needs (for example `bm25` needs `postgres`). Use the answer name, not the cargo feature
   (`email`, not `mail`).
3. **Dry run.** Change nothing yet and read the plan:
   ```sh
   kitchen enable <feature> --dry-run
   kitchen disable <feature> --dry-run
   ```
   Enabling also turns on what it needs; disabling also turns off what needs it. Show the
   list of files to the user if it is long or touches files they edited.
4. **Apply.** The same command without `--dry-run`. For a text answer use
   `kitchen set <name>=<value>` (e.g. `kitchen set frontend=vanilla`), with the same dry run first.
5. **Resolve conflicts.** Files reported as `conflict` have `<<<<<<<` markers or a
   `<file>.kitchen-new` copy next to them. Merge by hand, then delete the `.kitchen-new` file.
   Files the user edited and the template removed are kept and listed: decide with the user.
6. **Check.**
   ```sh
   kitchen doctor
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test -p desktop-core --features <cargo feature>
   ```
   The cargo feature names are in `src-tauri/Cargo.toml` (`email` → `mail`, `mcp_server` → `mcp`).
7. **Tell the user** that the feature is compiled in but its switch in Settings starts off,
   and where to turn it on. AGENTS.md now lists it.

Add `--json` to any command for machine-readable output. With the `kitchen` MCP server
(`.mcp.json`), the same steps are the `features`, `enable`, `disable` and `set` tools, each
with `dry_run`.
