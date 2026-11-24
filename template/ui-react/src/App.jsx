// React frontend talking to the Rust commands in src-tauri/src/lib.rs.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useRef, useState } from 'react';
import { api, errorText } from './ipc.js';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { APP_NAME, DESCRIPTION, configGet, setConfig } from './config.js';
import Settings from './Settings.jsx';
import Ask from './Ask.jsx';
import Dictate from './Dictate.jsx';
import { provenance } from './provenance.js';
import { setLocale, t, useLocale } from './i18n.js';
{% if app_shell %}import Titlebar from './Titlebar.jsx';
import { drawsTitlebar, useAppearance, useSettingsShortcut } from './shell.js';
{% endif %}// kitchen:imports (`kitchen generate page` adds imports above this line)

// The start of the error that asks before the database is upgraded (desktop_core::pgupgrade).
const UPGRADE_NEEDED = 'pg-upgrade-needed';

function describe(info, status) {
  if (!info) return '';
  const s = status || info.status;
  const base = `${info.name} ${info.version}`;
  if (!s) return base;
  return t('footer-status', '{ $app } · { $chunks } chunks · { $backend } store · { $embedder }', {
    app: base,
    chunks: s.chunks,
    backend: s.backend,
    embedder: s.embedder || t('footer-keyword-search', 'keyword search'),
  });
}

// A live region: screen readers read each new message (errors at once, the rest politely).
function Status({ text, error }) {
  return (
    <p className={error ? 'status error' : 'status'} role={error ? 'alert' : 'status'}>
      {text}
    </p>
  );
}

function Wizard({ info, onSetup, busy, message }) {
  const f = info.features;
  const [backend, setBackend] = useState('file');
  const [embed, setEmbed] = useState(true);
  // A model built into the installer (bundled models) needs no download step.
  const embedBundled = (info.models || []).some((m) => m.kind === 'embed' && m.present);
  return (
    <section>
      <h2>{t('wizard-title', 'Welcome')}</h2>
      <p className="hint">
        {f.mobile
          ? t('wizard-intro-device', 'A one-time setup. Everything stays on this device.')
          : t('wizard-intro', 'A one-time setup. Everything stays on this computer.')}
      </p>
      <ol className="steps">
        <li>
          <h3>{t('wizard-store', '1. Where to keep your data')}</h3>
          <label>
            <input type="radio" checked={backend === 'file'} onChange={() => setBackend('file')} />{' '}
            {t('wizard-store-file', 'A simple file (fast, nothing to download)')}
          </label>
          {f.postgres && (
            <label>
              <input
                type="radio"
                disabled={!info.postgresAvailable}
                checked={backend === 'postgres'}
                onChange={() => setBackend('postgres')}
              />{' '}
              {t('wizard-store-postgres', 'A private Postgres database with vector search (downloads about 30 MB the first time)')}
            </label>
          )}
          {f.postgres && !info.postgresAvailable && (
            <p className="hint">
              {t('wizard-no-postgres', 'This processor cannot run the bundled database; the file store works the same.')}
            </p>
          )}
        </li>
        {f.embed && (
          <li>
            <h3>{t('wizard-model', '2. Smart search')}</h3>
            <label>
              <input type="checkbox" checked={embed} onChange={(e) => setEmbed(e.target.checked)} />{' '}
              {embedBundled
                ? t(
                    'wizard-model-embed-bundled',
                    'Search by meaning with a local AI model (built into the app, no download; nothing is sent anywhere)',
                  )
                : t('wizard-model-embed', 'Search by meaning with a local AI model (downloads once, nothing is sent anywhere)')}
            </label>
          </li>
        )}
        <li>
          <h3>{t('wizard-go-title', '3. Set up')}</h3>
          <button disabled={busy} onClick={() => onSetup(backend, embed)}>
            {t('wizard-go', 'Set up')}
          </button>
          {busy && <progress />}
          <Status text={message.text} error={message.error} />
        </li>
      </ol>
    </section>
  );
}

function UpdateBanner() {
  const [update, setUpdate] = useState(null);
  const [text, setText] = useState('');
  useEffect(() => {
    api
      .check_update()
      .then((u) => u.available && setUpdate(u))
      .catch(() => {});
  }, []);
  if (!update) return null;
  const install = async () => {
    const off = await listen('update-progress', (e) => {
      const [done, total] = e.payload;
      setText(
        total
          ? t('update-downloading-percent', 'Downloading… { $percent }%', { percent: Math.round((100 * done) / total) })
          : t('update-downloading', 'Downloading…'),
      );
    });
    try {
      await api.install_update();
    } catch (e) {
      setText(t('update-failed', 'Update failed: { $error }', { error: errorText(e) }));
      off();
    }
  };
  return (
    <div className="banner">
      <span>
        {text ||
          t('update-available', 'Version { $version } is available (you have { $current }).', {
            version: update.version,
            current: update.current,
          })}
      </span>
      <button onClick={install}>{t('update-install', 'Install and restart')}</button>
    </div>
  );
}

