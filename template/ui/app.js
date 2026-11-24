// Plain JS, no build step: the Tauri API comes from `withGlobalTauri`.
// Typed IPC: bindings/*.d.ts (written by `cargo test -p <app> export_bindings`) give `invoke`
// each command's argument and result types in the editor (docs/GUIDE.md, "Typed IPC").
/** @type {import('./bindings/commands').Invoke} */
const invoke = window.__TAURI__.core.invoke;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const show = (id, on = true) => { $(id).hidden = !on; };
const say = (id, text, error = false) => { $(id).textContent = text; $(id).classList.toggle('error', error); };
// Translations (feature `i18n`, ui/i18n.js): t(id, english, args), English with { $name } variables. Without
// the feature the English is used as it is.
const t = window.i18n ? window.i18n.t : (id, english, args = {}) => english.replace(/\{ \$(\w+) \}/g, (_, k) => String(args[k] ?? ''));

/** @type {import('./bindings/types').AppInfo} */
let info;

// One line on where a passage came from (chunk provenance from the backend).
function provenance(p) {
  if (!p) return '';
  const parts = [p.extractor, p.sha256 && `sha256 ${p.sha256.slice(0, 12)}`, p.model, p.ingested && `indexed ${p.ingested.slice(0, 16).replace('T', ' ')} UTC`];
  return parts.filter(Boolean).join(' · ');
}

async function start() {
  if (window.i18n) await window.i18n.ready;
  info = await invoke('app_info');
  const f = info.features;
  $('about').textContent = `${info.name} ${info.version}`;
  show('check-update', f.updater);
  show('claude', f.claudeMcp);
  show('python', f.pythonExample);
  if (f.backgroundAgent) { show('agent'); refreshAgent(); }
  if (f.daemon) { show('service'); refreshService(); invoke('service_system_hint').then((h) => { $('service-hint').textContent = h; }).catch(() => {}); }
  if (f.mcp) { show('mcp'); invoke('mcp_setup').then((s) => { $('mcp-cmd').textContent = s.claude_code; }).catch(() => {}); }
  show('llama', f.llama);
  show('tts', f.tts);
  show('stt', f.stt);
  if (f.ai) { show('ask'); invoke('ai_keys').then(keysText).catch(() => {}); }
  show('w-pg-option', f.postgres);
  if (f.postgres && !info.postgresAvailable) {
    $('w-pg-option').querySelector('input').disabled = true;
    say('w-pg-note', t('wizard-no-postgres', 'This processor cannot run the bundled database; the file store works the same.'));
    show('w-pg-note');
  }
  show('w-model', f.embed);
  // A model built into the installer (bundled models) needs no download step.
  if ((info.models || []).some((m) => m.kind === 'embed' && m.present)) {
    const en = 'Search by meaning with a local AI model (built into the app, no download; nothing is sent anywhere)';
    Object.assign($('w-embed-text').dataset, { i18n: 'wizard-model-embed-bundled', i18nEn: en });
    $('w-embed-text').textContent = t('wizard-model-embed-bundled', en);
  }
  // iOS and Android: no folders of the user's to read.
  show('add-folder', !f.mobile);
  if (f.wizard && !info.settings.setupDone) {
    show('loading', false);
    show('wizard');
  } else {
    setup(info.settings.backend, info.settings.embeddings, 'loading');
  }
  if (f.updater) checkUpdate(true);
}

// The start of the error that asks before the database is upgraded (desktop_core::pgupgrade).
const UPGRADE_NEEDED = 'pg-upgrade-needed';

