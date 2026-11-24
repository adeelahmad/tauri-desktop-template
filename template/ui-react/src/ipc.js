// Typed IPC: the Tauri commands, typed from the Rust code (docs/GUIDE.md, "Typed IPC").
//
//   import { api } from './ipc.js';
//   const hits = await api.search({ query: 'heart' }); // Hit[]: the editor knows the fields
//
// The types are in ./bindings/, written by `cargo test -p <app> export_bindings` from the
// commands in src-tauri/src/lib.rs; nothing here changes when a command is added.
//
// A failed command rejects the promise with a CommandError, { code, message, details? }
// (src-tauri/src/error.rs; the codes are in docs/GUIDE.md, "Command errors"). Show it with
// errorText(e), which reads e.message and falls back to String(e):
//
//   try { await api.ingest_file({ path }); } catch (e) { setMsg(errorText(e)); }
import { invoke as tauriInvoke } from '@tauri-apps/api/core';

export { errorCode, errorText } from './errors.js';

/** @typedef {import('./bindings/types').CommandError} CommandError */

/**
 * `invoke`, typed by command name: for a command chosen at run time.
 * @type {import('./bindings/commands').Invoke}
 */
export const invoke = (cmd, ...args) => tauriInvoke(cmd, args[0]);

/**
 * One typed function per command: `api.search({ query })` is `invoke('search', { query })`.
 * @type {import('./bindings/commands').Api}
 */
export const api = new Proxy(/** @type {any} */ ({}), {
  get: (_, cmd) => (typeof cmd === 'string' ? (/** @type {any} */ args) => tauriInvoke(cmd, args) : undefined),
});
