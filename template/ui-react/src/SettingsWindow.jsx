// The Settings window (feature app-shell): src-tauri/src/shell.rs opens it with Cmd/Ctrl+,,
// the titlebar's gear button or the app menu. Its capability allows events only.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useState } from 'react';
import { api } from './ipc.js';
import Settings from './Settings.jsx';
import { useAppearance } from './shell.js';
import { setLocale, useLocale } from './i18n.js';

export default function SettingsWindow() {
  const [features, setFeatures] = useState(null);
  const [error, setError] = useState('');
  useAppearance();
  useLocale();
  useEffect(() => {
    api
      .app_info()
      .then(async (i) => {
        await setLocale(i.settings.config?.ui?.language || 'system');
        setFeatures(i.features);
      })
      .catch((e) => setError(String(e)));
  }, []);
  if (error)
    return (
      <p className="status error" role="alert">
        {error}
      </p>
    );
  return features && <Settings features={features} />;
}