// Opens the store and model; progress lines arrive as events. pgUpgrade answers the
// database-upgrade prompt: true upgrades now, false waits (Postgres only).
function setup(backend, embeddings, statusArea, pgUpgrade) {
  show('pg-upgrade', false);
  const statusId = statusArea === 'loading' ? null : 'w-status';
  const line = (t, e) => statusId ? say(statusId, t, e) : ($('loading').firstElementChild.textContent = t);
  const offs = [];
  listen('setup-progress', (e) => line(`${e.payload}…`)).then((u) => offs.push(u));
  listen('setup-done', (e) => {
    offs.forEach((u) => u());
    show('w-progress', false);
    $('w-go').disabled = false;
    const error = e.payload.error || '';
    if (!e.payload.ok && error.includes(UPGRADE_NEEDED)) {
      // The data folder is from an older Postgres and postgres.upgrade is "ask".
      line(error.slice(error.indexOf(UPGRADE_NEEDED) + UPGRADE_NEEDED.length + 1).trim());
      $('pg-upgrade-yes').onclick = () => setup(backend, embeddings, statusArea, true);
      $('pg-upgrade-no').onclick = () => setup(backend, embeddings, statusArea, false);
      return show('pg-upgrade');
    }
    if (!e.payload.ok) return line(t('setup-failed', 'Setup failed: { $error }', { error }), true);
    if (e.payload.status.notice) { $('store-notice').textContent = e.payload.status.notice; show('store-notice'); }
    show('wizard', false); show('loading', false); show('main'); show('clear'); loadConfig().catch(() => {});
    const s = e.payload.status;
    $('about').textContent = footer(info, s);
  }).then((u) => offs.push(u));
  invoke('run_setup', { backend, embeddings, pgUpgrade }).catch((e) => line(errorText(e), true));
}

$('w-go').onclick = () => {
  $('w-go').disabled = true;
  show('w-progress');
  const backend = document.querySelector('input[name=backend]:checked').value;
  setup(backend, $('w-embed').checked, 'wizard');
};

// The footer line: name, version, chunks, store and embedder.
const footer = (i, s) => t('footer-status', '{ $app } · { $chunks } chunks · { $backend } store · { $embedder }', {
  app: `${i.name} ${i.version}`, chunks: s.chunks, backend: s.backend, embedder: s.embedder || t('footer-keyword-search', 'keyword search'),
});
async function refreshAbout() {
  const i = await invoke('app_info');
  if (i.status) $('about').textContent = footer(i, i.status);
}

async function addFile(path) {
  say('add-status', t('add-reading', 'Reading { $path }…', { path }));
  try {
    const n = await invoke('ingest_file', { path });
    say('add-status', t('add-file-done', 'Added { $count } chunks from { $path }.', { count: n, path }));
    refreshAbout();
  } catch (e) { say('add-status', errorText(e), true); }
}

$('add-file').onclick = () => { const p = $('file-path').value.trim(); if (p) addFile(p); };
$('add-folder').onclick = async () => {
  const path = $('file-path').value.trim();
  if (!path) return;
  say('add-status', t('add-reading-folder', 'Reading every document in { $path }…', { path }));
  try {
    const r = await invoke('ingest_folder', { path, recursive: true });
    say('add-status', r.errors.length
      ? t('add-folder-failed', 'Added { $files } files ({ $chunks } chunks), { $failed } failed: { $error }.', { files: r.files, chunks: r.chunks, failed: r.errors.length, error: r.errors[0] })
      : t('add-folder-done', 'Added { $files } files ({ $chunks } chunks).', { files: r.files, chunks: r.chunks }), r.errors.length > 0);
    refreshAbout();
  } catch (e) { say('add-status', errorText(e), true); }
};

// ---- settings (desktop-core Config) ----
async function loadConfig() { $('config').value = JSON.stringify(await invoke('get_config'), null, 2); }
async function saveConfig(config) {
  try {
    $('config').value = JSON.stringify(await invoke('set_config', { config }), null, 2);
    say('config-status', t('settings-saved', 'Saved.'));
  } catch (e) { say('config-status', t('settings-not-saved', 'Not saved: { $error }', { error: errorText(e) }), true); }
}
$('config-save').onclick = () => {
  let config;
  try { config = JSON.parse($('config').value); } catch (e) { return say('config-status', `Not valid JSON: ${e.message}`, true); }
  saveConfig(config);
};
$('config-reset').onclick = () => saveConfig({});
$('add-text').onclick = async () => {
  const text = $('paste').value;
  if (!text.trim()) return;
  try {
    const n = await invoke('ingest_text', { source: $('paste-name').value || 'pasted', text });
    say('add-status', t('add-text-done', 'Added { $count } chunks.', { count: n }));
    $('paste').value = '';
    refreshAbout();
  } catch (e) { say('add-status', errorText(e), true); }
};
listen('tauri://drag-drop', async (e) => { for (const p of e.payload.paths || []) await addFile(p); });

