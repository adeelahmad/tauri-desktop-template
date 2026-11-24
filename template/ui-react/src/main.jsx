import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App.jsx';
import './style.css';
{% if app_shell %}import './shell.css';
{% endif %}
// In a plain browser (`npm run dev`), src/dev-mock.js stands in for the Rust side. Outside
// development it is left out of the build (unless VITE_DEV_MOCK is set, for screenshots).
const mock =
  (import.meta.env.DEV || import.meta.env.VITE_DEV_MOCK) && !window.__TAURI_INTERNALS__ ? import('./dev-mock.js') : Promise.resolve();

mock.then(() =>
  createRoot(document.getElementById('root')).render(
    <StrictMode>
      <App />
    </StrictMode>,
  ),
);
