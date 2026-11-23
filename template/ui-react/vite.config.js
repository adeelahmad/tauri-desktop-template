import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri expects a fixed port in dev and static files in dist/ for builds.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2021' },
});