// On iOS and Android (features.mobile) there is no dropping files on a window and no folder
// of the user's to read, so only a file path and pasted text are offered.
function AddDocuments({ onChange, mobile }) {
  const [path, setPath] = useState('');
  const [text, setText] = useState('');
  const [name, setName] = useState('pasted');
  const [msg, setMsg] = useState({ text: '' });
  const addFile = async (p) => {
    setMsg({ text: t('add-reading', 'Reading { $path }…', { path: p }) });
    try {
      const count = await api.ingest_file({ path: p });
      setMsg({ text: t('add-file-done', 'Added { $count } chunks from { $path }.', { count, path: p }) });
      onChange();
    } catch (e) {
      setMsg({ text: errorText(e), error: true });
    }
  };
  const addFileRef = useRef(addFile);
  useEffect(() => {
    addFileRef.current = addFile;
  });
  useEffect(() => {
    const un = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === 'drop') e.payload.paths.forEach((p) => addFileRef.current(p));
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  const addFolder = async (p) => {
    setMsg({ text: t('add-reading-folder', 'Reading every document in { $path }…', { path: p }) });
    try {
      const r = await api.ingest_folder({ path: p, recursive: true });
      const args = { files: r.files, chunks: r.chunks, failed: r.errors.length, error: r.errors[0] || '' };
      setMsg({
        text: r.errors.length
          ? t('add-folder-failed', 'Added { $files } files ({ $chunks } chunks), { $failed } failed: { $error }.', args)
          : t('add-folder-done', 'Added { $files } files ({ $chunks } chunks).', args),
        error: r.errors.length > 0,
      });
      onChange();
    } catch (e) {
      setMsg({ text: errorText(e), error: true });
    }
  };
  const addText = async () => {
    if (!text.trim()) return;
    try {
      const count = await api.ingest_text({ source: name || 'pasted', text });
      setMsg({ text: t('add-text-done', 'Added { $count } chunks.', { count }) });
      setText('');
      onChange();
    } catch (e) {
      setMsg({ text: errorText(e), error: true });
    }
  };
  return (
    <section>
      <h2>{t('add-title', 'Add documents')}</h2>
      <p className="hint">
        {mobile
          ? t('add-hint-mobile', 'Paste text, or give the path of a file in this app.')
          : t('add-hint', 'Drop files on this window (PDF, .txt, .md), give a file path, or paste text.')}
      </p>
      <div className="row">
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder={t('add-path-placeholder', '/path/to/file.pdf')}
          aria-label={t('add-path-label', 'File or folder path')}
        />
        <button onClick={() => path.trim() && addFile(path.trim())}>{t('add-file', 'Add file')}</button>
        {!mobile && <button onClick={() => path.trim() && addFolder(path.trim())}>{t('add-folder', 'Add folder')}</button>}
      </div>
      <textarea
        rows={4}
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder={t('add-paste-placeholder', 'Paste text here')}
        aria-label={t('add-paste-label', 'Text to add')}
      />
      <div className="row">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t('add-name', 'Name for the pasted text')}
          aria-label={t('add-name', 'Name for the pasted text')}
        />
        <button onClick={addText}>{t('add-text', 'Add text')}</button>
      </div>
      <Status text={msg.text} error={msg.error} />
    </section>
  );
}

