// Settings for search, ingest and folder watching (desktop-core's Config).
// Every optional behaviour is off until it is switched on here.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const num = (v) => (v === '' || v === null || v === undefined ? null : Number(v));

// [section, key, label, kind, extra]. kind: number | optional (number or empty) | bool | select | text | list
const FIELDS = [
  ['search', 'mode', 'Search mode', 'select', { options: ['auto', 'vector', 'keyword', 'hybrid'] }],
  ['search', 'topK', 'Results', 'number', { min: 1, max: 1000 }],
  ['search', 'candidates', 'Candidates per list (hybrid)', 'number', { min: 1, max: 5000 }],
  ['search', 'vectorWeight', 'Vector weight', 'number', { step: 0.1, min: 0 }],
  ['search', 'keywordWeight', 'Keyword weight', 'number', { step: 0.1, min: 0 }],
  ['search', 'rrfK', 'RRF k', 'number', { min: 1 }],
  ['search', 'minVectorScore', 'Minimum vector score (cosine)', 'optional', { step: 0.05 }],
  ['search', 'minKeywordScore', 'Minimum keyword score', 'optional', { step: 0.1 }],
  ['search', 'minScore', 'Minimum final score', 'optional', { step: 0.001 }],
  ['search', 'keywordEngine', 'Keyword engine (Postgres)', 'select', { options: ['auto', 'bm25', 'fulltext'], needs: 'postgres' }],
  ['search', 'bm25Tokenizer', 'BM25 tokenizer model', 'select', { options: ['bert_base_uncased', 'llmlingua2', 'gemma2b'], needs: 'bm25' }],
  ['search', 'fulltextLanguage', 'Full-text language', 'text', { needs: 'postgres' }],
  ['ingest', 'workers', 'Ingest workers (0 = one per core)', 'number', { min: 0, max: 64 }],
  ['ingest', 'chunkChars', 'Chunk size (characters)', 'number', { min: 100, max: 20000 }],
  ['ingest', 'chunkOverlap', 'Chunk overlap (characters)', 'number', { min: 0 }],
  ['ingest', 'embedBatch', 'Chunks per embedding call', 'number', { min: 1, max: 1024 }],
  ['ingest', 'maxFileMb', 'Largest file (MB)', 'number', { min: 1 }],
  ['ingest', 'extensions', 'File types', 'list', {}],
  ['llm', 'enabled', 'Turn on AI answers', 'bool', { needs: 'ai' }],
  ['llm', 'provider', 'Provider', 'select', { needs: 'ai', options: ['anthropic', 'openai', 'gemini', 'ollama', 'compatible', 'local'] }],
  ['llm', 'model', 'Model (empty = provider default)', 'text', { needs: 'ai' }],
  ['llm', 'baseUrl', 'Server URL (empty = provider default)', 'text', { needs: 'ai' }],
  ['llm', 'local.modelPath', 'Local model file (.gguf)', 'text', { needs: 'llama', nullable: true }],
  ['llm', 'local.modelUrl', 'Local model download URL', 'text', { needs: 'llama', nullable: true }],
  ['llm', 'local.context', 'Local context (tokens)', 'number', { needs: 'llama', min: 512 }],
  ['llm', 'local.gpuLayers', 'Layers on the GPU (0 = CPU only)', 'number', { needs: 'llama', min: 0 }],
  ['llm', 'local.threads', 'CPU threads (0 = one per core)', 'number', { needs: 'llama', min: 0 }],
  ['llm', 'local.keepLoaded', 'Keep the local model in memory', 'bool', { needs: 'llama' }],
  ['llm', 'temperature', 'Temperature', 'number', { needs: 'ai', step: 0.1, min: 0, max: 2 }],
  ['llm', 'maxTokens', 'Longest answer (tokens)', 'number', { needs: 'ai', min: 1 }],
  ['llm', 'timeoutSecs', 'Timeout (seconds)', 'number', { needs: 'ai', min: 5 }],
  ['llm', 'retries', 'Retries before the next provider', 'number', { needs: 'ai', min: 0, max: 10 }],
  ['llm', 'dailyTokenBudget', 'Daily token budget', 'optional', { needs: 'ai', min: 0 }],
  ['llm', 'monthlyTokenBudget', 'Monthly token budget', 'optional', { needs: 'ai', min: 0 }],
  ['llm', 'promptCache', 'Prompt caching (Anthropic)', 'bool', { needs: 'ai' }],
  ['llm', 'systemPrompt', 'System prompt (empty = built in)', 'text', { needs: 'ai', nullable: true }],
  ['llm', 'httpHosts', 'Hosts allowed over plain HTTP', 'list', { needs: 'ai' }],
  ['mcp', 'enabled', 'Let MCP clients (Claude Desktop, Claude Code, ...) search these documents', 'bool', { needs: 'mcp' }],
  ['mcp', 'allowWrite', 'Also allow tools that change data (add, remove)', 'bool', { needs: 'mcp' }],
  ['mcp', 'tools', 'Only these tools (empty = all allowed)', 'list', { needs: 'mcp' }],
  ['mcp', 'auditLog', 'Keep an audit log of tool calls', 'bool', { needs: 'mcp' }],
  ['mcp', 'auditArguments', 'Log the call arguments too', 'bool', { needs: 'mcp' }],
  ['mcp', 'maxResultChars', 'Longest reply (characters)', 'number', { needs: 'mcp', min: 1000 }],
  ['tts', 'enabled', 'Read answers aloud (tiny-voca, on this computer)', 'bool', { needs: 'tts' }],
  ['tts', 'voice', 'Voice', 'select', { needs: 'tts', options: [0, 1], labels: ['F2 (female)', 'M2 (male)'], numeric: true }],
  ['tts', 'speed', 'Speed', 'number', { needs: 'tts', step: 0.1, min: 0.25, max: 4 }],
  ['tts', 'pitchShift', 'Pitch shift', 'number', { needs: 'tts', step: 0.05, min: -1, max: 1 }],
  ['tts', 'pitchRange', 'Pitch range', 'number', { needs: 'tts', step: 0.1, min: 0, max: 3 }],
  ['tts', 'pauseMs', 'Pause between sentences (ms)', 'number', { needs: 'tts', min: 0, max: 5000 }],
  ['tts', 'threads', 'CPU threads (0 = automatic)', 'number', { needs: 'tts', min: 0 }],
  ['stt', 'enabled', 'Dictation and audio files (Whisper, on this computer)', 'bool', { needs: 'stt' }],
  [
    'stt',
    'model',
    'Model',
    'select',
    {
      needs: 'stt',
      options: ['tiny', 'base', 'small'],
      labels: ['Tiny (40 MB, fastest)', 'Base (75 MB)', 'Small (250 MB, most accurate)'],
    },
  ],
  ['stt', 'language', 'Language code, e.g. en, de (empty = detect)', 'text', { needs: 'stt' }],
  ['stt', 'translate', 'Translate into English', 'bool', { needs: 'stt' }],
  ['stt', 'maxSeconds', 'Longest recording (seconds)', 'number', { needs: 'stt', min: 5, max: 7200 }],
  ['stt', 'threads', 'CPU threads (0 = automatic)', 'number', { needs: 'stt', min: 0 }],
  ['telemetry', 'crashReports', 'Send crash reports (Sentry)', 'bool', { needs: 'telemetry' }],
  ['telemetry', 'sentryDsn', 'Sentry DSN', 'text', { needs: 'telemetry' }],
  ['telemetry', 'analytics', 'Send anonymous usage statistics', 'bool', { needs: 'telemetry' }],
  [
    'telemetry',
    'analyticsProvider',
    'Analytics service',
    'select',
    { needs: 'telemetry', options: ['posthog', 'aptabase'], labels: ['PostHog', 'Aptabase'] },
  ],
  ['telemetry', 'analyticsKey', 'Analytics project key', 'text', { needs: 'telemetry' }],
  ['telemetry', 'analyticsHost', 'Analytics host (empty = the service default)', 'text', { needs: 'telemetry' }],
  ['telemetry', 'traces', 'Send traces (OpenTelemetry)', 'bool', { needs: 'telemetry' }],
  ['telemetry', 'otlpEndpoint', 'OTLP endpoint, e.g. http://localhost:4318', 'text', { needs: 'telemetry' }],
  ['telemetry', 'otlpHeaders', 'OTLP headers (key=value, comma separated)', 'text', { needs: 'telemetry' }],
  ['telemetry', 'environment', 'Environment name', 'text', { needs: 'telemetry' }],
  ['watch', 'enabled', 'Watch folders and ingest new files', 'bool', { needs: 'watch' }],
  ['watch', 'dropFolder', 'Use the drop folder', 'bool', { needs: 'watch' }],
  ['watch', 'dropFolderPath', 'Drop folder (empty = Documents/<app> Inbox)', 'text', { needs: 'watch', nullable: true }],
  ['watch', 'paths', 'More folders to watch', 'list', { needs: 'watch', lines: true }],
  ['watch', 'recursive', 'Include subfolders', 'bool', { needs: 'watch' }],
  ['watch', 'debounceMs', 'Wait after the last change (ms)', 'number', { needs: 'watch', min: 100 }],
  ['watch', 'ingestExisting', 'Ingest files already there', 'bool', { needs: 'watch' }],
  ['watch', 'removeDeleted', 'Forget deleted files', 'bool', { needs: 'watch' }],
  ['watch', 'ignore', 'Ignore names (wildcards)', 'list', { needs: 'watch' }],
];

