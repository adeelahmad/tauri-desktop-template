import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// `cargo tauri android dev` / `ios dev` set this to this computer's address so a phone or
// emulator can reach the dev server; on desktop it is unset and the server stays local.
const host = process.env.TAURI_DEV_HOST;

// Tauri expects a fixed port in dev and static files in dist/ for builds.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 5174 } : undefined,{% if i18n %}
    // The translations live at the app's root (locales/*/app.ftl), shared with the Rust side.
    fs: { allow: ['.', '../locales'] },{% endif %}
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2021',{% if app_shell %}
    // Two pages: the main window and the Settings window (feature app-shell).
    rollupOptions: { input: { main: 'index.html', settings: 'settings.html' } },{% endif %}
  },
});
