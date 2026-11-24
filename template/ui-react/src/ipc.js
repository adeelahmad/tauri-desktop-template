// Typed IPC: the Tauri commands, typed from the Rust code (docs/GUIDE.md, "Typed IPC").
//
//   import { api } from './ipc.js';
//   const hits = await api.search({ query: 'heart' }); // Hit[]: the editor knows the fields
//
// The types are in ./bindings/, written by `cargo test -p <app> export_bindings` from the
// commands in src-tauri/src/lib.rs; nothing here changes when a command is added. Errors
// reject the promise with { code, message } (see errors.js).
import { invoke as tauriInvoke } from '@tauri-apps/api/core';

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
