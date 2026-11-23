// Settings for search, ingest and folder watching (desktop-core's Config).
// Every optional behaviour is off until it is switched on here.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

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

const TITLES = { search: 'Search', ingest: 'Ingest', watch: 'Watch folders' };

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
        <select value={value} onChange={(e) => onChange(e.target.value)}>
          {extra.options.map((o) => (
            <option key={o}>{o}</option>
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

export default function Settings({ features }) {
  const [cfg, setCfg] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    invoke('get_config')
      .then(setCfg)
      .catch((e) => setMsg(String(e)));
  }, []);
  if (!cfg) return null;
  const set = (section, key, v) => setCfg({ ...cfg, [section]: { ...cfg[section], [key]: v } });
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
  const sections = ['search', 'ingest', 'watch'].filter((s) => s !== 'watch' || features.watch);
  return (
    <section>
      <h2>Settings</h2>
      {sections.map((s) => (
        <details key={s}>
          <summary>{TITLES[s]}</summary>
          <div className="grid">
            {FIELDS.filter(([sec, , , , x]) => sec === s && (!x.needs || features[x.needs])).map(([sec, key, label, kind, extra]) => (
              <Field key={key} kind={kind} label={label} extra={extra} value={cfg[sec][key]} onChange={(v) => set(sec, key, v)} />
            ))}
          </div>
          {s === 'watch' && <WatchStatus />}
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