async function runSearch() {
  const query = $('query').value.trim();
  if (!query) return;
  const list = $('results');
  list.innerHTML = '';
  try {
    const hits = await invoke('search', { query });
    if (!hits.length) { const li = document.createElement('li'); li.className = 'meta'; li.textContent = t('search-none', 'Nothing found.'); list.append(li); }
    for (const h of hits) {
      const li = document.createElement('li');
      const meta = document.createElement('div');
      meta.className = 'meta';
      meta.textContent = `${h.source}${h.page ? t('search-page', ', page { $page }', { page: h.page }) : ''} · ${h.score.toFixed(3)}` +
        (h.vector_score != null ? t('search-vector-score', ' · vector { $score }', { score: h.vector_score.toFixed(3) }) : '') +
        (h.keyword_score != null ? t('search-keyword-score', ' · keyword { $score }', { score: h.keyword_score.toFixed(2) }) : '');
      meta.title = provenance(h.prov);
      const body = document.createElement('div');
      body.textContent = h.text;
      li.append(meta, body);
      list.append(li);
    }
  } catch (e) { list.innerHTML = ''; const li = document.createElement('li'); li.className = 'status error'; li.textContent = errorText(e); list.append(li); }
}
$('search').onclick = runSearch;
$('query').addEventListener('keydown', (e) => { if (e.key === 'Enter') runSearch(); });

$('clear').onclick = async () => {
  if ($('clear').dataset.armed !== '1') { $('clear').dataset.armed = '1'; $('clear').textContent = t('clear-confirm', 'Click again to clear everything'); return; }
  $('clear').dataset.armed = ''; $('clear').textContent = t('clear', 'Clear all data');
  await invoke('clear');
  refreshAbout();
};

// ---- updates (feature `updater`) ----
async function checkUpdate(quiet) {
  try {
    const u = await invoke('check_update');
    if (!u.configured) { if (!quiet) say('add-status', t('update-not-configured', 'Updates are not set up in this build (no updater public key).')); return; }
    if (u.available) {
      $('update-text').textContent = t('update-available', 'Version { $version } is available (you have { $current }).', { version: u.version, current: u.current });
      show('update-banner');
    } else if (!quiet) say('add-status', t('update-newest', 'You have the newest version ({ $version }).', { version: u.current }));
  } catch (e) { if (!quiet) say('add-status', t('update-check-failed', 'Update check failed: { $error }', { error: errorText(e) }), true); }
}
$('check-update').onclick = () => checkUpdate(false);
$('update-install').onclick = async () => {
  $('update-install').disabled = true;
  listen('update-progress', (e) => {
    const [done, total] = e.payload;
    $('update-text').textContent = total ? t('update-downloading-percent', 'Downloading… { $percent }%', { percent: Math.round((100 * done) / total) }) : t('update-downloading', 'Downloading…');
  });
  try { await invoke('install_update'); } catch (e) { $('update-text').textContent = t('update-failed', 'Update failed: { $error }', { error: errorText(e) }); $('update-install').disabled = false; }
};

// ---- background agent (feature `background-agent`) ----
async function refreshAgent(set) {
  try {
    const a = set === undefined ? await invoke('agent_status') : await invoke('set_agent', { enabled: set });
    $('agent-on').checked = a.installed;
    say('agent-status', a.installed ? t('agent-on', 'On: { $kind } at { $location }', a) : t('agent-off', 'Off ({ $kind })', a));
  } catch (e) { say('agent-status', errorText(e), true); }
}
$('agent-on').onchange = () => refreshAgent($('agent-on').checked);