const TITLES = {
  search: 'Search',
  ingest: 'Ingest',
  llm: 'AI',
  mcp: 'MCP server',
  tts: 'Speech',
  stt: 'Dictation',
  telemetry: 'Privacy',
  watch: 'Watch folders',
};

function Field({ kind, label, value, extra, onChange }) {
  if (kind === 'bool')
    return (
      <label className="field">
        <input type="checkbox" checked={!!value} onChange={(e) => onChange(e.target.checked)} /> {label}
      </label>
    );
  if (kind === 'select')
    return (
      <label className="field">
        {label}
        <select value={value} onChange={(e) => onChange(extra.numeric ? Number(e.target.value) : e.target.value)}>
          {extra.options.map((o, i) => (
            <option key={o} value={o}>
              {extra.labels ? extra.labels[i] : o}
            </option>
          ))}
        </select>
      </label>
    );
  if (kind === 'list') {
    const sep = extra.lines ? '\n' : ', ';
    return (
      <label className="field">
        {label}
        <textarea
          rows={extra.lines ? 3 : 1}
          value={(value || []).join(sep)}
          onChange={(e) =>
            onChange(
              e.target.value
                .split(extra.lines ? '\n' : ',')
                .map((s) => s.trim())
                .filter(Boolean),
            )
          }
        />
      </label>
    );
  }
  const isNum = kind === 'number' || kind === 'optional';
  const change = (e) => {
    const v = e.target.value;
    if (kind === 'number') onChange(Number(v));
    else if (kind === 'optional') onChange(num(v));
    else onChange(extra.nullable && !v ? null : v);
  };
  return (
    <label className="field">
      {label}
      <input type={isNum ? 'number' : 'text'} min={extra.min} max={extra.max} step={extra.step} value={value ?? ''} onChange={change} />
    </label>
  );
}

