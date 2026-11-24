// App shell (feature app-shell): custom titlebar, live theme changes, and the Settings window.
// The theme is already on <html> before this runs (a script from src-tauri/src/shell.rs).
(() => {
  if (!window.__appearance) return;
  /** @type {import('./bindings/commands').Invoke} (typed IPC, see app.js) */
  const invoke = window.__TAURI__.core.invoke;
  const { listen } = window.__TAURI__.event;
  const root = document.documentElement;
  const os = root.dataset.os || '';
  const openSettings = () => invoke('open_settings').catch(() => {});

  // Saved appearance changes (from any window) apply at once.
  listen('appearance', (e) => window.__appearance.apply(e.payload));
  invoke('get_appearance').then((a) => window.__appearance.apply(a)).catch(() => {});
  const win = window.__TAURI__.window.getCurrentWindow();
  // The rest is for the main desktop window (the Settings window keeps native decorations).
  if (os === 'ios' || os === 'android' || win.label !== 'main') return;

  // Settings lives in its own window: Cmd+, / Ctrl+, the gear button, the menu, or the tray.
  const inline = document.getElementById('settings-section');
  if (inline) inline.hidden = true;
  window.addEventListener('keydown', (e) => {
    if (e.key === ',' && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      openSettings();
    }
  });
  listen('app-action', (e) => { if (e.payload && e.payload.action === 'settings') openSettings(); });

  // The titlebar: a drag region, the title, Settings, and window buttons on Windows and Linux
  // (macOS keeps its traffic lights, drawn over the content).
  const mac = os === 'macos';
  const bar = document.createElement('div');
  bar.className = 'titlebar';
  bar.setAttribute('data-tauri-drag-region', '');
  const title = document.createElement('span');
  title.className = 'titlebar-title';
  title.setAttribute('data-tauri-drag-region', '');
  title.textContent = document.title;
  bar.append(title);
  const button = (label, text, onClick, cls) => {
    const b = document.createElement('button');
    b.title = label;
    b.setAttribute('aria-label', label);
    b.textContent = text;
    if (cls) b.className = cls;
    b.onclick = onClick;
    bar.append(b);
    return b;
  };
  button(mac ? 'Settings (⌘,)' : 'Settings (Ctrl+,)', '⚙', openSettings);
  if (!mac) {
    button('Minimize', '─', () => win.minimize());
    const max = button('Maximize', '☐', () => win.toggleMaximize());
    const sync = () => win.isMaximized().then((m) => {
      max.textContent = m ? '❐' : '☐';
      max.title = m ? 'Restore' : 'Maximize';
    }).catch(() => {});
    sync();
    win.onResized(sync);
    button('Close', '✕', () => win.close(), 'close');
  }
  document.body.prepend(bar);
  root.classList.add('has-titlebar');
})();