// ---- the backend as a service (feature `daemon`) ----
async function refreshService(set) {
  try {
    const s = set === undefined ? await invoke('service_status') : await invoke('set_service', { enabled: set });
    $('service-on').checked = s.installed;
    // The same text as Settings -> Service in the React UI; the pid is an id, so text (no separators).
    const running = s.running
      ? (s.pid ? t('service-running-pid', 'running (pid { $pid })', { pid: String(s.pid) }) : t('service-running', 'running'))
      : t('service-not-running', 'not running');
    const status = t('service-status', '{ $manager }: { $installed }, { $running }. Log: { $log }.', {
      manager: s.manager, installed: s.installed ? t('service-installed', 'installed') : t('service-not-installed', 'not installed'), running, log: s.log,
    });
    say('service-status', `${status} ${s.note || ''}`);
  } catch (e) { say('service-status', errorText(e), true); }
}
$('service-on').onchange = () => refreshService($('service-on').checked);

// ---- Python example (feature `python-example`) ----
$('python-go').onclick = async () => {
  try {
    const r = await invoke('python_call', { method: 'stats', params: { text: $('paste').value || 'Type some text in the box above.' } });
    $('python-out').textContent = JSON.stringify(r, null, 2);
  } catch (e) { say('python-out', errorText(e), true); }
};

// ---- Claude Desktop (feature `claude-mcp`) ----
$('claude-go').onclick = async () => {
  $('claude-go').disabled = true;
  try {
    const r = await invoke('register_claude');
    say('claude-status', r.backup
      ? t('claude-registered-backup', 'Claude Desktop config { $status } (backup: { $backup }). Preparing the tools once…', r)
      : t('claude-registered', 'Claude Desktop config { $status }. Preparing the tools once…', r));
    listen('warmup-done', (e) => say('claude-status', e.payload ? t('claude-ready', 'Ready. Restart Claude Desktop to use it.') : t('claude-retry', 'Connected; the first start failed and will retry when Claude starts.'), !e.payload));
  } catch (e) { say('claude-status', errorText(e), true); }
  $('claude-go').disabled = false;
};

start().catch((e) => say('loading', errorText(e), true));

// Language and digits (feature `i18n`): switch at once, save only `ui.language` and
// `ui.digits`, and tell the Settings window. The backend picks the digits (Latin unless
// `ui.digits` is "native") and hands them over with the bundle (`numbers`).
if (window.i18n && $('language')) {
  window.i18n.ready.then(async () => {
    for (const l of window.i18n.available()) {
      const o = document.createElement('option');
      o.value = l.code; o.textContent = l.name; o.lang = l.code; o.dir = l.dir;
      $('language').append(o);
    }
    const ui = (await invoke('get_config')).ui || {};
    $('language').value = ui.language || 'system';
    $('digits').value = ui.digits || 'latin';
  });
  $('language').onchange = $('digits').onchange = async () => {
    const ui = { language: $('language').value, digits: $('digits').value };
    try {
      const saved = await invoke('get_config');
      await invoke('set_config', { config: { ...saved, ui: { ...saved.ui, ...ui } } });
      await window.i18n.load(ui.language);
      await window.__TAURI__.event.emit('locale-changed', ui);
      refreshAbout();
      loadConfig().catch(() => {});
    } catch (e) { say('config-status', errorText(e), true); }
  };
}

// AI answers (feature `ai`): the reply streams in as `ask-token` events.
async function ask() {
  const question = $('question').value.trim();
  if (!question) return;
  $('answer').textContent = '';
  $('sources').innerHTML = '';
  const off = await listen('ask-token', (e) => { $('answer').textContent += e.payload; });
  try {
    const r = await invoke('ask', { question });
    $('answer').textContent = r.reply.text;
    r.sources.forEach((h, i) => {
      const li = document.createElement('li');
      li.className = 'meta';
      li.textContent = `[${i + 1}] ${h.source}${h.page ? t('search-page', ', page { $page }', { page: h.page }) : ''}`;
      li.title = provenance(h.prov);
      $('sources').append(li);
    });
  } catch (e) { $('answer').textContent = errorText(e); } finally { off(); }
}
$('ask-go').onclick = ask;
$('question').addEventListener('keydown', (e) => { if (e.key === 'Enter') ask(); });
const keysText = (have) => { $('key-status').textContent = have.length ? t('ai-keys-set', 'Keys set: { $list }', { list: have.join(', ') }) : t('ai-keys-none', 'No keys set.'); };
$('key-save').onclick = async () => {
  try { keysText(await invoke('set_ai_key', { provider: $('key-provider').value, key: $('key').value })); $('key').value = ''; }
  catch (e) { $('key-status').textContent = errorText(e); }
};

