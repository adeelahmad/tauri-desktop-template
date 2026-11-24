// Read text aloud with the built-in voice (feature `tts`).
// (No double curly braces in this folder: the template engine would eat them.)
import { useRef, useState } from 'react';
import { api } from './ipc.js';
import { t } from './i18n.js';

export default function Speak({ text }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const audio = useRef(null);
  const play = async () => {
    if (audio.current) audio.current.pause();
    setBusy(true);
    setError('');
    try {
      const r = await api.speak({ text });
      audio.current = new Audio(`data:audio/wav;base64,${r.wav}`);
      await audio.current.play();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <span>
      <button className="link" onClick={play} disabled={busy || !text}>
        {busy ? t('speak-preparing', 'Preparing…') : t('speak', 'Read aloud')}
      </button>
      {error && (
        <span className="status error" role="alert">
          {' '}
          {error}
        </span>
      )}
    </span>
  );
}
