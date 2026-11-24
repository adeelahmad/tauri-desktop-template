// Entry point of the Settings window's page (settings.html, feature app-shell).
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import SettingsWindow from './SettingsWindow.jsx';
import './style.css';
import './shell.css';

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <SettingsWindow />
  </StrictMode>,
);