function WatchStatus() {
  const [st, setSt] = useState(null);
  useEffect(() => {
    const tick = () =>
      invoke('watch_status')
        .then(setSt)
        .catch(() => {});
    tick();
    const t = setInterval(tick, 3000);
    return () => clearInterval(t);
  }, []);
  if (!st || !st.built) return null;
  const last = st.events.slice(-5).reverse();
  return (
    <div className="hint">
      <div>{st.folders.length ? `Watching: ${st.folders.join(', ')}` : 'Not watching.'}</div>
      {last.map((e, i) => (
        <div key={i}>
          {e.kind === 'ingested' && `Added ${e.files} files (${e.chunks} chunks)${e.errors.length ? `, ${e.errors.length} failed` : ''}`}
          {e.kind === 'removed' && `Forgot ${e.path}`}
          {e.kind === 'error' && `Problem: ${e.message}`}
          {e.kind === 'started' && `Started on ${e.folders.length} folders`}
        </div>
      ))}
    </div>
  );
}

function VoiceDownload() {
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    const unlisten = await listen('voice-progress', (e) => setMsg(`Downloading ${e.payload}`));
    try {
      await invoke('download_voice');
      setMsg('Voice ready.');
    } catch (e) {
      setMsg(String(e));
    } finally {
      unlisten();
      setBusy(false);
    }
  };
  return (
    <div className="row">
      <button onClick={go} disabled={busy}>
        Download the voice (25 MB)
      </button>
      <span className="hint">{msg}</span>
    </div>
  );
}