function Search({ features }) {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState(null);
  const [error, setError] = useState('');
  const run = async () => {
    if (!query.trim()) return;
    setError('');
    try {
      setHits(await api.search({ query }));
    } catch (e) {
      setHits(null);
      setError(errorText(e));
    }
  };
  return (
    <section>
      <h2>{t('search-title', 'Search')}</h2>
      <div className="row">
        <input
          id="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && run()}
          placeholder={t('search-placeholder', 'What are you looking for?')}
          aria-label={t('search-title', 'Search')}
        />
        <button onClick={run}>{t('search-go', 'Search')}</button>
        {features?.stt && <Dictate onText={setQuery} />}
      </div>
      {error && <Status text={error} error />}
      {hits && (
        <ol id="results">
          {hits.length === 0 && <li className="meta">{t('search-none', 'Nothing found.')}</li>}
          {hits.map((h, i) => (
            <li key={i}>
              <div className="meta" title={provenance(h.prov)}>
                {h.source}
                {h.page ? t('search-page', ', page { $page }', { page: h.page }) : ''} · {h.score.toFixed(3)}
                {h.vector_score != null && t('search-vector-score', ' · vector { $score }', { score: h.vector_score.toFixed(3) })}
                {h.keyword_score != null && t('search-keyword-score', ' · keyword { $score }', { score: h.keyword_score.toFixed(2) })}
              </div>
              <div>{h.text}</div>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function BackgroundAgent() {
  const [agent, setAgent] = useState(null);
  const [error, setError] = useState('');
  useEffect(() => {
    api
      .agent_status()
      .then(setAgent)
      .catch((e) => setError(errorText(e)));
  }, []);
  const toggle = async (enabled) => {
    try {
      setAgent(await api.set_agent({ enabled }));
      setError('');
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <section>
      <h2>{t('agent-title', 'Background')}</h2>
      <label>
        <input type="checkbox" checked={!!agent?.installed} onChange={(e) => toggle(e.target.checked)} />{' '}
        {t('agent-toggle', 'Keep the backend running after the window closes, and start it at login')}
      </label>
      <Status
        text={
          error ||
          (agent
            ? agent.installed
              ? t('agent-on', 'On: { $kind } at { $location }', { kind: agent.kind, location: agent.location })
              : t('agent-off', 'Off ({ $kind })', { kind: agent.kind })
            : '')
        }
        error={!!error}
      />
    </section>
  );
}

function PythonExample() {
  const [text, setText] = useState('Python runs in its own process. It counts these words!');
  const [out, setOut] = useState('');
  const run = async () => {
    try {
      setOut(JSON.stringify(await api.python_call({ method: 'stats', params: { text } }), null, 2));
    } catch (e) {
      setOut(errorText(e));
    }
  };
  return (
    <section>
      <h2>{t('python-title', 'Python example')}</h2>
      <p className="hint">
        {t('python-hint', 'Sends this text to a separate Python process:')} <code>python/worker.py</code>
      </p>
      <textarea rows={2} value={text} onChange={(e) => setText(e.target.value)} />
      <button onClick={run}>{t('python-go', 'Text stats in Python')}</button>
      <pre className="status" role="status">
        {out}
      </pre>
    </section>
  );
}

function ClaudeDesktop() {
  const [msg, setMsg] = useState({ text: '' });
  const connect = async () => {
    try {
      const r = await api.register_claude();
      setMsg({
        text: r.backup
          ? t('claude-registered-backup', 'Claude Desktop config { $status } (backup: { $backup }). Preparing the tools once…', r)
          : t('claude-registered', 'Claude Desktop config { $status }. Preparing the tools once…', r),
      });
      const off = await listen('warmup-done', (e) => {
        setMsg(
          e.payload
            ? { text: t('claude-ready', 'Ready. Restart Claude Desktop to use it.') }
            : { text: t('claude-retry', 'Connected; the first start failed and will retry when Claude starts.'), error: true },
        );
        off();
      });
    } catch (e) {
      setMsg({ text: errorText(e), error: true });
    }
  };
  return (
    <section>
      <h2>{t('claude-title', 'Claude Desktop')}</h2>
      <p className="hint">{t('claude-hint', 'Adds this app’s tools to Claude Desktop (your current settings are backed up first).')}</p>
      <button onClick={connect}>{t('claude-go', 'Connect Claude Desktop')}</button>
      <Status text={msg.text} error={msg.error} />
    </section>
  );
}

export default function App() {
  const [info, setInfo] = useState(null);
  const [phase, setPhase] = useState('loading'); // loading | wizard | ready
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState({ text: t('starting', 'Starting…') });
  useLocale();
  const [status, setStatus] = useState(null);
  const [armed, setArmed] = useState(false);
  const [upgrade, setUpgrade] = useState(null);
  const [storeNotice, setStoreNotice] = useState('');
{% if app_shell %}  // App shell (feature app-shell): live themes, Cmd/Ctrl+, and Settings in its own window.
  useAppearance();
  useSettingsShortcut();
  const shell = drawsTitlebar();
{% endif %}
  // pgUpgrade answers the database-upgrade prompt: true upgrades now, false waits (Postgres only).
  const setup = async (backend, embeddings, pgUpgrade) => {
    setBusy(true);
    setUpgrade(null);
    const offs = [
      await listen('setup-progress', (e) => setMessage({ text: `${e.payload}…` })),
      await listen('setup-done', (e) => {
        offs.forEach((f) => f());
        setBusy(false);
        const error = e.payload.error || '';
        if (!e.payload.ok && error.includes(UPGRADE_NEEDED)) {
          // The data folder is from an older Postgres and postgres.upgrade is "ask".
          setMessage({ text: error.slice(error.indexOf(UPGRADE_NEEDED) + UPGRADE_NEEDED.length + 1).trim() });
          return setUpgrade({ backend, embeddings });
        }
        if (!e.payload.ok) return setMessage({ text: t('setup-failed', 'Setup failed: { $error }', { error }), error: true });
        setStatus(e.payload.status);
        if (e.payload.status?.notice) setStoreNotice(e.payload.status.notice);
        setPhase('ready');
      }),
    ];
    api.run_setup({ backend, embeddings, pgUpgrade }).catch((e) => {
      setBusy(false);
      setMessage({ text: errorText(e), error: true });
    });
  };

  useEffect(() => {
    api
      .app_info()
      .then(async (i) => {
        // The saved language, or the OS one (feature i18n; nothing to do without it).
        await setLocale(i.settings.config?.ui?.language || 'system');
        setConfig(i.config);
        setInfo(i);
        if (i.features.wizard && !i.settings.setupDone) setPhase('wizard');
        else setup(i.settings.backend, i.settings.embeddings);
      })
      .catch((e) => setMessage({ text: errorText(e), error: true }));
  }, []);

  const refresh = () =>
    api.app_info().then((i) => {
      setInfo(i);
      setStatus(i.status);
    });
  const clearAll = async () => {
    if (!armed) return setArmed(true);
    setArmed(false);
    setStatus(await api.clear());
  };
  const f = info?.features || {};

  // Tray, menu and shortcut actions (feature tray-menu): move the focus, show results.
  const [notice, setNotice] = useState(null);
  useEffect(() => {
    if (!f.trayMenu) return;
    const offs = [
      listen('app-action', (e) => {
        const id = e.payload.action === 'search' ? 'search' : e.payload.action === 'settings' ? 'settings' : null;
        const el = id && document.getElementById(id);
        if (!el) return;
        el.scrollIntoView({ behavior: 'smooth' });
        if (id === 'search') el.focus();
      }),
      listen('action-done', (e) => e.payload.message && setNotice({ text: e.payload.message, error: !e.payload.ok })),
    ];
    return () => offs.forEach((p) => p.then((off) => off()));
  }, [f.trayMenu]);

  return (
    <>
{% if app_shell %}      {shell && <Titlebar title={APP_NAME} />}
{% endif %}      {f.updater && <UpdateBanner />}
      <header>
        <h1>{APP_NAME}</h1>
        <p className="sub">{DESCRIPTION}</p>
      </header>
      {phase === 'loading' && (
        <section>
          <Status text={message.text} error={message.error} />
        </section>
      )}
      {phase === 'wizard' && <Wizard info={info} onSetup={setup} busy={busy} message={message} />}
      {upgrade && (
        <section className="upgrade" aria-label={t('upgrade-label', 'Database upgrade')}>
          <button onClick={() => setup(upgrade.backend, upgrade.embeddings, true)}>{t('upgrade-go', 'Upgrade database')}</button>{' '}
          <button onClick={() => setup(upgrade.backend, upgrade.embeddings, false)}>{t('upgrade-later', 'Not now')}</button>
        </section>
      )}
      {storeNotice && <Status text={storeNotice} error />}
      {notice && <Status text={notice.text} error={notice.error} />}
      {phase === 'ready' && (
        <main>
          <AddDocuments onChange={refresh} mobile={f.mobile} />
          <Search features={f} />
          {f.ai && <Ask features={f} />}
{% if app_shell %}          {!shell && <Settings features={f} />}
{% else %}          <Settings features={f} />
{% endif %}          {f.backgroundAgent && <BackgroundAgent />}
          {f.pythonExample && <PythonExample />}
          {f.claudeMcp && <ClaudeDesktop />}
          {/* kitchen:pages */}
        </main>
      )}
      <footer>
        <span>{describe(info, status)}</span>
        {configGet('ui.showEnvBadge', false, info?.config) && (
          <span className="meta">{t('footer-env', '{ $env } build', { env: info?.config?.env })}</span>
        )}
        {phase === 'ready' && (
          <button className="link" onClick={clearAll}>
            {armed ? t('clear-confirm', 'Click again to clear everything') : t('clear', 'Clear all data')}
          </button>
        )}
      </footer>
    </>
  );
}
