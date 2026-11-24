// A stand-in for the Rust side, so the UI runs in a plain browser: `npm run dev`, then open
// http://localhost:5173 (and /settings.html for the Settings window). main.jsx and
// settings-main.jsx load it only in development and only outside Tauri, so the app itself
// never contains it. `VITE_DEV_MOCK=1 npm run build && npx vite preview` builds it in too,
// for screenshots and browser tests of the built pages.
//
// Every invoke() lands in `commands` below, with a small in-memory state, and events work
// (listen and emit). A command without a mock rejects, as a real command does, with
// { code: 'not_found', message } saying so: add it here
// when you add a Tauri command. Each call is logged to the console as `[dev-mock]`.
//
// URL options: ?mode=light|dark (default: the OS), ?theme=nord (any theme in shell.css),
// ?os=macos|windows|linux, ?wizard=1 (start at the setup wizard).
// (No double curly braces in this folder: the template engine would eat them.)
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';

// What this app was generated with (the same keys as features() in src-tauri/src/lib.rs).
const FEATURES = {
  mobile: false,
  appShell: {% if app_shell %}true{% else %}false{% endif %},
  embed: {% if embeddings %}true{% else %}false{% endif %},
  postgres: {% if postgres %}true{% else %}false{% endif %},
  bm25: {% if postgres %}{% if bm25 %}true{% else %}false{% endif %}{% else %}false{% endif %},
  watch: {% if watch_folders %}true{% else %}false{% endif %},
  ai: {% if ai %}true{% else %}false{% endif %},
  llama: {% if ai %}{% if local_llm %}true{% else %}false{% endif %}{% else %}false{% endif %},
  tts: {% if tts %}true{% else %}false{% endif %},
  stt: {% if stt %}true{% else %}false{% endif %},
  mail: {% if email %}true{% else %}false{% endif %},
  plugins: {% if plugins %}true{% else %}false{% endif %},
  jobs: {% if background_jobs %}true{% else %}false{% endif %},
  rclone: {% if rclone %}true{% else %}false{% endif %},
  tailscale: {% if tailscale %}true{% else %}false{% endif %},
  oauth: {% if oauth %}true{% else %}false{% endif %},
  oauthDeepLink: false,
  webview: {% if webview %}true{% else %}false{% endif %},
  httpApi: {% if http_api %}true{% else %}false{% endif %},
  telemetry: {% if telemetry %}true{% else %}false{% endif %},
  mcp: {% if mcp_server %}true{% else %}false{% endif %},
  pdf: {% if pdf %}true{% else %}false{% endif %},
  wizard: {% if wizard %}true{% else %}false{% endif %},
  updater: {% if updater %}true{% else %}false{% endif %},
  ipcWorker: {% if ipc_worker %}true{% else %}false{% endif %},
  backgroundAgent: {% if ipc_worker %}{% if background_agent %}true{% else %}false{% endif %}{% else %}false{% endif %},
  daemon: {% if ipc_worker %}{% if daemon %}true{% else %}false{% endif %}{% else %}false{% endif %},
  pythonExample: {% if python_example %}true{% else %}false{% endif %},
  claudeMcp: {% if claude_mcp %}true{% else %}false{% endif %},
  trayMenu: {% if tray_menu %}true{% else %}false{% endif %},
};

const params = new URLSearchParams(location.search);
const clone = (v) => JSON.parse(JSON.stringify(v));

// The user's settings (desktop-core's Config, camelCase), with the defaults the Rust side has.
const DEFAULTS = {
  search: {
    mode: 'auto',
    topK: 10,
    candidates: 50,
    minVectorScore: null,
    minKeywordScore: null,
    minScore: null,
    rrfK: 60,
    vectorWeight: 1,
    keywordWeight: 1,
    keywordEngine: 'auto',
    bm25Tokenizer: 'bert_base_uncased',
    fulltextLanguage: 'english',
  },
  ingest: {
    workers: 1,
    chunkChars: 800,
    chunkOverlap: 0,
    embedBatch: 64,
    maxFileMb: 64,
    extensions: ['txt', 'md', 'pdf', 'csv', 'json', 'html'],
    pdf: { isolate: true, timeoutSecs: 120, memoryMb: 2048 },
  },
  watch: { enabled: false, dropFolder: true, dropFolderPath: null, paths: [], recursive: true, debounceMs: 2000, ignore: ['.*', '*.tmp'] },
  resources: { cpuPercent: 100, backgroundPriority: true, resumeInterrupted: true, memoryBudgetMb: 0, gpuBudget: false, gpuLayers: 0 },
  log: { level: 'info', file: true, maxMb: 10, keep: 5 },
  net: { offline: false, allow: [], deny: [], proxy: '', noProxy: [], caFiles: [], pins: {}, pinsOff: false },
  backup: { intervalHours: 0, keep: 5, folder: null, encrypt: false },
  appearance: { mode: 'system', theme: 'kitchen', uiFont: '', monoFont: '', fontSize: 15 },
  oauth: { enabled: false, providers: [], redirect: 'loopback', timeoutSecs: 300, refreshBeforeSecs: 120 },
  jobs: { enabled: false, workers: 2, maxAttempts: 5, backoffBaseSecs: 10, backoffMaxSecs: 3600, historyDays: 7, queues: {} },
  httpApi: { enabled: false, host: '127.0.0.1', port: 0, corsOrigins: [], maxBodyKb: 1024, timeoutSecs: 30 },
  webview: {
    enabled: false,
    allow: [],
    blocked: 'refuse',
    isolation: 'profile',
    allowHttp: false,
    maxWindows: 8,
    bridge: false,
    bridgeHosts: [],
  },
  tray: {
    enabled: false,
    items: ['show_hide', 'search', 'pause_resume', 'settings', 'quit'].map((id) => ({
      id,
      label: '',
      action: id,
      shortcutHint: null,
      enabled: true,
    })),
    showInMenuBarOnly: false,
    closeToTray: false,
    leftClickOpens: true,
  },
  shortcuts: [],
  menu: { title: 'Actions', extra: [] },
};

