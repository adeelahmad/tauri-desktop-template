// Plain JS, no build step: the Tauri API comes from `withGlobalTauri`.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const show = (id, on = true) => { $(id).hidden = !on; };
const say = (id, text, error = false) => { $(id).textContent = text; $(id).classList.toggle('error', error); };

let info;

async function start() {
  info = await invoke('app_info');
  const f = info.features;
  $('about').textContent = `${info.name} ${info.version}`;
  show('check-update', f.updater);
  show('claude', f.claudeMcp);
  show('python', f.pythonExample);
  if (f.backgroundAgent) { show('agent'); refreshAgent(); }
  if (f.ai) { show('ask'); invoke('ai_keys').then(keysText).catch(() => {}); }
  show('w-pg-option', f.postgres);
  if (f.postgres && !info.postgresAvailable) {
    $('w-pg-option').querySelector('input').disabled = true;
    say('w-pg-note', 'This computer\'s processor cannot run the bundled database; the file store works the same.');
    show('w-pg-note');
  }
  show('w-model', f.embed);
  if (f.wizard && !info.settings.setupDone) {
    show('loading', false);
    show('wizard');
  } else {
    setup(info.settings.backend, info.settings.embeddings, 'loading');
  }
  if (f.updater) checkUpdate(true);
}

// Opens the store and model; progress lines arrive as events.
function setup(backend, embeddings, statusArea) {
  const statusId = statusArea === 'loading' ? null : 'w-status';
  const line = (t, e) => statusId ? say(statusId, t, e) : ($('loading').firstElementChild.textContent = t);
  const offs = [];
  listen('setup-progress', (e) => line(`${e.payload}…`)).then((u) => offs.push(u));
  listen('setup-done', (e) => {
    offs.forEach((u) => u());
    show('w-progress', false);
    $('w-go').disabled = false;
    if (!e.payload.ok) return line(`Setup failed: ${e.payload.error}`, true);
    show('wizard', false); show('loading', false); show('main'); show('clear'); loadConfig().catch(() => {});
    const s = e.payload.status;
    $('about').textContent = `${info.name} ${info.version} · ${s.chunks} chunks · ${s.backend} store` +
      (s.embedder ? ` · ${s.embedder}` : ' · keyword search');
  }).then((u) => offs.push(u));
  invoke('run_setup', { backend, embeddings }).catch((e) => line(String(e), true));
}

$('w-go').onclick = () => {
  $('w-go').disabled = true;
  show('w-progress');
  const backend = document.querySelector('input[name=backend]:checked').value;
  setup(backend, $('w-embed').checked, 'wizard');
};

async function refreshAbout() {
  const i = await invoke('app_info');
  if (i.status) $('about').textContent = `${i.name} ${i.version} · ${i.status.chunks} chunks · ${i.status.backend} store` +
    (i.status.embedder ? ` · ${i.status.embedder}` : ' · keyword search');
}

async function addFile(path) {
  say('add-status', `Reading ${path}…`);
  try {
    const n = await invoke('ingest_file', { path });
    say('add-status', `Added ${n} chunks from ${path}.`);
    refreshAbout();
  } catch (e) { say('add-status', String(e), true); }
}

$('add-file').onclick = () => { const p = $('file-path').value.trim(); if (p) addFile(p); };
$('add-folder').onclick = async () => {
  const path = $('file-path').value.trim();
  if (!path) return;
  say('add-status', `Reading every document in ${path}…`);
  try {
    const r = await invoke('ingest_folder', { path, recursive: true });
    say('add-status', `Added ${r.files} files (${r.chunks} chunks)${r.errors.length ? `, ${r.errors.length} failed: ${r.errors[0]}` : ''}.`, r.errors.length > 0);
    refreshAbout();
  } catch (e) { say('add-status', String(e), true); }
};

// ---- settings (desktop-core Config) ----
async function loadConfig() { $('config').value = JSON.stringify(await invoke('get_config'), null, 2); }
async function saveConfig(config) {
  try {
    $('config').value = JSON.stringify(await invoke('set_config', { config }), null, 2);
    say('config-status', 'Saved.');
  } catch (e) { say('config-status', `Not saved: ${e}`, true); }
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
    say('add-status', `Added ${n} chunks.`);
    $('paste').value = '';
    refreshAbout();
  } catch (e) { say('add-status', String(e), true); }
};
listen('tauri://drag-drop', async (e) => { for (const p of e.payload.paths || []) await addFile(p); });

