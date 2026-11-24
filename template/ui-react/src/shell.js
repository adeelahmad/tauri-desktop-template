// App shell (feature app-shell): live theme changes and the Settings window. The theme is
// already on <html> before this runs (a script from src-tauri/src/shell.rs); Titlebar.jsx
// draws the custom titlebar.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect } from 'react';
import { api } from './ipc.js';
import { listen } from '@tauri-apps/api/event';

export const os = () => document.documentElement.dataset.os || '';

/** Whether this build has the app shell: the window script ran. */
export const hasShell = () => typeof window !== 'undefined' && !!window.__appearance;

/** Whether this window draws its own titlebar (desktop main window only). */
export const drawsTitlebar = () => hasShell() && !['ios', 'android'].includes(os());

export const openSettings = () => api.open_settings().catch(() => {});

/** Apply saved appearance changes (from any window) as they happen. */
export function useAppearance() {
  useEffect(() => {
    if (!hasShell()) return;
    const off = listen('appearance', (e) => window.__appearance.apply(e.payload));
    api.get_appearance()
      .then((a) => window.__appearance.apply(a))
      .catch(() => {});
    return () => {
      off.then((f) => f());
    };
  }, []);
}

/** Cmd+, (macOS) or Ctrl+, opens Settings; the tray/menu "settings" action does too. */
export function useSettingsShortcut() {
  useEffect(() => {
    if (!drawsTitlebar()) return;
    const onKey = (e) => {
      if (e.key === ',' && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
        e.preventDefault();
        openSettings();
      }
    };
    window.addEventListener('keydown', onKey);
    const off = listen('app-action', (e) => e.payload?.action === 'settings' && openSettings());
    return () => {
      window.removeEventListener('keydown', onKey);
      off.then((f) => f());
    };
  }, []);
}