// Sample documents, so search has something to find.
const docs = [
  { source: 'welcome.md', text: 'Welcome. This page runs in a plain browser with src/dev-mock.js standing in for the Rust side.' },
  { source: 'notes/kitchen.md', text: 'Kitchen apps are Tauri apps: a React UI, a Rust backend, and invoke() between them.' },
  { source: 'notes/search.md', text: 'Search ranks passages by the words they share with the query, best first.' },
];
const state = {
  config: clone(DEFAULTS),
  settings: { setupDone: !params.has('wizard'), backend: 'file', embeddings: FEATURES.embed },
  profiles: [{ id: 'default', name: 'Default' }],
  secrets: {},
  signedIn: {},
  jobs: [],
  history: [],
};

const status = () => ({ chunks: docs.length, backend: state.settings.backend, embedder: FEATURES.embed ? 'mock embeddings' : null });
const ingest = (source, text) => {
  docs.push({ source, text });
  return 1;
};

function appearance() {
  const a = { ...DEFAULTS.appearance, ...state.config.appearance };
  if (params.get('mode')) a.mode = params.get('mode');
  if (params.get('theme')) a.theme = params.get('theme');
  const stack = (font, system) => (font ? `'${font}', ${system}` : system);
  return {
    ...a,
    uiStack: stack(a.uiFont, "system-ui, -apple-system, 'Segoe UI', sans-serif"),
    monoStack: stack(a.monoFont, 'ui-monospace, Menlo, Consolas, monospace'),
  };
}