async function runSearch() {
  const query = $('query').value.trim();
  if (!query) return;
  const list = $('results');
  list.innerHTML = '';
  try {
    const hits = await invoke('search', { query });
    if (!hits.length) list.innerHTML = '<li class="meta">Nothing found.</li>';
    for (const h of hits) {
      const li = document.createElement('li');
      const meta = document.createElement('div');
      meta.className = 'meta';
      meta.textContent = `${h.source}${h.page ? `, page ${h.page}` : ''} · ${h.score.toFixed(3)}` +
        (h.vector_score != null ? ` · vector ${h.vector_score.toFixed(3)}` : '') + (h.keyword_score != null ? ` · keyword ${h.keyword_score.toFixed(2)}` : '');
      const body = document.createElement('div');
      body.textContent = h.text;
      li.append(meta, body);
      list.append(li);
    }
  } catch (e) { list.innerHTML = ''; const li = document.createElement('li'); li.className = 'status error'; li.textContent = String(e); list.append(li); }
}
$('search').onclick = runSearch;
$('query').addEventListener('keydown', (e) => { if (e.key === 'Enter') runSearch(); });

$('clear').onclick = async () => {
  if ($('clear').dataset.armed !== '1') { $('clear').dataset.armed = '1'; $('clear').textContent = 'Click again to clear everything'; return; }
  $('clear').dataset.armed = ''; $('clear').textContent = 'Clear all data';
  await invoke('clear');
  refreshAbout();
};

// ---- updates (feature `updater`) ----
async function checkUpdate(quiet) {
  try {
    const u = await invoke('check_update');
    if (!u.configured) { if (!quiet) say('add-status', 'Updates are not set up in this build (no updater public key).'); return; }
    if (u.available) {
      $('update-text').textContent = `Version ${u.version} is available (you have ${u.current}).`;
      show('update-banner');
    } else if (!quiet) say('add-status', `You have the newest version (${u.current}).`);
  } catch (e) { if (!quiet) say('add-status', `Update check failed: ${e}`, true); }
}
$('check-update').onclick = () => checkUpdate(false);
$('update-install').onclick = async () => {
  $('update-install').disabled = true;
  listen('update-progress', (e) => {
    const [done, total] = e.payload;
    $('update-text').textContent = total ? `Downloading… ${Math.round((100 * done) / total)}%` : 'Downloading…';
  });
  try { await invoke('install_update'); } catch (e) { $('update-text').textContent = `Update failed: ${e}`; $('update-install').disabled = false; }
};

// ---- background agent (feature `background-agent`) ----
async function refreshAgent(set) {
  try {
    const a = set === undefined ? await invoke('agent_status') : await invoke('set_agent', { enabled: set });
    $('agent-on').checked = a.installed;
    say('agent-status', a.installed ? `On: ${a.kind} at ${a.location}` : `Off (${a.kind})`);
  } catch (e) { say('agent-status', String(e), true); }
}
$('agent-on').onchange = () => refreshAgent($('agent-on').checked);

// ---- Python example (feature `python-example`) ----
$('python-go').onclick = async () => {
  try {
    const r = await invoke('python_call', { method: 'stats', params: { text: $('paste').value || 'Type some text in the box above.' } });
    $('python-out').textContent = JSON.stringify(r, null, 2);
  } catch (e) { say('python-out', String(e), true); }
};

// ---- Claude Desktop (feature `claude-mcp`) ----
$('claude-go').onclick = async () => {
  $('claude-go').disabled = true;
  try {
    const r = await invoke('register_claude');
    say('claude-status', `Claude Desktop config ${r.status}${r.backup ? ` (backup: ${r.backup})` : ''}. Preparing the tools once…`);
    listen('warmup-done', (e) => say('claude-status', e.payload ? 'Ready. Restart Claude Desktop to use it.' : 'Connected, but the first start failed; it will retry when Claude starts.', !e.payload));
  } catch (e) { say('claude-status', String(e), true); }
  $('claude-go').disabled = false;
};

start().catch((e) => say('loading', String(e), true));

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
      li.textContent = `[${i + 1}] ${h.source}${h.page ? `, page ${h.page}` : ''}`;
      $('sources').append(li);
    });
  } catch (e) { $('answer').textContent = String(e); } finally { off(); }
}
$('ask-go').onclick = ask;
$('question').addEventListener('keydown', (e) => { if (e.key === 'Enter') ask(); });
const keysText = (have) => { $('key-status').textContent = have.length ? `Keys set: ${have.join(', ')}` : 'No keys set.'; };
$('key-save').onclick = async () => {
  try { keysText(await invoke('set_ai_key', { provider: $('key-provider').value, key: $('key').value })); $('key').value = ''; }
  catch (e) { $('key-status').textContent = String(e); }
};
