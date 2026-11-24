// Entry point of the Settings window's page (settings.html, feature app-shell).
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import SettingsWindow from './SettingsWindow.jsx';
import './style.css';
import './shell.css';

// In a plain browser (`npm run dev`), src/dev-mock.js stands in for the Rust side. Outside
// development it is left out of the build (unless VITE_DEV_MOCK is set, for screenshots).
// `?pseudo=1` then shows every text accented and 30% longer, to check the layout (src/pseudo.js).
const mock =
  (import.meta.env.DEV || import.meta.env.VITE_DEV_MOCK) && !window.__TAURI_INTERNALS__
    ? import('./dev-mock.js').then(() => import('./pseudo.js')).then((p) => p.wanted() && p.startPseudoLocale())
    : Promise.resolve();

mock.then(() =>
  createRoot(document.getElementById('root')).render(
    <StrictMode>
      <SettingsWindow />
    </StrictMode>,
  ),
);