const commands = {
  app_info: () => ({
    name: document.title.replace(/ Settings$/, '') || 'App',
    version: '0.1.0',
    features: FEATURES,
    postgresAvailable: FEATURES.postgres,
    settings: state.settings,
    status: status(),
    dataDir: '/home/you/.local/share/app',
    profile: 'default',
    config: { env: 'development', ui: { showEnvBadge: true } },
  }),
  run_setup: ({ backend, embeddings }) => {
    Object.assign(state.settings, { setupDone: true, backend, embeddings });
    setTimeout(() => emit('setup-done', { ok: true, status: status() }), 300);
    return null;
  },
  get_config: () => clone(state.config),
  set_config: ({ config }) => {
    state.config = Object.keys(config).length ? clone(config) : clone(DEFAULTS);
    if (window.__appearance) {
      window.__appearance.apply(appearance());
      emit('appearance', appearance());
    }
    return clone(state.config);
  },
  get_appearance: () => appearance(),
  open_settings: () => {
    window.open('/settings.html', 'settings', 'width=760,height=680');
    return null;
  },
  check_update: () => ({ available: false }),
  search: ({ query }) => {
    const words = query.toLowerCase().split(/\W+/).filter(Boolean);
    return docs
      .map((d) => ({ ...d, score: words.filter((w) => d.text.toLowerCase().includes(w)).length / Math.max(1, words.length) }))
      .filter((d) => d.score > 0)
      .sort((a, b) => b.score - a.score)
      .map((d) => ({ ...d, prov: { extractor: 'text' } }));
  },
  ingest_text: ({ source, text }) => ingest(source, text),
  ingest_file: ({ path }) => ingest(path, `The contents of ${path} (mocked).`),
  ingest_folder: ({ path }) => ({ files: 1, chunks: ingest(path, `A file in ${path} (mocked).`), errors: [] }),
  clear: () => {
    docs.length = 0;
    return status();
  },
  list_profiles: () => ({ running: 'default', saved: { profiles: state.profiles } }),
  create_profile: ({ name }) => {
    const p = { id: name.toLowerCase().replace(/\W+/g, '-'), name };
    state.profiles.push(p);
    return p;
  },
  switch_profile: () => null,
  remove_profile: ({ id }) => {
    state.profiles = state.profiles.filter((p) => p.id !== id);
    return null;
  },
  list_backups: () => [],
  backup_now: () => ({ chunks: docs.length }),
  backup_recovery_key: () => ({ key: 'MOCK-RECOVERY-KEY' }),
  agent_status: () => ({ installed: false, kind: 'mock agent', location: '' }),
  set_agent: ({ enabled }) => ({ installed: enabled, kind: 'mock agent', location: '~/.config/autostart' }),
  python_call: ({ params: p }) => ({ words: p.text.split(/\s+/).filter(Boolean).length }),
  watch_status: () => ({ watching: [], errors: [] }),
  telemetry_status: () => ({ enabled: false }),
  oauth_status: () => ({
    enabled: state.config.oauth.enabled,
    providers: state.config.oauth.providers.map((p) => ({
      id: p.id,
      ready: !!p.clientId,
      signedIn: !!state.signedIn[p.id],
      clientSecret: !!state.secrets[p.id],
      user: state.signedIn[p.id] ? { email: 'you@example.com' } : null,
    })),
  }),
  oauth_login: ({ provider }) => {
    state.signedIn[provider] = true;
    return { signedIn: true };
  },
  oauth_logout: ({ provider }) => {
    delete state.signedIn[provider];
    return { revoked: true };
  },
  oauth_client_secret: ({ provider, secret }) => {
    state.secrets[provider] = !!secret;
    return { set: !!secret };
  },
  jobs_list: () => ({ enabled: state.config.jobs.enabled, types: ['example'], jobs: state.jobs, dead: [], history: state.history }),
  jobs_schedules: () => [],
  jobs_enqueue: ({ kind }) => {
    state.history.unshift({ finished: Date.now(), kind, attempt: 1, outcome: 'done', message: '' });
    return { kind };
  },
  jobs_cancel: () => null,
  jobs_retry: () => null,
  jobs_set_schedule: () => null,
  http_api_info: () =>
    state.config.httpApi.enabled
      ? { running: true, url: 'http://127.0.0.1:47011', token: 'mock-token-0123456789' }
      : { running: false, url: null, token: null },
  webview_open: ({ url }) => {
    window.open(url, '_blank', 'noopener');
    return 'webview-settings';
  },
  webview_close: () => null,
  tray_status: () => ({
    actions: [
      { id: 'show_hide', label: 'Show or hide the window' },
      { id: 'search', label: 'Search' },
      { id: 'pause_resume', label: 'Pause background work' },
      { id: 'settings', label: 'Settings' },
      { id: 'quit', label: 'Quit' },
    ],
    problems: [],
    shortcuts: [],
  }),
  run_action: () => null,
};

// Reject the way a real command does: with a CommandError, { code, message, details? }
// (src-tauri/src/error.rs). A mock that should fail calls this, e.g. fail('invalid', 'give a name').
function fail(code, message, details) {
  throw details === undefined ? { code, message } : { code, message, details };
}

// Window calls (titlebar buttons, size events) do nothing in a browser.
const quiet = (cmd) => cmd.startsWith('plugin:window|') || cmd.startsWith('plugin:webview|') || cmd.startsWith('plugin:app|');

mockWindows(location.pathname.endsWith('settings.html') ? 'settings' : 'main');
mockIPC(
  async (cmd, args) => {
    console.debug('[dev-mock]', cmd, args);
    if (commands[cmd]) return commands[cmd](args || {});
    if (quiet(cmd)) return cmd.endsWith('|is_maximized') ? false : null;
    fail('not_found', `dev-mock: no mock for "${cmd}"; add one to src/dev-mock.js`);
  },
  { shouldMockEvents: true },
);

// What the window script from src-tauri/src/shell.rs does before the page loads (app shell).
if (FEATURES.appShell) {
  const root = document.documentElement;
  const dark = window.matchMedia('(prefers-color-scheme: dark)');
  const ua = navigator.userAgent;
  const os = params.get('os') || (ua.includes('Mac') ? 'macos' : ua.includes('Windows') ? 'windows' : 'linux');
  const apply = (a) => {
    window.__appearance.current = a;
    root.dataset.os = os;
    root.dataset.theme = a.theme;
    root.dataset.mode = a.mode;
    root.dataset.scheme = a.mode === 'system' ? (dark.matches ? 'dark' : 'light') : a.mode;
    root.style.colorScheme = root.dataset.scheme;
    root.style.setProperty('--font-ui', a.uiStack);
    root.style.setProperty('--font-mono', a.monoStack);
    root.style.setProperty('--font-size', `${a.fontSize}px`);
  };
  window.__appearance = { apply, current: null, priority: 0 };
  apply(appearance());
  dark.addEventListener('change', () => apply(window.__appearance.current));
}