function SttDownload() {
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    const unlisten = await listen('stt-progress', (e) => setMsg(`Downloading ${e.payload}`));
    try {
      await invoke('download_stt');
      setMsg('Model ready.');
    } catch (e) {
      setMsg(String(e));
    } finally {
      unlisten();
      setBusy(false);
    }
  };
  return (
    <div className="row">
      <button onClick={go} disabled={busy}>
        Download the chosen model (save settings first)
      </button>
      <span className="hint">{msg}</span>
    </div>
  );
}

function TelemetryStatus() {
  const [s, setS] = useState(null);
  useEffect(() => {
    invoke('telemetry_status')
      .then(setS)
      .catch(() => {});
  }, []);
  if (!s?.running) return <p className="hint">Nothing is sent unless you switch it on above.</p>;
  const on = [s.crashReports && 'crash reports', s.analytics && 'usage statistics', s.traces && 'traces'].filter(Boolean);
  return (
    <p className="hint">
      Sending: {on.length ? on.join(', ') : 'nothing'}. Anonymous id {s.installId.slice(0, 8)}…, {s.sent} sent
      {s.pendingCrashes ? `, ${s.pendingCrashes} crash reports kept on this computer` : ''}
      {s.lastError ? `. Last problem: ${s.lastError}` : ''}. Only call names, timings and counts are sent, never your documents or
      questions.
    </p>
  );
}

function McpConnect() {
  const [setup, setSetup] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    invoke('mcp_setup')
      .then(setSetup)
      .catch(() => {});
  }, []);
  const register = async () => {
    try {
      const r = await invoke('mcp_register_claude');
      setMsg(`Claude Desktop: ${r}. Restart Claude Desktop to load it.`);
    } catch (e) {
      setMsg(String(e));
    }
  };
  if (!setup) return null;
  return (
    <div className="hint">
      <div className="row">
        <button onClick={register}>Add to Claude Desktop</button>
        <span>{msg}</span>
      </div>
      <div>
        Claude Code: <code>{setup.claude_code}</code>
      </div>
      <div>Save settings with the server turned on first.</div>
    </div>
  );
}

function ModelDownload({ onDone }) {
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    const unlisten = await listen('model-progress', (e) => setMsg(`Downloading: ${e.payload}`));
    try {
      const path = await invoke('download_model', {});
      onDone(path);
      setMsg(`Ready: ${path}`);
    } catch (e) {
      setMsg(String(e));
    } finally {
      unlisten();
      setBusy(false);
    }
  };
  return (
    <div className="row">
      <button onClick={go} disabled={busy}>
        Download the local model
      </button>
      <span className="hint">{msg}</span>
    </div>
  );
}