$('mcp-claude').onclick = async () => {
  try { $('mcp-status').textContent = t('mcp-registered', 'Claude Desktop: { $result }. Restart Claude Desktop to load it.', { result: await invoke('mcp_register_claude') }); }
  catch (e) { $('mcp-status').textContent = errorText(e); }
};

$('model-go').onclick = async () => {
  const off = await listen('model-progress', (e) => { $('model-status').textContent = t('model-progress', 'Downloading: { $progress }', { progress: e.payload }); });
  try { $('model-status').textContent = t('model-ready', 'Ready: { $path }', { path: await invoke('download_model', {}) }); }
  catch (e) { $('model-status').textContent = errorText(e); } finally { off(); }
};

// Speech (feature `tts`): reads the last answer aloud.
let playing = null;
$('speak-go').onclick = async () => {
  if (playing) playing.pause();
  try { const r = await invoke('speak', { text: $('answer').textContent }); playing = new Audio(`data:audio/wav;base64,${r.wav}`); await playing.play(); }
  catch (e) { $('voice-status').textContent = errorText(e); }
};
$('voice-go').onclick = async () => {
  const off = await listen('voice-progress', (e) => { $('voice-status').textContent = t('download-progress', 'Downloading { $progress }', { progress: e.payload }); });
  try { await invoke('download_voice'); $('voice-status').textContent = t('voice-ready', 'Voice ready.'); }
  catch (e) { $('voice-status').textContent = errorText(e); } finally { off(); }
};

// Dictation (feature `stt`): Whisper turns the microphone or an audio file into the query.
// Turn it on and pick the model in the settings JSON (`stt.enabled`, `stt.model`).
async function toPcm16(blob) {
  const ctx = new AudioContext();
  let decoded;
  try { decoded = await ctx.decodeAudioData(await blob.arrayBuffer()); } finally { ctx.close(); }
  const off = new OfflineAudioContext(1, Math.max(1, Math.ceil(decoded.duration * 16000)), 16000);
  const src = off.createBufferSource();
  src.buffer = decoded; src.connect(off.destination); src.start();
  const mono = (await off.startRendering()).getChannelData(0);
  const pcm = new Int16Array(mono.length);
  for (let i = 0; i < mono.length; i++) pcm[i] = Math.max(-1, Math.min(1, mono[i])) * 0x7fff;
  const bytes = new Uint8Array(pcm.buffer);
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}
async function transcribe(blob) {
  say('stt-status', t('dictate-transcribing', 'Transcribing…'));
  try {
    const r = await invoke('transcribe', { audio: await toPcm16(blob) });
    $('query').value = r.text;
    say('stt-status', r.text ? `${r.language}, ${r.seconds.toFixed(1)} s` : t('dictate-nothing', 'No speech heard.'));
  } catch (e) { say('stt-status', errorText(e), true); }
}
let recorder = null;
$('dictate').onclick = async () => {
  if (recorder) { recorder.stop(); recorder = null; $('dictate').textContent = '🎤'; return; }
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    const chunks = [];
    const r = new MediaRecorder(stream);
    r.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    r.onstop = () => { stream.getTracks().forEach((t) => t.stop()); transcribe(new Blob(chunks, { type: r.mimeType })); };
    r.start(); recorder = r; $('dictate').textContent = t('dictate-stop', '■ Stop'); say('stt-status', t('dictate-listening', 'Listening…'));
  } catch (e) { say('stt-status', t('dictate-mic-error', 'Microphone: { $error }', { error: e.message || e }), true); }
};
$('stt-file-go').onclick = () => $('stt-file').click();
$('stt-file').onchange = (e) => { const f = e.target.files[0]; e.target.value = ''; if (f) transcribe(f); };
$('stt-download').onclick = async () => {
  const off = await listen('stt-progress', (e) => say('stt-status', t('download-progress', 'Downloading { $progress }', { progress: e.payload })));
  try { await invoke('download_stt'); say('stt-status', t('stt-ready', 'Model ready.')); }
  catch (e) { say('stt-status', errorText(e), true); } finally { off(); }
};
