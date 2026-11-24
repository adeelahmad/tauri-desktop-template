# Roadmap

Kitchen's goal: building a cross-platform desktop app should be as easy as building a web
app, for people and for AI agents. It should also be the shortest path from an existing
Electron or web app to a secure Rust app.

Shipped work is in the [README](README.md) and the app's `docs/GUIDE.md`. This file lists
what comes next and how it is meant to work. Nothing here is built yet.

## Next: `kitchen migrate`

This command ports an existing Electron or web app into a Kitchen app. Today an agent can do
it by hand with the `kitchen-port-app` skill. `kitchen migrate` turns that skill into a
command that does most of the porting itself and flags the rest for review.

```sh
kitchen migrate ../my-electron-app --into notes   # new Kitchen app, ported
kitchen migrate ../my-electron-app --plan          # only the report: what maps where
kitchen migrate ../my-electron-app --json          # same, machine-readable (MCP tool `migrate`)
```

### How it works

1. **Inventory (read-only).**
   - Detect the frontend framework and its build: Vite, webpack, CRA, Next export, Vue,
     Svelte or plain HTML.
   - Parse the Electron main and preload files with a JS parser (`oxc`, pure Rust) for:
     - `ipcMain.handle` and `ipcMain.on` channels;
     - `contextBridge.exposeInMainWorld` APIs;
     - Node built-ins used (`fs`, `child_process`, `net`, `shell`, `dialog`, `Notification`);
     - `BrowserWindow` options;
     - `loadURL` targets and `<webview>`s.
   - Look for secrets bundled into the frontend.
2. **Plan.** Map each finding to a row of the porting table in the skill. Each row is
   marked **auto** (Kitchen can do it), **assisted** (stub plus TODO) or **manual**
   (with a reason). The plan is the report that `--plan` prints.
3. **Apply** (only with `--into`).
   - Run `kitchen new` with the features the inventory implies. Examples: `dialog` turns on
     `tauri_dialog`, `Notification` turns on `tauri_notification`, and a remote site
     turns on `webview`.
   - Copy the frontend and wire `frontendDist`, `devUrl` and the before-commands.
   - Generate one Tauri command per IPC channel with `kitchen generate command`. Each gets
     a typed stub in `desktop-core` and the channel's original JS body as a comment.
   - Write a `window.api` shim that keeps the preload's function names on top of the
     typed `invoke` wrappers, so callers do not change.
   - Map events to `emit` and `listen`.
   - Move `fs` and `child_process` use into commands with fixed programs and scoped paths.
4. **Secure defaults stay on.**
   - The strict CSP stays on.
   - `freezePrototype` stays on.
   - Remote URLs are not given IPC; they go through the `webview` allowlist.
   - Any secret found in the bundle is reported and moved to an owner-only key file.
   - Each loosening the source app relied on is listed as a decision for the person,
     never applied silently.
5. **Verify.** Run `kitchen doctor` and `kitchen test --unit`, then build the UI. The
   report ends with the manual items and the commands to finish them.

### Done means

- Three sample apps port with no manual Rust left beyond the TODO bodies. The samples are
  a small Electron app with IPC, fs and a dialog; a Vite React app; and an app with a
  remote login page.
- The ported apps pass `kitchen test` on Linux, macOS and Windows.
- The planner and the IPC extraction have tests built from real Electron code.
- The `kitchen-port-app` skill points to `kitchen migrate` first and keeps the manual
  steps for the rest.

## Later (Adeel decides when)

- **Mobile:** iOS and Android from the same app. Scaffolding is merged; finishing it is
  deferred until the desktop work is done.
- **Web:** the same UI and backend served as a web app.
- **Peer-to-peer sync:** serverless sync and streaming between app instances. Not started,
  by request.
