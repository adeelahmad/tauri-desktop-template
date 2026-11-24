---
name: kitchen-port-app
description: Port an existing Electron app or web app into this Tauri app - move the frontend as it is, map Electron main/preload/IPC to Tauri commands and capabilities, and keep the strict CSP. Use when asked to port, migrate, wrap or bring an Electron, React, Vue, Svelte or plain web app into this app.
---

# Port an Electron or web app

The goal: the existing frontend runs unchanged where possible, its privileged parts become
Rust commands, and the security defaults stay on. Read AGENTS.md first.

## 1. Inventory the source app

- **Frontend**: framework, build command, output folder, entry `index.html`.
- **Electron main process**: every `ipcMain.handle` / `ipcMain.on` channel, and every Node
  API it uses (`fs`, `child_process`, `net`, `shell`, `dialog`, `Notification`, ...).
- **Preload**: everything put on `window` with `contextBridge.exposeInMainWorld`.
- **Remote content**: any `BrowserWindow.loadURL('https://...')`, `<webview>` or iframe.
- **Secrets**: API keys or tokens in the frontend bundle or config files.

Write the list down; each item maps to one row below.

## 2. Move the frontend as it is

- Copy the source into the app's frontend folder (`ui-react/src` for React; `ui/` for plain
  HTML/JS). For another framework, keep its own folder and point `src-tauri/tauri.conf.json`
  at it: `build.frontendDist` (its build output), `build.devUrl`, `beforeDevCommand`,
  `beforeBuildCommand`.
- Asset paths must be relative or root-relative to the bundle; there is no `file://`.
- Remove Node built-ins from frontend code (`require('fs')` and friends). They move to Rust.

## 3. Map main, preload and IPC to Tauri

| Electron | Here |
|---|---|
| `ipcMain.handle('x', fn)` + `ipcRenderer.invoke('x', args)` | `kitchen generate command x`; logic in a `desktop-core` module; UI calls `invoke('x', { input: args })` |
| `ipcMain.on` + `webContents.send('evt', data)` | Rust `app.emit("evt", data)`; UI `listen('evt', cb)` from `@tauri-apps/api/event` |
| `contextBridge.exposeInMainWorld('api', {...})` | a small JS module that wraps `invoke` with the same function names, so callers do not change |
| `fs`, `path` in main | Rust `std::fs` in a command, under the app's data folders; or the Tauri fs plugin with a narrow scope |
| `child_process` | a Rust command using `std::process::Command` with fixed program and arguments, never a shell string from the UI |
| `net`, `fetch` from main | Rust through `desktop_core::net` (one policy for every request) |
| `dialog`, `clipboard`, `Notification`, `shell.openExternal`, `globalShortcut`, auto-launch, `electron-store` | the official Tauri plugins: `kitchen enable tauri_dialog` (likewise `tauri_clipboard`, `tauri_notification`, `tauri_opener`, `tauri_global_shortcut`, `tauri_autostart`, `tauri_store`); each also turns on `tauri_plugins` |
| `autoUpdater` | `kitchen enable updater` |
| A second instance focusing the first | `kitchen enable single_instance` |

Imports: React uses `import { invoke } from '@tauri-apps/api/core'`; the plain frontend
uses `window.__TAURI__.core.invoke`.

## 4. Capabilities

Every Tauri permission the window has is in `src-tauri/capabilities/*.json`. App commands
need nothing extra; plugin APIs need their permission. Add the narrowest permission
(`clipboard-manager:allow-write-text`, not everything) and only for the `main` window.
Never add a `remote` URL to a capability.

## 5. Content security policy

Keep `app.security.csp` in `src-tauri/tauri.conf.json` strict. Never set it to `null`, never
add `'unsafe-eval'`, never allow remote `script-src`.

- Inline `<script>` blocks: move them into files.
- Remote images or fonts the app needs: bundle them, or add that one origin to `img-src` /
  `font-src` only.
- API calls from the frontend to remote servers: move them to Rust commands (step 3), so
  keys stay out of the bundle and `connect-src` stays local. Add an origin to
  `connect-src` only when the user accepts that, and never with credentials in the bundle.
- A page that must show remote content opens in the system browser, not in the app's window
  (`src-tauri/src/guard.rs` refuses navigation elsewhere).

## 6. Secrets and settings

Settings become fields in `desktop-core` config (`kitchen generate module <name> --config`,
then `kitchen generate setting ...`), off by default. Secrets go in owner-only files like the
existing key files, never in `settings.json`.

## 7. Check

```sh
kitchen doctor
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p desktop-core
kitchen dev            # run it; watch the console for CSP violations
```

Walk through every item in the step 1 inventory and confirm it works, or say what is left.
