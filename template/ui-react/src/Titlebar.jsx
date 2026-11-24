// The custom titlebar (feature app-shell): a drag region, the title, Settings and, on
// Windows and Linux, the window buttons. macOS keeps its traffic lights over the content.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openSettings, os } from './shell.js';

/** The custom titlebar: drag region, title, Settings button and, on Windows and Linux, window buttons. */
export default function Titlebar({ title }) {
  const [maximized, setMaximized] = useState(false);
  const mac = os() === 'macos';
  useEffect(() => {
    document.documentElement.classList.add('has-titlebar');
    if (mac) return () => document.documentElement.classList.remove('has-titlebar');
    const w = getCurrentWindow();
    const sync = () =>
      w
        .isMaximized()
        .then(setMaximized)
        .catch(() => {});
    sync();
    const off = w.onResized(sync);
    return () => {
      document.documentElement.classList.remove('has-titlebar');
      off.then((f) => f());
    };
  }, [mac]);
  const w = getCurrentWindow();
  return (
    <div className="titlebar" data-tauri-drag-region>
      <span className="titlebar-title" data-tauri-drag-region>
        {title}
      </span>
      <button title={mac ? 'Settings (⌘,)' : 'Settings (Ctrl+,)'} aria-label="Settings" onClick={openSettings}>
        ⚙
      </button>
      {!mac && (
        <>
          <button title="Minimize" aria-label="Minimize" onClick={() => w.minimize()}>
            ─
          </button>
          <button
            title={maximized ? 'Restore' : 'Maximize'}
            aria-label={maximized ? 'Restore' : 'Maximize'}
            onClick={() => w.toggleMaximize()}
          >
            {maximized ? '❐' : '☐'}
          </button>
          <button className="close" title="Close" aria-label="Close" onClick={() => w.close()}>
            ✕
          </button>
        </>
      )}
    </div>
  );
}