function AiKeys() {
  const [have, setHave] = useState([]);
  const [provider, setProvider] = useState('anthropic');
  const [key, setKey] = useState('');
  const [msg, setMsg] = useState('');
  useEffect(() => {
    invoke('ai_keys')
      .then(setHave)
      .catch(() => {});
  }, []);
  const save = async (value) => {
    try {
      setHave(await invoke('set_ai_key', { provider, key: value }));
      setKey('');
      setMsg(value ? 'Key saved.' : 'Key removed.');
    } catch (e) {
      setMsg(String(e));
    }
  };
  return (
    <div className="row">
      <select value={provider} onChange={(e) => setProvider(e.target.value)} aria-label="Key for provider">
        {['anthropic', 'openai', 'gemini', 'ollama', 'compatible'].map((p) => (
          <option key={p}>{p}</option>
        ))}
      </select>
      <input
        type="password"
        autoComplete="off"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="API key"
        aria-label="API key"
      />
      <button onClick={() => save(key)} disabled={!key}>
        Save key
      </button>
      {have.includes(provider) && (
        <button className="link" onClick={() => save(null)}>
          Remove
        </button>
      )}
      <span className="hint">
        {have.length ? `Keys set: ${have.join(', ')}` : 'No keys set.'} {msg}
      </span>
    </div>
  );
}

export default function Settings({ features }) {
  const [cfg, setCfg] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    invoke('get_config')
      .then(setCfg)
      .catch((e) => setMsg(String(e)));
  }, []);
  if (!cfg) return null;
  // Keys like 'local.modelPath' reach one level down.
  const set = (section, key, v) => {
    const [k, sub] = key.split('.');
    const value = sub ? { ...cfg[section][k], [sub]: v } : v;
    setCfg({ ...cfg, [section]: { ...cfg[section], [k]: value } });
  };
  const save = async () => {
    try {
      setCfg(await invoke('set_config', { config: cfg }));
      setMsg('Saved.');
    } catch (e) {
      setMsg(`Not saved: ${e}`);
    }
  };
  const reset = async () => {
    setMsg('');
    setCfg(await invoke('set_config', { config: {} }));
  };
  const needs = { watch: 'watch', llm: 'ai', mcp: 'mcp', tts: 'tts', stt: 'stt', telemetry: 'telemetry' };
  const sections = ['search', 'ingest', 'llm', 'mcp', 'tts', 'stt', 'telemetry', 'watch'].filter((s) => !needs[s] || features[needs[s]]);
  return (
    <section>
      <h2>Settings</h2>
      {sections.map((s) => (
        <details key={s}>
          <summary>{TITLES[s]}</summary>
          <div className="grid">
            {FIELDS.filter(([sec, , , , x]) => sec === s && (!x.needs || features[x.needs])).map(([sec, key, label, kind, extra]) => (
              <Field
                key={key}
                kind={kind}
                label={label}
                extra={extra}
                value={key.split('.').reduce((o, k) => o?.[k], cfg[sec])}
                onChange={(v) => set(sec, key, v)}
              />
            ))}
          </div>
          {s === 'watch' && <WatchStatus />}
          {s === 'llm' && <AiKeys />}
          {s === 'llm' && features.llama && <ModelDownload onDone={(p) => set('llm', 'local.modelPath', p)} />}
          {s === 'mcp' && <McpConnect />}
          {s === 'tts' && <VoiceDownload />}
          {s === 'stt' && <SttDownload />}
          {s === 'telemetry' && <TelemetryStatus />}
        </details>
      ))}
      <div className="row">
        <button onClick={save}>Save settings</button>
        <button className="link" onClick={reset}>
          Reset to defaults
        </button>
      </div>
      <p className="status">{msg}</p>
    </section>
  );
}
