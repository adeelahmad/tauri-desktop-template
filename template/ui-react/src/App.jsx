// React frontend talking to the Rust commands in src-tauri/src/lib.rs.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { APP_NAME, DESCRIPTION, configGet, setConfig } from './config.js';
import { errorText } from './errors.js';
import Settings from './Settings.jsx';
import Ask from './Ask.jsx';
import Dictate from './Dictate.jsx';
import { provenance } from './provenance.js';
{% if app_shell %}import Titlebar from './Titlebar.jsx';
import { drawsTitlebar, useAppearance, useSettingsShortcut } from './shell.js';
{% endif %}// kitchen:imports (`kitchen generate page` adds imports above this line)

function describe(info, status) {
  if (!info) return '';
  const s = status || info.status;
  const base = `${info.name} ${info.version}`;
  if (!s) return base;
  return `${base} · ${s.chunks} chunks · ${s.backend} store · ${s.embedder || 'keyword search'}`;
}

function Status({ text, error }) {
  return <p className={error ? 'status error' : 'status'}>{text}</p>;
}

function Wizard({ info, onSetup, busy, message }) {
  const f = info.features;
  const [backend, setBackend] = useState('file');
  const [embed, setEmbed] = useState(true);
  return (
    <section>
      <h2>Welcome</h2>
      <p className="hint">A one-time setup. Everything stays on this {f.mobile ? 'device' : 'computer'}.</p>
      <ol className="steps">
        <li>
          <h3>1. Where to keep your data</h3>
          <label>
            <input type="radio" checked={backend === 'file'} onChange={() => setBackend('file')} /> A simple file (fast, nothing to
            download)
          </label>
          {f.postgres && (
            <label>
              <input
                type="radio"
                disabled={!info.postgresAvailable}
                checked={backend === 'postgres'}
                onChange={() => setBackend('postgres')}
              />{' '}
              A private Postgres database with vector search (downloads about 30 MB the first time)
            </label>
          )}
          {f.postgres && !info.postgresAvailable && (
            <p className="hint">This processor cannot run the bundled database; the file store works the same.</p>
          )}
        </li>
        {f.embed && (
          <li>
            <h3>2. Smart search</h3>
            <label>
              <input type="checkbox" checked={embed} onChange={(e) => setEmbed(e.target.checked)} /> Search by meaning with a local AI model
              (downloads once, nothing is sent anywhere)
            </label>
          </li>
        )}
        <li>
          <h3>3. Set up</h3>
          <button disabled={busy} onClick={() => onSetup(backend, embed)}>
            Set up
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
    invoke('check_update')
      .then((u) => u.available && setUpdate(u))
      .catch(() => {});
  }, []);
  if (!update) return null;
  const install = async () => {
    const off = await listen('update-progress', (e) => {
      const [done, total] = e.payload;
      setText(total ? `Downloading… ${Math.round((100 * done) / total)}%` : 'Downloading…');
    });
    try {
      await invoke('install_update');
    } catch (e) {
      setText(`Update failed: ${e}`);
      off();
    }
  };
  return (
    <div className="banner">
      <span>{text || `Version ${update.version} is available (you have ${update.current}).`}</span>
      <button onClick={install}>Install and restart</button>
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
    setMsg({ text: `Reading ${p}…` });
    try {
      setMsg({ text: `Added ${await invoke('ingest_file', { path: p })} chunks from ${p}.` });
      onChange();
    } catch (e) {
      setMsg({ text: String(e), error: true });
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
    setMsg({ text: `Reading every document in ${p}…` });
    try {
      const r = await invoke('ingest_folder', { path: p, recursive: true });
      const failed = r.errors.length ? `, ${r.errors.length} failed: ${r.errors[0]}` : '';
      setMsg({ text: `Added ${r.files} files (${r.chunks} chunks)${failed}.`, error: r.errors.length > 0 });
      onChange();
    } catch (e) {
      setMsg({ text: String(e), error: true });
    }
  };
  const addText = async () => {
    if (!text.trim()) return;
    try {
      setMsg({ text: `Added ${await invoke('ingest_text', { source: name || 'pasted', text })} chunks.` });
      setText('');
      onChange();
    } catch (e) {
      setMsg({ text: String(e), error: true });
    }
  };
  return (
    <section>
      <h2>Add documents</h2>
      <p className="hint">
        {mobile
          ? 'Paste text, or give the path of a file in this app.'
          : 'Drop files on this window (PDF, .txt, .md), give a file path, or paste text.'}
      </p>
      <div className="row">
        <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/path/to/file.pdf" aria-label="File or folder path" />
        <button onClick={() => path.trim() && addFile(path.trim())}>Add file</button>
        {!mobile && <button onClick={() => path.trim() && addFolder(path.trim())}>Add folder</button>}
      </div>
      <textarea rows={4} value={text} onChange={(e) => setText(e.target.value)} placeholder="Paste text here" aria-label="Text to add" />
      <div className="row">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Name for the pasted text"
          aria-label="Name for the pasted text"
        />
        <button onClick={addText}>Add text</button>
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
      setHits(await invoke('search', { query }));
    } catch (e) {
      setHits(null);
      setError(String(e));
    }
  };
  return (
    <section>
      <h2>Search</h2>
      <div className="row">
        <input
          id="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && run()}
          placeholder="What are you looking for?"
          aria-label="Search"
        />
        <button onClick={run}>Search</button>
        {features?.stt && <Dictate onText={setQuery} />}
      </div>
      {error && <Status text={error} error />}
      {hits && (
        <ol id="results">
          {hits.length === 0 && <li className="meta">Nothing found.</li>}
          {hits.map((h, i) => (
            <li key={i}>
              <div className="meta" title={provenance(h.prov)}>
                {h.source}
                {h.page ? `, page ${h.page}` : ''} · {h.score.toFixed(3)}
                {h.vector_score != null && ` · vector ${h.vector_score.toFixed(3)}`}
                {h.keyword_score != null && ` · keyword ${h.keyword_score.toFixed(2)}`}
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
    invoke('agent_status')
      .then(setAgent)
      .catch((e) => setError(String(e)));
  }, []);
  const toggle = async (enabled) => {
    try {
      setAgent(await invoke('set_agent', { enabled }));
      setError('');
    } catch (e) {
      setError(String(e));
    }
  };
  return (
    <section>
      <h2>Background</h2>
      <label>
        <input type="checkbox" checked={!!agent?.installed} onChange={(e) => toggle(e.target.checked)} /> Keep the backend running after the
        window closes, and start it at login
      </label>
      <Status
        text={error || (agent ? (agent.installed ? `On: ${agent.kind} at ${agent.location}` : `Off (${agent.kind})`) : '')}
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
      setOut(JSON.stringify(await invoke('python_call', { method: 'stats', params: { text } }), null, 2));
    } catch (e) {
      setOut(String(e));
    }
  };
  return (
    <section>
      <h2>Python example</h2>
      <p className="hint">
        Sends this text to <code>python/worker.py</code>, a separate Python process.
      </p>
      <textarea rows={2} value={text} onChange={(e) => setText(e.target.value)} />
      <button onClick={run}>Text stats in Python</button>
      <pre className="status">{out}</pre>
    </section>
  );
}

function ClaudeDesktop() {
  const [msg, setMsg] = useState({ text: '' });
  const connect = async () => {
    try {
      const r = await invoke('register_claude');
      setMsg({ text: `Claude Desktop config ${r.status}${r.backup ? ` (backup: ${r.backup})` : ''}. Preparing the tools once…` });
      const off = await listen('warmup-done', (e) => {
        setMsg(
          e.payload
            ? { text: 'Ready. Restart Claude Desktop to use it.' }
            : { text: 'Connected; the first start failed and will retry when Claude starts.', error: true },
        );
        off();
      });
    } catch (e) {
      setMsg({ text: String(e), error: true });
    }
  };
  return (
    <section>
      <h2>Claude Desktop</h2>
      <p className="hint">Adds this app’s tools to Claude Desktop (your current settings are backed up first).</p>
      <button onClick={connect}>Connect Claude Desktop</button>
      <Status text={msg.text} error={msg.error} />
    </section>
  );
}

export default function App() {
  const [info, setInfo] = useState(null);
  const [phase, setPhase] = useState('loading'); // loading | wizard | ready
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState({ text: 'Starting…' });
  const [status, setStatus] = useState(null);
  const [armed, setArmed] = useState(false);
{% if app_shell %}  // App shell (feature app-shell): live themes, Cmd/Ctrl+, and Settings in its own window.
  useAppearance();
  useSettingsShortcut();
  const shell = drawsTitlebar();
{% endif %}
  const setup = async (backend, embeddings) => {
    setBusy(true);
    const offs = [
      await listen('setup-progress', (e) => setMessage({ text: `${e.payload}…` })),
      await listen('setup-done', (e) => {
        offs.forEach((f) => f());
        setBusy(false);
        if (!e.payload.ok) return setMessage({ text: `Setup failed: ${e.payload.error}`, error: true });
        setStatus(e.payload.status);
        setPhase('ready');
      }),
    ];
    invoke('run_setup', { backend, embeddings }).catch((e) => {
      setBusy(false);
      setMessage({ text: String(e), error: true });
    });
  };

  useEffect(() => {
    invoke('app_info')
      .then((i) => {
        setConfig(i.config);
        setInfo(i);
        if (i.features.wizard && !i.settings.setupDone) setPhase('wizard');
        else setup(i.settings.backend, i.settings.embeddings);
      })
      .catch((e) => setMessage({ text: errorText(e), error: true }));
  }, []);

  const refresh = () =>
    invoke('app_info').then((i) => {
      setInfo(i);
      setStatus(i.status);
    });
  const clearAll = async () => {
    if (!armed) return setArmed(true);
    setArmed(false);
    setStatus(await invoke('clear'));
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
        {configGet('ui.showEnvBadge', false, info?.config) && <span className="meta">{info.config.env} build</span>}
        {phase === 'ready' && (
          <button className="link" onClick={clearAll}>
            {armed ? 'Click again to clear everything' : 'Clear all data'}
          </button>
        )}
      </footer>
    </>
  );
}
