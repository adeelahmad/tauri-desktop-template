// Settings for search, ingest and folder watching (desktop-core's Config).
// Every optional behaviour is off until it is switched on here.
// (No double curly braces in this folder: the template engine would eat them.)
import { useEffect, useState } from 'react';
import { api, errorText, invoke } from './ipc.js';
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
  [
    'postgres',
    'upgrade',
    'When a new version brings a newer Postgres',
    'select',
    { options: ['ask', 'auto', 'off'], labels: ['Ask first', 'Upgrade at start', 'Keep the old version'], needs: 'postgres' },
  ],
  ['postgres', 'upgradeCopy', 'Upgrade by copying (slower, twice the disk space)', 'bool', { needs: 'postgres' }],
  ['ingest', 'workers', 'Ingest workers (0 = one per core)', 'number', { min: 0, max: 64 }],
  ['ingest', 'chunkChars', 'Chunk size (characters)', 'number', { min: 100, max: 20000 }],
  ['ingest', 'chunkOverlap', 'Chunk overlap (characters)', 'number', { min: 0 }],
  ['ingest', 'embedBatch', 'Chunks per embedding call', 'number', { min: 1, max: 1024 }],
  ['ingest', 'maxFileMb', 'Largest file (MB)', 'number', { min: 1 }],
  ['ingest', 'extensions', 'File types', 'list', {}],
  ['ingest', 'pdf.isolate', 'Read PDFs in a separate, limited process', 'bool', { needs: 'pdf' }],
  ['ingest', 'pdf.timeoutSecs', 'PDF time limit (seconds)', 'number', { needs: 'pdf', min: 5, max: 3600 }],
  ['ingest', 'pdf.memoryMb', 'PDF memory limit (MB)', 'number', { needs: 'pdf', min: 256, max: 65536 }],
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
  ['llm', 'temperature', 'Temperature (empty = model default)', 'optional', { needs: 'ai', step: 0.1, min: 0, max: 2 }],
  ['llm', 'maxTokens', 'Longest answer (tokens)', 'number', { needs: 'ai', min: 1 }],
  ['llm', 'timeoutSecs', 'Timeout (seconds)', 'number', { needs: 'ai', min: 5 }],
  ['llm', 'retries', 'Retries before the next provider', 'number', { needs: 'ai', min: 0, max: 10 }],
  ['llm', 'retryBaseMs', 'First retry wait (ms, doubles each retry)', 'number', { needs: 'ai', min: 50, max: 60000 }],
  [
    'llm',
    'retryMaxSecs',
    'Longest retry wait (s; a longer Retry-After moves to the next provider)',
    'number',
    { needs: 'ai', min: 1, max: 600 },
  ],
  ['llm', 'dailyTokenBudget', 'Daily token budget', 'optional', { needs: 'ai', min: 0 }],
  ['llm', 'monthlyTokenBudget', 'Monthly token budget', 'optional', { needs: 'ai', min: 0 }],
  ['llm', 'budget.dailyUsd', 'Daily budget (USD)', 'optional', { needs: 'ai', min: 0, step: 0.01 }],
  ['llm', 'budget.monthlyUsd', 'Monthly budget (USD)', 'optional', { needs: 'ai', min: 0, step: 0.01 }],
  ['llm', 'budget.warnPercent', 'Warn at (% of a budget; 0 = never)', 'number', { needs: 'ai', min: 0, max: 100 }],
  ['llm', 'budget.overrideLimits', 'Keep going over budget (override)', 'bool', { needs: 'ai' }],
  ['llm', 'promptCache', 'Prompt caching (Anthropic cache_control; OpenAI and Gemini cache on their own)', 'bool', { needs: 'ai' }],
  ['llm', 'systemPrompt', 'System prompt (empty = built in)', 'text', { needs: 'ai', nullable: true }],
  ['llm', 'httpHosts', 'Hosts allowed over plain HTTP', 'list', { needs: 'ai' }],
  ['llm', 'cache.enabled', 'Reuse answers to repeated questions (cleared when documents change)', 'bool', { needs: 'ai' }],
  ['llm', 'cache.ttlHours', 'Keep cached answers for (hours)', 'number', { needs: 'ai', min: 1 }],
  ['llm', 'cache.maxEntries', 'Most cached answers', 'number', { needs: 'ai', min: 1 }],
  ['llm', 'cache.similarity', 'Similar question match (cosine, 1 = exact only)', 'number', { needs: 'ai', step: 0.01, min: 0.5, max: 1 }],
  ['mcp', 'enabled', 'Let MCP clients (Claude Desktop, Claude Code, ...) search these documents', 'bool', { needs: 'mcp' }],
  ['mcp', 'allowWrite', 'Also allow tools that change data (add, remove)', 'bool', { needs: 'mcp' }],
  ['mcp', 'tools', 'Only these tools (empty = all allowed)', 'list', { needs: 'mcp' }],
  ['mcp', 'auditLog', 'Keep an audit log of tool calls', 'bool', { needs: 'mcp' }],
  ['mcp', 'auditArguments', 'Log the call arguments too', 'bool', { needs: 'mcp' }],
  ['mcp', 'maxResultChars', 'Longest reply (characters)', 'number', { needs: 'mcp', min: 1000 }],
  ['mcp', 'approvalTimeoutSecs', 'Wait for approval of "ask" tools (seconds)', 'number', { needs: 'mcp', min: 5, max: 600 }],
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
  ['resources', 'cpuPercent', 'CPU share for indexing (%)', 'number', { min: 10, max: 100 }],
  ['resources', 'backgroundPriority', 'Index at background priority (keeps the computer responsive)', 'bool', {}],
  ['resources', 'resumeInterrupted', 'Finish an interrupted import at the next start', 'bool', {}],
  ['resources', 'memoryBudgetMb', 'Memory budget for indexing (MB, 0 = none)', 'number', { min: 0 }],
  ['resources', 'gpuBudget', 'Limit the GPU for the local model', 'bool', { needs: 'llama' }],
  ['resources', 'gpuLayers', 'GPU layers while limited (0 = CPU only)', 'number', { needs: 'llama', min: 0 }],
  [
    'resources',
    'onnxProvider',
    'ONNX accelerator (falls back to the CPU)',
    'select',
    {
      needs: 'embed',
      options: ['cpu', 'auto', 'coreml', 'directml', 'cuda'],
      labels: ['CPU', 'Best available', 'Core ML (macOS)', 'DirectML (Windows)', 'CUDA (NVIDIA)'],
    },
  ],
  ['backup', 'intervalHours', 'Back up every (hours; 0 = only when asked)', 'number', { min: 0 }],
  ['backup', 'keep', 'Backups to keep (0 = all)', 'number', { min: 0 }],
  ['backup', 'encrypt', 'Encrypt backups (keep the recovery key below somewhere safe)', 'bool', {}],
  ['backup', 'folder', 'Backup folder (empty = app data; a synced folder keeps a copy off this computer)', 'text', { nullable: true }],
  ['net', 'offline', 'Offline mode (nothing leaves this computer)', 'bool', {}],
  ['net', 'allow', 'Only these hosts (*.example.com for subdomains; empty = any)', 'list', {}],
  ['net', 'deny', 'Never these hosts', 'list', {}],
  ['net', 'proxy', 'Proxy (empty = system, direct = none, or http:// / socks5:// URL)', 'text', {}],
  ['net', 'noProxy', 'Hosts that skip the proxy', 'list', {}],
  ['net', 'caFiles', 'Trust only these CA certificate files (PEM), e.g. a company proxy', 'list', { lines: true }],
  ['net', 'pinsOff', 'Ignore certificate pins (behind a TLS-inspecting proxy)', 'bool', {}],
  ['log', 'level', 'Log level', 'select', { options: ['error', 'warn', 'info', 'debug', 'trace'] }],
  ['log', 'file', 'Write a log file (logs folder in the app data)', 'bool', {}],
  ['log', 'maxMb', 'Start a new file at (MB)', 'number', { min: 1, max: 1024 }],
  ['log', 'keep', 'Old log files to keep', 'number', { min: 0, max: 50 }],
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
  ['mail', 'enabled', 'Send email (SMTP)', 'bool', { needs: 'mail' }],
  ['mail', 'host', 'Mail server, e.g. smtp.gmail.com', 'text', { needs: 'mail' }],
  ['mail', 'port', 'Port (465 for SSL/TLS, 587 for STARTTLS)', 'number', { needs: 'mail', min: 1, max: 65535 }],
  [
    'mail',
    'security',
    'Encryption',
    'select',
    { needs: 'mail', options: ['tls', 'starttls', 'none'], labels: ['SSL/TLS', 'STARTTLS', 'None (this computer only)'] },
  ],
  ['mail', 'username', 'Login name (empty = no login)', 'text', { needs: 'mail' }],
  ['mail', 'from', 'Sender, e.g. Me <me@example.com>', 'text', { needs: 'mail' }],
  ['mail', 'maxMb', 'Largest message (MB)', 'number', { needs: 'mail', min: 1, max: 100 }],
  ['mail', 'timeoutSecs', 'Timeout (seconds)', 'number', { needs: 'mail', min: 5, max: 300 }],
  ['oauth', 'enabled', 'Allow signing in (OAuth 2 / OpenID Connect in the browser)', 'bool', { needs: 'oauth' }],
  [
    'oauth',
    'redirect',
    'The browser comes back through',
    'select',
    {
      needs: 'oauth',
      options: ['loopback', 'deepLink'],
      labels: ['127.0.0.1 (works everywhere)', "The app's deep link (needs the deep-link plugin)"],
    },
  ],
  ['oauth', 'timeoutSecs', 'Time to finish signing in (seconds)', 'number', { needs: 'oauth', min: 30, max: 1800 }],
  ['oauth', 'refreshBeforeSecs', 'Refresh tokens this long before they expire (seconds)', 'number', { needs: 'oauth', min: 10, max: 3600 }],
  ['jobs', 'enabled', 'Run background jobs and schedules', 'bool', { needs: 'jobs' }],
  ['jobs', 'workers', 'Jobs at once', 'number', { needs: 'jobs', min: 1, max: 32 }],
  ['jobs', 'maxAttempts', 'Attempts before a job is dead', 'number', { needs: 'jobs', min: 1, max: 100 }],
  ['jobs', 'backoffBaseSecs', 'First retry after about (seconds, doubling)', 'number', { needs: 'jobs', min: 1 }],
  ['jobs', 'backoffMaxSecs', 'Longest wait between retries (seconds)', 'number', { needs: 'jobs', min: 1 }],
  ['jobs', 'historyDays', 'Keep the run history for (days)', 'number', { needs: 'jobs', min: 1, max: 365 }],
  ['plugins', 'enabled', 'Run plugins', 'bool', { needs: 'plugins' }],
  ['plugins', 'disabled', 'Leave out these plugins (names)', 'list', { needs: 'plugins' }],
  ['rclone', 'enabled', 'Sync to the cloud with rclone', 'bool', { needs: 'rclone' }],
  ['rclone', 'remote', 'Remote (its name in the rclone config)', 'text', { needs: 'rclone' }],
  ['rclone', 'path', 'Folder on the remote (empty = its root)', 'text', { needs: 'rclone' }],
  [
    'rclone',
    'folder',
    'What to sync',
    'select',
    { needs: 'rclone', options: ['backups', 'data'], labels: ['Backups folder', 'Data folder (up only, no secrets)'] },
  ],
  [
    'rclone',
    'direction',
    'Direction',
    'select',
    {
      needs: 'rclone',
      options: ['push', 'pull', 'mirror', 'bisync'],
      labels: ['Up (copy, never delete)', 'Down (copy, never delete)', 'Mirror up (deletes extra files there)', 'Both ways (bisync)'],
    },
  ],
  ['rclone', 'intervalMinutes', 'Sync every (minutes, 0 = only when asked)', 'number', { needs: 'rclone', min: 0 }],
  ['rclone', 'bandwidth', 'Bandwidth limit, e.g. 2M or 10M:1M (empty = none)', 'text', { needs: 'rclone' }],
  ['tailscale', 'enabled', 'Use Tailscale', 'bool', { needs: 'tailscale' }],
  ['tailscale', 'serve', 'Share a local port with my tailnet (tailscale serve)', 'bool', { needs: 'tailscale' }],
  ['tailscale', 'port', 'Local port to share (on 127.0.0.1)', 'number', { needs: 'tailscale', min: 0, max: 65535 }],
  ['tailscale', 'httpsPort', 'HTTPS port on the tailnet', 'select', { needs: 'tailscale', options: [443, 8443, 10000], numeric: true }],
  ['webview', 'enabled', 'Open allowed web pages inside the app', 'bool', { needs: 'webview' }],
  ['webview', 'allow', 'Allowed hosts (example.com, or *.example.com for subdomains too)', 'list', { needs: 'webview', lines: true }],
  [
    'webview',
    'blocked',
    'Other pages',
    'select',
    { needs: 'webview', options: ['refuse', 'browser'], labels: ['Refuse them', 'Open them in the system browser'] },
  ],
  [
    'webview',
    'isolation',
    'Cookies and sign-ins',
    'select',
    { needs: 'webview', options: ['profile', 'incognito'], labels: ['Kept, separately for each profile', 'Never kept (incognito)'] },
  ],
  ['webview', 'allowHttp', 'Also show plain HTTP pages on allowed hosts', 'bool', { needs: 'webview' }],
  ['webview', 'maxWindows', 'Most web view windows open at once', 'number', { needs: 'webview', min: 1, max: 64 }],
  ['webview', 'bridge', 'Message bridge for trusted pages (window.kitchenBridge)', 'bool', { needs: 'webview' }],
  ['webview', 'bridgeHosts', 'Hosts that may use the bridge (must be allowed above)', 'list', { needs: 'webview', lines: true }],
  ['httpApi', 'enabled', 'Serve the local HTTP API', 'bool', { needs: 'httpApi' }],
  ['httpApi', 'host', 'Listen on (127.0.0.1 = this computer only)', 'text', { needs: 'httpApi' }],
  ['httpApi', 'port', 'Port (0 = any free port)', 'number', { needs: 'httpApi', min: 0, max: 65535 }],
  ['httpApi', 'corsOrigins', 'Browser origins allowed (CORS)', 'list', { needs: 'httpApi', lines: true }],
  ['httpApi', 'maxBodyKb', 'Largest request (KB)', 'number', { needs: 'httpApi', min: 1 }],
  ['httpApi', 'timeoutSecs', 'Request timeout (seconds)', 'number', { needs: 'httpApi', min: 1, max: 3600 }],
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
  [
    'appearance',
    'mode',
    'Light or dark',
    'select',
    { needs: 'appShell', options: ['system', 'light', 'dark'], labels: ['Follow the system', 'Light', 'Dark'] },
  ],
  [
    'appearance',
    'theme',
    'Theme',
    'select',
    {
      needs: 'appShell',
      options: ['kitchen', 'nord', 'solarized', 'forest', 'rose', 'contrast'],
      labels: ['Kitchen', 'Nord', 'Solarized', 'Forest', 'Rosé', 'High contrast'],
    },
  ],
  [
    'appearance',
    'uiFont',
    'Interface font (empty = system; a preset or any installed or bundled font)',
    'text',
    { needs: 'appShell', suggest: ['humanist', 'rounded', 'serif'] },
  ],
  [
    'appearance',
    'monoFont',
    'Code and editor font (empty = system)',
    'text',
    { needs: 'appShell', suggest: ['cascadia', 'jetbrains', 'fira'] },
  ],
  ['appearance', 'fontSize', 'Text size (px)', 'number', { needs: 'appShell', min: 10, max: 24 }],
  ['tray', 'enabled', 'Show an icon in the tray (menu bar on macOS)', 'bool', { needs: 'trayMenu' }],
  ['tray', 'leftClickOpens', 'Left click opens the window (right click: the menu)', 'bool', { needs: 'trayMenu' }],
  ['tray', 'closeToTray', 'Closing the window keeps the app in the tray', 'bool', { needs: 'trayMenu' }],
  ['tray', 'showInMenuBarOnly', 'macOS: menu bar only, no Dock icon', 'bool', { needs: 'trayMenu' }],
  [
    'updates',
    'channel',
    'Update channel',
    'select',
    { needs: 'releaseChannels', options: ['stable', 'beta'], labels: ['Stable', 'Beta (pre-releases as well)'] },
  ],
  ['kv', 'uiNamespaces', 'Namespaces the window, HTTP API and MCP tools may use', 'list', { needs: 'kv' }],
  ['kv', 'encryptedNamespaces', 'Namespaces encrypted at rest', 'list', { needs: 'kv' }],
  [
    'kv',
    'keySource',
    'Encryption key kept in',
    'select',
    { needs: 'kv', options: ['file', 'keychain'], labels: ['A key file in the data folder', 'The OS keychain (needs sign-in support)'] },
  ],
  ['kv', 'maxSizeMb', 'Largest store (MB)', 'number', { needs: 'kv', min: 16, max: 65536 }],
  ['kv', 'maxValueKb', 'Largest value (KB)', 'number', { needs: 'kv', min: 1, max: 16384 }],
  ['kv', 'sweepMinutes', 'Delete expired keys every (minutes)', 'number', { needs: 'kv', min: 1, max: 1440 }],
  ['kv', 'backup', 'Include the store in backups', 'bool', { needs: 'kv' }],
  ['cache', 'maxMb', 'Largest cache (MB)', 'number', { needs: 'cache', min: 1, max: 4096 }],
  ['cache', 'ttlSecs', 'Keep entries for (seconds, 0 = until evicted)', 'number', { needs: 'cache', min: 0 }],
  ['cache', 'ttiSecs', 'Drop entries unused for (seconds, 0 = never)', 'number', { needs: 'cache', min: 0 }],
  ['cache', 'persist', 'Keep entries across restarts (in the key-value store)', 'bool', { needs: 'kv' }],
  // kitchen:settings (`kitchen generate setting` adds fields above this line)
];

const TITLES = {
  appearance: 'Appearance',
  search: 'Search',
  postgres: 'Database',
  ingest: 'Ingest',
  llm: 'AI',
  mcp: 'MCP server',
  tts: 'Speech',
  stt: 'Dictation',
  telemetry: 'Privacy',
  log: 'Logs',
  resources: 'Performance',
  net: 'Network',
  backup: 'Backups',
  mail: 'Email',
  oauth: 'Sign-in',
  plugins: 'Plugins',
  jobs: 'Jobs',
  rclone: 'Cloud sync',
  tailscale: 'Tailnet',
  httpApi: 'HTTP API',
  webview: 'Web views',
  watch: 'Watch folders',
  service: 'Service',
  tray: 'Tray & shortcuts',
  updates: 'Updates',
  about: 'About',
  kv: 'Key-value store',
  cache: 'Cache',
  // kitchen:titles
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
  // `suggest`: values offered in a drop-down while typing (any other text is allowed too).
  const listId = extra.suggest && `suggest-${label.replace(/\W+/g, '-')}`;
  return (
    <label className="field">
      {label}
      <input
        type={isNum ? 'number' : 'text'}
        min={extra.min}
        max={extra.max}
        step={extra.step}
        value={value ?? ''}
        onChange={change}
        list={listId}
      />
      {listId && (
        <datalist id={listId}>
          {extra.suggest.map((o) => (
            <option key={o} value={o} />
          ))}
        </datalist>
      )}
    </label>
  );
}

function WatchStatus() {
  const [st, setSt] = useState(null);
  useEffect(() => {
    const tick = () =>
      api
        .watch_status()
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

// The SMTP password (kept in an owner-only file, never in settings) and a test send.
function MailSetup({ from }) {
  const [password, setPassword] = useState('');
  const [saved, setSaved] = useState(false);
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.mail_password().then(
      (r) => setSaved(r.set),
      () => {},
    );
  }, []);
  const store = async () => {
    try {
      setSaved((await api.mail_password({ password })).set);
      setPassword('');
      setMsg('Password saved.');
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  const test = async (to) => {
    setBusy(true);
    setMsg(to ? `Sending to ${to}...` : 'Connecting...');
    try {
      await api.test_mail({ to });
      setMsg(to ? `Sent. Check ${to}.` : 'Connected and logged in.');
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const me = (from || '').match(/<([^>]+)>/)?.[1] || from;
  return (
    <>
      <div className="row">
        <input
          type="password"
          aria-label="SMTP password"
          placeholder={saved ? 'Password saved (type to replace)' : 'Password or app password'}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <button onClick={store}>{password ? 'Save password' : 'Remove password'}</button>
      </div>
      <div className="row">
        <button onClick={() => test(null)} disabled={busy}>
          Test login
        </button>
        <button onClick={() => test(me)} disabled={busy || !me}>
          Send a test email to myself
        </button>
        <span className="hint">{msg || 'Save settings first.'}</span>
      </div>
    </>
  );
}

const PROVIDER_KINDS = ['google', 'github', 'microsoft', 'oidc'];
const PROVIDER_LABELS = { google: 'Google', github: 'GitHub', microsoft: 'Microsoft', oidc: 'OpenID Connect (any provider)' };
const KIND_FIELD = { options: PROVIDER_KINDS, labels: PROVIDER_KINDS.map((k) => PROVIDER_LABELS[k]) };

// Sign-in providers (client ids in settings; client secrets and tokens in the OS keychain),
// with Sign in / Sign out for each saved provider.
function OauthSetup({ providers, onChange, deepLink, redirect }) {
  const [status, setStatus] = useState(null);
  const [secrets, setSecrets] = useState({});
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState('');
  const refresh = () => api.oauth_status().then(setStatus, (e) => setMsg(errorText(e)));
  useEffect(() => {
    refresh();
    const off = listen('oauth-changed', (e) => {
      if (e.payload?.error) setMsg(String(e.payload.error));
      refresh();
    });
    return () => {
      off.then((f) => f());
    };
  }, []);
  const put = (i, key, v) => onChange(providers.map((p, j) => (j === i ? { ...p, [key]: v } : p)));
  const add = () => {
    const kind = PROVIDER_KINDS.find((k) => !providers.some((p) => p.id === k)) || 'oidc';
    onChange([...providers, { id: kind, kind, clientId: '', scopes: [], issuer: '', tenant: 'common' }]);
  };
  const act = async (label, fn) => {
    setBusy(label);
    setMsg(label === 'signin' ? 'Finish signing in in your browser...' : '');
    try {
      const r = await fn();
      if (r?.pending) setMsg('Finish signing in in your browser; the app opens again when you are done.');
      else if (r && 'revoked' in r) setMsg(r.revoked ? 'Signed out and the token was revoked.' : 'Signed out on this computer.');
      else setMsg('Signed in.');
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      setBusy('');
      refresh();
    }
  };
  const saveSecret = async (id) => {
    try {
      const r = await api.oauth_client_secret({ provider: id, secret: secrets[id] || '' });
      setSecrets({ ...secrets, [id]: '' });
      setMsg(r.set ? 'Client secret saved in the keychain.' : 'Client secret removed.');
      refresh();
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  const live = (id) => status?.providers.find((p) => p.id === id);
  return (
    <div>
      {redirect === 'deepLink' && !deepLink && (
        <p className="hint">The deep-link plugin is not built in; the 127.0.0.1 redirect is used.</p>
      )}
      {providers.map((p, i) => {
        const st = live(p.id);
        const who = st?.user && (st.user.email || st.user.login || st.user.name || st.user.id);
        return (
          <div key={i} className="grid card">
            <Field kind="select" label="Provider" value={p.kind} extra={KIND_FIELD} onChange={(v) => put(i, 'kind', v)} />
            <Field kind="text" label="Name (used to sign in; e.g. google)" value={p.id} extra={NONE} onChange={(v) => put(i, 'id', v)} />
            <Field kind="text" label="Client id" value={p.clientId} extra={NONE} onChange={(v) => put(i, 'clientId', v)} />
            <Field
              kind="list"
              label="Scopes (empty = the provider's usual ones)"
              value={p.scopes}
              extra={NONE}
              onChange={(v) => put(i, 'scopes', v)}
            />
            {p.kind === 'oidc' && (
              <Field
                kind="text"
                label="Issuer URL, e.g. https://id.example.com"
                value={p.issuer}
                extra={NONE}
                onChange={(v) => put(i, 'issuer', v)}
              />
            )}
            {p.kind === 'microsoft' && (
              <Field
                kind="text"
                label="Tenant (common, organizations, consumers or a tenant id)"
                value={p.tenant}
                extra={NONE}
                onChange={(v) => put(i, 'tenant', v)}
              />
            )}
            <div className="row">
              <input
                type="password"
                autoComplete="off"
                placeholder={st?.clientSecret ? 'Client secret saved (type to replace)' : 'Client secret, if the provider needs one'}
                value={secrets[p.id] || ''}
                onChange={(e) => setSecrets({ ...secrets, [p.id]: e.target.value })}
                aria-label="Client secret"
              />
              <button onClick={() => saveSecret(p.id)} disabled={!st}>
                {secrets[p.id] ? 'Save secret' : 'Remove secret'}
              </button>
            </div>
            <div className="row">
              {st?.signedIn ? (
                <button onClick={() => act('signout', () => api.oauth_logout({ provider: p.id }))} disabled={!!busy}>
                  Sign out
                </button>
              ) : (
                <button
                  onClick={() => act('signin', () => api.oauth_login({ provider: p.id }))}
                  disabled={!!busy || !st?.ready || !status?.enabled}
                >
                  Sign in with {PROVIDER_LABELS[p.kind]}
                </button>
              )}
              <button className="link" onClick={() => onChange(providers.filter((_, j) => j !== i))}>
                Remove provider
              </button>
              <span className="hint">
                {st ? (st.signedIn ? `Signed in${who ? ` as ${who}` : ''}.` : 'Not signed in.') : 'Save settings to use this provider.'}
                {st?.error ? ` Keychain: ${st.error}` : ''}
              </span>
            </div>
          </div>
        );
      })}
      <div className="row">
        <button onClick={add}>Add a provider</button>
        <span className="hint">{msg || 'Turn sign-in on and save settings, then sign in.'}</span>
      </div>
    </div>
  );
}

const NONE = {};

// Each built-in plugin, with a form drawn from its settings schema (JSON for objects).
function PluginList({ settings, onChange }) {
  const [list, setList] = useState([]);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    api.plugins_list().then(setList, (e) => setMsg(errorText(e)));
  }, []);
  const put = (name, key, v) => onChange({ ...settings, [name]: { ...(settings[name] || {}), [key]: v } });
  return (
    <div className="panel">
      {list.length === 0 && !msg && <p className="hint">No plugins are built in.</p>}
      {list.map((p) => (
        <div key={p.name}>
          <div>
            <b>{p.name}</b> {p.version} {p.active ? '(on)' : '(off)'}: {p.description}
            {p.methods.length > 0 && ` Methods: ${p.methods.join(', ')}.`}
          </div>
          <div className="grid">
            {Object.entries(p.schema.properties || {}).map(([key, prop]) => {
              const value = (settings[p.name] || {})[key] ?? prop.default;
              const label = prop.title || key;
              if (prop.type === 'boolean')
                return <Field key={key} kind="bool" label={label} value={value} extra={NONE} onChange={(v) => put(p.name, key, v)} />;
              if (prop.type === 'number' || prop.type === 'integer')
                return <Field key={key} kind="optional" label={label} value={value} extra={NONE} onChange={(v) => put(p.name, key, v)} />;
              if (prop.type === 'string')
                return <Field key={key} kind="text" label={label} value={value} extra={NONE} onChange={(v) => put(p.name, key, v)} />;
              return <JsonField key={key} label={label} value={value} onChange={(v) => put(p.name, key, v)} />;
            })}
          </div>
        </div>
      ))}
      {msg}
    </div>
  );
}

function JsonField({ label, value, onChange }) {
  const [text, setText] = useState(JSON.stringify(value ?? {}, null, 2));
  const [bad, setBad] = useState(false);
  const change = (t) => {
    setText(t);
    try {
      onChange(JSON.parse(t));
      setBad(false);
    } catch {
      setBad(true);
    }
  };
  return (
    <label className="field">
      {label} {bad && '(not valid JSON yet)'}
      <textarea rows={4} value={text} onChange={(e) => change(e.target.value)} />
    </label>
  );
}

const when = (ms) => (ms ? new Date(ms).toLocaleString() : '-');

// Background jobs: the queue, the dead letters (retry or drop), schedules (pause or resume)
// and recent runs, polled while the section is open.
function JobsPanel() {
  const [st, setSt] = useState(null);
  const [schedules, setSchedules] = useState([]);
  const [kind, setKind] = useState('');
  const [payload, setPayload] = useState('{}');
  const [msg, setMsg] = useState('');
  const refresh = () => {
    api.jobs_list().then(setSt, (e) => setMsg(errorText(e)));
    api.jobs_schedules().then(setSchedules, () => {});
  };
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, []);
  const act = (cmd, args) => async () => {
    try {
      await invoke(cmd, args);
      setMsg('');
    } catch (e) {
      setMsg(errorText(e));
    }
    refresh();
  };
  const enqueue = async () => {
    try {
      const job = await api.jobs_enqueue({ kind: kind || st.types[0], payload: JSON.parse(payload || '{}') });
      setMsg(`Queued ${job.kind}.`);
    } catch (e) {
      setMsg(errorText(e));
    }
    refresh();
  };
  if (!st) return <p className="hint">{msg}</p>;
  return (
    <div className="panel">
      {!st.enabled && <div>Jobs are kept but not run until you turn them on above and save.</div>}
      <div className="row">
        <select value={kind || st.types[0]} onChange={(e) => setKind(e.target.value)} aria-label="Job type">
          {st.types.map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>
        <input value={payload} onChange={(e) => setPayload(e.target.value)} aria-label="Payload (JSON)" placeholder="Payload (JSON)" />
        <button onClick={enqueue}>Run now</button>
      </div>
      <div>{st.jobs.length ? 'Queue:' : 'The queue is empty.'}</div>
      <ul className="pending">
        {st.jobs.map((j) => (
          <li key={j.id}>
            <code>{j.kind}</code> {j.state}, attempt {j.attempts} of {j.maxAttempts},{' '}
            {j.state === 'running' ? 'now' : `at ${when(j.runAt)}`}
            {j.lastError ? ` (${j.lastError})` : ''}{' '}
            <button className="link" onClick={act('jobs_cancel', { id: j.id })}>
              Cancel
            </button>
          </li>
        ))}
      </ul>
      {st.dead.length > 0 && <div>Dead letters (out of attempts):</div>}
      <ul className="pending">
        {st.dead.map((j) => (
          <li key={j.id}>
            <code>{j.kind}</code> after {j.attempts} attempts: {j.lastError}{' '}
            <button className="link" onClick={act('jobs_retry', { id: j.id })}>
              Retry
            </button>
            <button className="link" onClick={act('jobs_cancel', { id: j.id })}>
              Drop
            </button>
          </li>
        ))}
      </ul>
      <div>{schedules.length ? 'Schedules:' : 'No schedules.'}</div>
      <ul className="pending">
        {schedules.map((s) => (
          <li key={s.id}>
            <b>{s.id}</b>: <code>{s.kind}</code> {s.cron ? `cron ${s.cron}` : `every ${s.everySecs} s`},{' '}
            {s.paused ? 'paused' : `next ${when(s.nextRun)}`}{' '}
            <button className="link" onClick={act('jobs_set_schedule', { schedule: { id: s.id, paused: !s.paused } })}>
              {s.paused ? 'Resume' : 'Pause'}
            </button>
          </li>
        ))}
      </ul>
      <div>{st.history.length ? 'Recent runs:' : 'No runs yet.'}</div>
      <ul className="pending">
        {st.history.slice(0, 10).map((r, i) => (
          <li key={i}>
            {when(r.finished)} <code>{r.kind}</code> attempt {r.attempt}: {r.outcome}
            {r.message ? ` (${r.message})` : ''}
          </li>
        ))}
      </ul>
      {msg}
    </div>
  );
}

// rclone: the remote config (owner-only file, never in settings), the download, and Sync now.
function RcloneSetup() {
  const [st, setSt] = useState(null);
  const [text, setText] = useState('');
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  const refresh = () => api.rclone_status().then(setSt, () => {});
  useEffect(() => {
    refresh();
  }, []);
  const saveConf = async () => {
    try {
      setSt(await api.rclone_config({ text }));
      setText('');
      setMsg(text ? 'Config saved.' : 'Config removed.');
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  const run = async (cmd) => {
    setBusy(true);
    const unlisten = await listen('rclone-progress', (e) => setMsg(e.payload));
    try {
      await invoke(cmd);
      setMsg(cmd === 'rclone_sync' ? 'Synced.' : 'rclone is ready.');
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      unlisten();
      setBusy(false);
      refresh();
    }
  };
  return (
    <>
      <div className="hint">
        {st &&
          `rclone ${st.version}: ${st.installed ? 'downloaded' : st.supported ? 'downloads on first sync' : 'not available for this computer'}. ` +
            (st.configured ? `Remotes: ${st.remotes.join(', ') || 'none'}. ` : 'No config yet. ') +
            (st.last ? `Last sync ${st.last.finished} to ${st.last.target}: ${st.last.ok ? 'ok' : 'failed'}.` : '')}
      </div>
      <label className="field">
        Remote config: paste what `rclone config show` prints (kept in a private file)
        <textarea rows={3} value={text} onChange={(e) => setText(e.target.value)} placeholder={'[gdrive]\ntype = drive\n...'} />
      </label>
      <div className="row">
        <button onClick={saveConf}>{text ? 'Save config' : 'Remove config'}</button>
        {st && !st.installed && st.supported && (
          <button onClick={() => run('rclone_install')} disabled={busy}>
            Download rclone
          </button>
        )}
        <button onClick={() => run('rclone_sync')} disabled={busy}>
          Sync now
        </button>
        <span className="hint">{msg || 'Save settings first.'}</span>
      </div>
    </>
  );
}

// Tailscale: never installed by the app; shows the download link when it is missing.
function TailscaleStatus() {
  const [r, setR] = useState(null);
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.tailscale_status().then(setR, (e) => setMsg(errorText(e)));
  }, []);
  const apply = async () => {
    setBusy(true);
    try {
      setR(await api.tailscale_apply());
      setMsg('Done.');
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  if (r && !r.installed)
    return (
      <div className="hint">
        Tailscale is not installed. Get it from <code>{r.installUrl}</code>, then reopen Settings.
      </div>
    );
  const st = r?.status;
  return (
    <>
      <div className="hint">
        {st && `${st.backendState}${st.dnsName ? ` as ${st.dnsName}` : ''}${st.tailnet ? ` in ${st.tailnet}` : ''}. `}
        {r?.served ? `Port ${r.served} is shared${r.url ? ` at ${r.url}` : ''}. ` : r && 'Nothing shared. '}
        {r?.note}
      </div>
      <div className="row">
        <button onClick={apply} disabled={busy || !r}>
          Apply sharing
        </button>
        <span className="hint">{msg || 'Save settings first.'}</span>
      </div>
    </>
  );
}

// The backend as a service (feature `daemon`): headless, at login, restarted on failure.
function ServiceSetup() {
  const [st, setSt] = useState(null);
  const [hint, setHint] = useState('');
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.service_status().then(setSt, (e) => setMsg(errorText(e)));
    api.service_system_hint().then(setHint, () => {});
  }, []);
  const toggle = async (enabled) => {
    setBusy(true);
    try {
      setSt(await api.set_service({ enabled }));
      setMsg(enabled ? 'Installed.' : 'Removed.');
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <div className="row">
        <label className="field">
          <input type="checkbox" checked={!!st?.installed} disabled={busy || !st} onChange={(e) => toggle(e.target.checked)} /> Run the
          backend as a service: no window needed, starts at login, restarts if it fails
        </label>
      </div>
      <div className="hint">
        {st &&
          `${st.manager}: ${st.installed ? 'installed' : 'not installed'}, ${
            st.running ? `running${st.pid ? ` (pid ${st.pid})` : ''}` : 'not running'
          }. Log: ${st.log}. `}
        {st?.note}
      </div>
      <div className="hint">
        To start it at boot for the whole machine instead, run as administrator: <code>{hint}</code>
      </div>
      <p className="status" role="status">
        {msg}
      </p>
    </>
  );
}

// Embedded web views: open an allowed page in its own window. Remote pages get no commands;
// bridge messages from trusted pages arrive as 'webview-message' events.
function WebviewTry() {
  const [url, setUrl] = useState('https://');
  const [msg, setMsg] = useState('Save settings first.');
  useEffect(() => {
    const off = listen('webview-message', (e) => setMsg(`Message from ${e.payload.host}: ${JSON.stringify(e.payload.data)}`));
    return () => {
      off.then((f) => f());
    };
  }, []);
  const open = async () => {
    try {
      const label = await api.webview_open({ url, name: 'settings' });
      setMsg(`Opened in ${label}.`);
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  return (
    <div className="row">
      <input value={url} onChange={(e) => setUrl(e.target.value)} aria-label="Web page to open" />
      <button onClick={open}>Open</button>
      <button className="link" onClick={() => api.webview_close({ name: 'settings' }).catch((e) => setMsg(errorText(e)))}>
        Close
      </button>
      <span className="hint">{msg}</span>
    </div>
  );
}

// The local REST API: where it listens and the token scripts send (Authorization: Bearer).
function HttpApiInfo() {
  const [r, setR] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    api.http_api_info().then(setR, (e) => setMsg(errorText(e)));
  }, []);
  const copy = async (text, what) => {
    try {
      await navigator.clipboard.writeText(text);
      setMsg(`${what} copied.`);
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  if (!r) return <div className="hint">{msg}</div>;
  return (
    <>
      <div className="hint">
        {r.running ? `Listening on ${r.url}.` : r.error ? `Not running: ${r.error}` : 'Not running. Turn it on and save settings.'}
      </div>
      {r.url && (
        <div className="row">
          <code>{r.url}</code>
          <button className="link" onClick={() => copy(r.url, 'URL')}>
            Copy URL
          </button>
        </div>
      )}
      {r.token && (
        <div className="row">
          <code>{`${r.token.slice(0, 8)}…`}</code>
          <button className="link" onClick={() => copy(r.token, 'Token')}>
            Copy token
          </button>
        </div>
      )}
      <span className="hint">{msg || 'Send the token as Authorization: Bearer <token>. Keep it secret.'}</span>
    </>
  );
}

const megabytes = (bytes) => `${(bytes / 1048576).toFixed(1)} MB`;

// The key-value store: where it is, how big, and its namespaces.
function KvInfo() {
  const [r, setR] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    api.kv_info().then(setR, (e) => setMsg(errorText(e)));
  }, []);
  if (!r) return <div className="hint">{msg}</div>;
  return (
    <>
      <div className="hint">
        <code>{r.path}</code> · {megabytes(r.bytes)}
      </div>
      {r.namespaces.length === 0 ? (
        <div className="hint">No keys yet.</div>
      ) : (
        <ul className="hint">
          {r.namespaces.map((n) => (
            <li key={n.name}>
              <code>{n.name}</code>: {n.keys} keys
              {n.encrypted ? ', encrypted' : ''}
              {r.uiNamespaces.includes(n.name) ? ', open to the window' : ''}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

// The app cache: hits, misses and size, and a button to empty it.
function CacheStats() {
  const [s, setS] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    api.cache_stats().then(setS, (e) => setMsg(errorText(e)));
  }, []);
  const clear = async () => {
    try {
      setS(await api.cache_clear());
      setMsg('Cache cleared.');
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  return (
    <>
      {s && (
        <div className="hint">
          {s.entries} entries, {megabytes(s.weight)} of {megabytes(s.maxWeight)} · {s.hits} hits, {s.misses} misses (
          {Math.round(s.hitRate * 100)}% hit rate)
        </div>
      )}
      <div className="row">
        <button className="link" onClick={clear}>
          Clear the cache
        </button>
        <span className="hint">{msg}</span>
      </div>
    </>
  );
}

function VoiceDownload() {
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    const unlisten = await listen('voice-progress', (e) => setMsg(`Downloading ${e.payload}`));
    try {
      await api.download_voice();
      setMsg('Voice ready.');
    } catch (e) {
      setMsg(errorText(e));
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
      await api.download_stt();
      setMsg('Model ready.');
    } catch (e) {
      setMsg(errorText(e));
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

// Separate settings, documents and index per profile; switching restarts the app.
function Profiles() {
  const [p, setP] = useState(null);
  const [name, setName] = useState('');
  const [msg, setMsg] = useState('');
  const load = () =>
    api
      .list_profiles()
      .then(setP)
      .catch((e) => setMsg(errorText(e)));
  useEffect(() => {
    load();
  }, []);
  if (!p) return null;
  const act = (fn) => async () => {
    setMsg('');
    try {
      await fn();
    } catch (e) {
      setMsg(errorText(e));
    }
    load();
  };
  return (
    <div>
      <ul className="profiles">
        {p.saved.profiles.map((x) => (
          <li key={x.id}>
            {x.name}
            {x.id === p.running ? ' (open now)' : ''}{' '}
            {x.id !== p.running && (
              <button className="link" onClick={act(() => api.switch_profile({ id: x.id }))}>
                Switch
              </button>
            )}
            {x.id !== p.running && x.id !== 'default' && (
              <button className="link" onClick={act(() => api.remove_profile({ id: x.id, deleteData: false }))}>
                Remove (keeps its files)
              </button>
            )}
          </li>
        ))}
      </ul>
      <div className="row">
        <input placeholder="New profile name" aria-label="New profile name" value={name} onChange={(e) => setName(e.target.value)} />
        <button
          disabled={!name.trim()}
          onClick={act(async () => {
            await api.create_profile({ name });
            setName('');
          })}
        >
          Add profile
        </button>
      </div>
      <p className="hint">{msg}</p>
    </div>
  );
}

function Backups() {
  const [list, setList] = useState([]);
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);
  const [recovery, setRecovery] = useState('');
  const [key, setKey] = useState('');
  const load = () =>
    api
      .list_backups()
      .then(setList)
      .catch((e) => setMsg(errorText(e)));
  useEffect(() => {
    load();
  }, []);
  const run = async (what, fn) => {
    setBusy(true);
    setMsg(what);
    try {
      setMsg(await fn());
    } catch (e) {
      setMsg(errorText(e));
    }
    setBusy(false);
    load();
  };
  const now = () => run('Backing up…', async () => `Backed up ${(await api.backup_now()).chunks} chunks.`);
  const restore = (b) =>
    run('Restoring…', async () => {
      const s = await api.restore_backup({ path: b.path, key: key || null });
      return `Restored ${s.chunks} chunks. The index it replaced was backed up first.`;
    });
  return (
    <div>
      <div className="row">
        <button onClick={now} disabled={busy}>
          Back up now
        </button>
        <span className="hint">{msg}</span>
      </div>
      {list.length === 0 && <p className="hint">No backups yet.</p>}
      <ul className="backups">
        {list.map((b) => (
          <li key={b.path}>
            {new Date(b.created).toLocaleString()}: {b.chunks} chunks from {b.sources} sources ({Math.ceil(b.bytes / 1024)} KB)
            {b.encrypted ? ', encrypted' : ''}{' '}
            <button className="link" disabled={busy} onClick={() => restore(b)}>
              Restore
            </button>
          </li>
        ))}
      </ul>
      <div className="row">
        <button
          className="link"
          onClick={() =>
            api.backup_recovery_key().then(
              (r) => setRecovery(r.key),
              (e) => setMsg(errorText(e)),
            )
          }
        >
          Show the recovery key
        </button>
        {recovery && <code>{recovery}</code>}
      </div>
      {recovery && <p className="hint">Keep it somewhere safe: an encrypted backup cannot be restored on another computer without it.</p>}
      <label className="field">
        Recovery key for a backup from another computer (optional)
        <input value={key} onChange={(e) => setKey(e.target.value)} />
      </label>
    </div>
  );
}

function TelemetryStatus() {
  const [s, setS] = useState(null);
  useEffect(() => {
    api
      .telemetry_status()
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
    api
      .mcp_setup()
      .then(setSetup)
      .catch(() => {});
  }, []);
  const register = async () => {
    try {
      const r = await api.mcp_register_claude();
      setMsg(`Claude Desktop: ${r}. Restart Claude Desktop to load it.`);
    } catch (e) {
      setMsg(errorText(e));
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

// Allow, ask or deny per MCP tool. "Ask" waits for an answer in the panel below.
function McpPolicies({ policies, onChange }) {
  const [tools, setTools] = useState([]);
  useEffect(() => {
    api
      .mcp_setup()
      .then((s) => setTools(['*', ...s.tools]))
      .catch(() => {});
  }, []);
  const map = policies || {};
  const pick = (tool, v) => {
    const next = { ...map };
    if (v === '') delete next[tool];
    else next[tool] = v;
    onChange(next);
  };
  return (
    <div className="grid">
      {tools.map((t) => (
        <label className="field" key={t}>
          {t === '*' ? 'Every other tool' : `Tool ${t}`}
          <select value={map[t] || ''} onChange={(e) => pick(t, e.target.value)}>
            <option value="">{t === '*' ? 'allow (default)' : 'as every other tool'}</option>
            <option value="allow">allow</option>
            <option value="ask">ask me first</option>
            <option value="deny">deny</option>
          </select>
        </label>
      ))}
    </div>
  );
}

// MCP calls waiting for approval (tools set to "ask"), polled while Settings is open.
function McpPending() {
  const [calls, setCalls] = useState([]);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    const tick = () =>
      api
        .mcp_pending()
        .then(setCalls)
        .catch(() => {});
    tick();
    const t = setInterval(tick, 2000);
    return () => clearInterval(t);
  }, []);
  const decide = async (id, allow) => {
    try {
      setCalls(await api.mcp_decide({ id, allow }));
      setMsg('');
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  return (
    <div className="hint">
      <div>{calls.length ? `${calls.length} MCP calls waiting for you:` : 'No MCP calls waiting for approval.'}</div>
      <ul className="pending">
        {calls.map((c) => (
          <li key={c.id}>
            <code>{c.tool}</code> {c.arguments}{' '}
            <button className="link" onClick={() => decide(c.id, true)}>
              Approve
            </button>
            <button className="link" onClick={() => decide(c.id, false)}>
              Deny
            </button>
          </li>
        ))}
      </ul>
      {msg}
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
      const path = await api.download_model({});
      onDone(path);
      setMsg(`Ready: ${path}`);
    } catch (e) {
      setMsg(errorText(e));
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
    api
      .ai_keys()
      .then(setHave)
      .catch(() => {});
  }, []);
  const save = async (value) => {
    try {
      setHave(await api.set_ai_key({ provider, key: value }));
      setKey('');
      setMsg(value ? 'Key saved.' : 'Key removed.');
    } catch (e) {
      setMsg(errorText(e));
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

// Settings → AI usage: tokens and cost today, this month and per recent day, with the
// budgets. Prices come from `llm.prices` in settings.json (edit them when prices change).
function AiUsage() {
  const [u, setU] = useState(null);
  const [err, setErr] = useState('');
  const load = () =>
    api
      .ai_usage()
      .then(setU)
      .catch((e) => setErr(errorText(e)));
  useEffect(() => {
    load();
  }, []);
  const usd = (t) => `$${(t?.costUsd || 0).toFixed(4)}${t?.unpriced ? ` (+${t.unpriced} unpriced)` : ''}`;
  const toks = (t) => `${t?.inputTokens || 0} in / ${t?.outputTokens || 0} out, ${t?.cachedTokens || 0} cached`;
  if (err) return <p className="status error">{err}</p>;
  if (!u) return null;
  const rows = [
    [`Today (${u.day})`, u.today],
    [`This month (${u.month})`, u.thisMonth],
    ...u.days.filter(([d]) => d !== u.day).slice(0, 7),
  ];
  return (
    <div className="ai-usage">
      <h3>AI usage</h3>
      <table>
        <thead>
          <tr>
            <th>Period</th>
            <th>Calls</th>
            <th>Tokens</th>
            <th>Cost</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([name, t]) => (
            <tr key={name}>
              <td>{name}</td>
              <td>{t?.requests || 0}</td>
              <td>{toks(t)}</td>
              <td>{usd(t)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {u.budgets.map((b) => (
        <div key={b.name} className="row">
          <label>
            {b.name}: {b.name.endsWith('USD') ? `$${b.used.toFixed(2)} of $${b.limit.toFixed(2)}` : `${b.used} of ${b.limit}`}
          </label>
          <meter min={0} max={100} low={79} high={99} optimum={0} value={Math.min(b.percent, 100)} />
          <span className={b.state === 'ok' ? 'hint' : 'status error'}>
            {b.state === 'over' ? (u.overrideLimits ? 'over (override on)' : 'used up') : `${b.percent.toFixed(0)}%`}
          </span>
        </div>
      ))}
      {!u.budgets.length && <p className="hint">No budgets set.</p>}
      <button className="link" onClick={load}>
        Refresh
      </button>
    </div>
  );
}

// A key press as an accelerator Tauri reads, like CmdOrCtrl+Shift+K (null for a lone modifier).
const MAC = navigator.userAgent.includes('Mac');
function keysOf(e) {
  if (['Shift', 'Control', 'Alt', 'Meta', 'OS'].includes(e.key)) return null;
  const c = e.code || '';
  const key = c.startsWith('Key')
    ? c.slice(3)
    : c.startsWith('Digit')
      ? c.slice(5)
      : c.startsWith('Numpad')
        ? c.replace('Numpad', 'Num')
        : c;
  const mods = [];
  if (MAC ? e.metaKey : e.ctrlKey) mods.push('CmdOrCtrl');
  if (MAC && e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (!MAC && e.metaKey) mods.push('Super');
  return [...mods, key].join('+');
}

// "Press keys" field: focus it and press the combination. Esc cancels, Backspace clears.
function KeyCapture({ label, value, onChange }) {
  const [listening, setListening] = useState(false);
  const down = (e) => {
    e.preventDefault();
    if (e.key === 'Escape') return e.target.blur();
    if ((e.key === 'Backspace' || e.key === 'Delete') && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
      onChange('');
      return e.target.blur();
    }
    const keys = keysOf(e);
    if (keys) {
      onChange(keys);
      e.target.blur();
    }
  };
  return (
    <label className="field">
      {label}
      <input
        readOnly
        value={listening ? 'Press keys…' : value || ''}
        placeholder="Click, then press keys"
        onFocus={() => setListening(true)}
        onBlur={() => setListening(false)}
        onKeyDown={down}
      />
    </label>
  );
}

function ActionPick({ value, actions, onChange, separator }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} aria-label="Action">
      {separator && <option value="separator">— separator —</option>}
      {actions.map((a) => (
        <option key={a.id} value={a.id}>
          {a.label} ({a.id})
        </option>
      ))}
    </select>
  );
}

// Tray or menu items: action, label, accelerator, on/off, order.
function EntryList({ title, entries, actions, onChange, keys }) {
  const put = (i, k, v) => onChange(entries.map((e, j) => (j === i ? { ...e, [k]: v } : e)));
  const move = (i, d) => {
    const next = [...entries];
    const [e] = next.splice(i, 1);
    next.splice(Math.max(0, Math.min(next.length, i + d)), 0, e);
    onChange(next);
  };
  const add = () => onChange([...entries, { id: `item${Date.now()}`, label: '', action: 'show', shortcutHint: null, enabled: true }]);
  return (
    <div className="panel">
      {title && <h4>{title}</h4>}
      {entries.map((e, i) => (
        <div className="row entry" key={e.id || i}>
          <ActionPick value={e.action} actions={actions} separator onChange={(v) => put(i, 'action', v)} />
          {e.action !== 'separator' && (
            <>
              <input
                value={e.label}
                placeholder="Label (empty = default)"
                aria-label="Menu label"
                onChange={(x) => put(i, 'label', x.target.value)}
              />
              {keys && <KeyCapture label="" value={e.shortcutHint} onChange={(v) => put(i, 'shortcutHint', v || null)} />}
            </>
          )}
          <span className="row-tools">
            {e.action !== 'separator' && (
              <label className="field">
                <input type="checkbox" checked={e.enabled} onChange={(x) => put(i, 'enabled', x.target.checked)} /> on
              </label>
            )}
            <button className="link" onClick={() => move(i, -1)} aria-label="Move up" title="Move up">
              ↑
            </button>
            <button className="link" onClick={() => move(i, 1)} aria-label="Move down" title="Move down">
              ↓
            </button>
            <button className="link" onClick={() => onChange(entries.filter((_, j) => j !== i))}>
              Remove
            </button>
          </span>
        </div>
      ))}
      <button className="link" onClick={add}>
        Add an item
      </button>
    </div>
  );
}

// The tray menu, the app menu and global shortcuts, with what could not be set up.
function TrayShortcuts({ cfg, setCfg }) {
  const [status, setStatus] = useState({ actions: [], problems: [], shortcuts: [] });
  useEffect(() => {
    api
      .tray_status()
      .then(setStatus)
      .catch(() => {});
    const off = listen('tray-status', (e) => setStatus(e.payload));
    return () => {
      off.then((f) => f());
    };
  }, []);
  const actions = status.actions;
  const shortcuts = cfg.shortcuts || [];
  const putShortcut = (i, k, v) => setCfg({ ...cfg, shortcuts: shortcuts.map((s, j) => (j === i ? { ...s, [k]: v } : s)) });
  const failed = (s) => status.shortcuts.find((b) => b.accelerator === s.accelerator && b.action === s.action)?.error;
  return (
    <>
      <EntryList
        title="Tray menu"
        entries={cfg.tray.items}
        actions={actions}
        onChange={(v) => setCfg({ ...cfg, tray: { ...cfg.tray, items: v } })}
      />
      <div className="panel">
        <h4>App menu</h4>
        <label className="field">
          Title
          <input value={cfg.menu.title} onChange={(e) => setCfg({ ...cfg, menu: { ...cfg.menu, title: e.target.value } })} />
        </label>
      </div>
      <EntryList
        title=""
        entries={cfg.menu.extra}
        actions={actions}
        keys
        onChange={(v) => setCfg({ ...cfg, menu: { ...cfg.menu, extra: v } })}
      />
      <div className="panel">
        <h4>Global shortcuts</h4>
        <p className="hint">
          {shortcuts.length ? 'They work while other apps are in front.' : 'None yet. They work while other apps are in front.'}
        </p>
        {shortcuts.map((s, i) => (
          <div className="row" key={i}>
            <KeyCapture label="" value={s.accelerator} onChange={(v) => putShortcut(i, 'accelerator', v)} />
            <ActionPick value={s.action} actions={actions} onChange={(v) => putShortcut(i, 'action', v)} />
            <button className="link" onClick={() => setCfg({ ...cfg, shortcuts: shortcuts.filter((_, j) => j !== i) })}>
              Remove
            </button>
            {failed(s) && <span className="status error">{failed(s)}</span>}
          </div>
        ))}
        <button className="link" onClick={() => setCfg({ ...cfg, shortcuts: [...shortcuts, { accelerator: '', action: 'show_hide' }] })}>
          Add a shortcut
        </button>
      </div>
      {status.problems.length > 0 && (
        <ul className="hint">
          {status.problems.map((p) => (
            <li key={p} className="status error">
              {p}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

// This install's rollout bucket and the rollback to the version before the last update.
function UpdateChannel() {
  const [st, setSt] = useState(null);
  const [msg, setMsg] = useState('');
  useEffect(() => {
    api
      .update_channel_status()
      .then(setSt)
      .catch((e) => setMsg(errorText(e)));
  }, []);
  const rollback = async () => {
    setMsg(`Going back to ${st.previous}...`);
    try {
      await api.rollback_update();
    } catch (e) {
      setMsg(`Rollback failed: ${errorText(e)}`);
    }
  };
  if (!st)
    return (
      <p className="status" role="status">
        {msg}
      </p>
    );
  return (
    <div className="hint">
      <div>Save to switch channels; the next update check uses the new one.</div>
      {st.bucket !== null && <div>Staged releases reach this install once they are rolled out to more than {st.bucket}%.</div>}
      {st.previous ? (
        <div className="row">
          <span>
            Previous version: {st.previous}
            {st.kept ? ' (kept on this computer)' : ' (downloaded again from its release)'}
          </span>
          {st.rollback && (
            <button className="link" onClick={rollback}>
              Roll back to {st.previous}
            </button>
          )}
        </div>
      ) : (
        <div>No earlier version to roll back to yet.</div>
      )}
      <p className="status" role="status">
        {msg}
      </p>
    </div>
  );
}

// Settings → About → Licences: the open-source software this build ships (made at build time).
function Licences() {
  const [n, setN] = useState(null);
  const [filter, setFilter] = useState('');
  const [full, setFull] = useState(false);
  useEffect(() => {
    api
      .third_party_notices()
      .then(setN)
      .catch(() => {});
  }, []);
  if (!n) return null;
  const q = filter.trim().toLowerCase();
  const shown = n.packages.filter((p) => !q || `${p.name} ${p.license}`.toLowerCase().includes(q));
  return (
    <details open>
      <summary>Licences ({n.packages.length} packages)</summary>
      <label className="field">
        Filter by name or licence
        <input value={filter} onChange={(e) => setFilter(e.target.value)} />
      </label>
      <table className="licences">
        <thead>
          <tr>
            <th>Package</th>
            <th>Version</th>
            <th>Licence</th>
          </tr>
        </thead>
        <tbody>
          {shown.map((p) => (
            <tr key={`${p.ecosystem}:${p.name}@${p.version}`}>
              <td>
                {p.name}
                {p.ecosystem === 'npm' ? ' (npm)' : ''}
              </td>
              <td>{p.version}</td>
              <td>{p.license}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <button className="link" onClick={() => setFull(!full)}>
        {full ? 'Hide' : 'Show'} the licence texts
      </button>
      {full && <pre className="notices">{n.text}</pre>}
    </details>
  );
}

export default function Settings({ features }) {
  const [cfg, setCfg] = useState(null);
  const [msg, setMsg] = useState('');
  const [open, setOpen] = useState('');
  useEffect(() => {
    api
      .get_config()
      .then(setCfg)
      .catch((e) => setMsg(errorText(e)));
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
      setCfg(await api.set_config({ config: cfg }));
      setMsg('Saved.');
    } catch (e) {
      setMsg(`Not saved: ${errorText(e)}`);
    }
  };
  const reset = async () => {
    setMsg('');
    setCfg(await api.set_config({ config: {} }));
  };
  const needs = {
    appearance: 'appShell',
    watch: 'watch',
    postgres: 'postgres',
    llm: 'ai',
    mcp: 'mcp',
    tts: 'tts',
    stt: 'stt',
    mail: 'mail',
    oauth: 'oauth',
    plugins: 'plugins',
    jobs: 'jobs',
    rclone: 'rclone',
    tailscale: 'tailscale',
    httpApi: 'httpApi',
    webview: 'webview',
    telemetry: 'telemetry',
    service: 'daemon',
    tray: 'trayMenu',
    updates: 'releaseChannels',
    about: 'supplyChain',
    kv: 'kv',
    cache: 'cache',
  };
  const sections = [
    'appearance',
    'search',
    'postgres',
    'ingest',
    'llm',
    'mcp',
    'tts',
    'stt',
    'resources',
    'backup',
    'jobs',
    'rclone',
    'mail',
    'oauth',
    'tailscale',
    'httpApi',
    'kv',
    'cache',
    'webview',
    'plugins',
    'net',
    'telemetry',
    'log',
    'watch',
    'service',
    'tray',
    'updates',
    'about',
    // kitchen:sections
  ].filter((s) => !needs[s] || features[needs[s]]);
  // One section at a time: a list of sections beside it on wide windows, above it on narrow ones.
  const all = ['profiles', ...sections];
  const s = all.includes(open) ? open : all[0];
  const fields = FIELDS.filter(([sec, , , , x]) => sec === s && (!x.needs || features[x.needs]));
  return (
    <section id="settings" className="settings">
      <h2>Settings</h2>
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {all.map((id) => (
            <button key={id} data-section={id} aria-current={id === s ? 'page' : undefined} onClick={() => setOpen(id)}>
              {id === 'profiles' ? 'Profiles' : TITLES[id]}
            </button>
          ))}
        </nav>
        <div className="settings-pane">
          <h3>{s === 'profiles' ? 'Profiles' : TITLES[s]}</h3>
          {s === 'profiles' && <Profiles />}
          {fields.length > 0 && (
            <div className="grid">
              {fields.map(([sec, key, label, kind, extra]) => (
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
          )}
          {s === 'watch' && <WatchStatus />}
          {s === 'llm' && <AiKeys />}
          {s === 'llm' && <AiUsage />}
          {s === 'llm' && features.llama && <ModelDownload onDone={(p) => set('llm', 'local.modelPath', p)} />}
          {s === 'mcp' && <McpPolicies policies={cfg.mcp.policies} onChange={(v) => set('mcp', 'policies', v)} />}
          {s === 'mcp' && cfg.mcp.enabled && <McpPending />}
          {s === 'mcp' && <McpConnect />}
          {s === 'tts' && <VoiceDownload />}
          {s === 'stt' && <SttDownload />}
          {s === 'telemetry' && <TelemetryStatus />}
          {s === 'backup' && <Backups />}
          {s === 'mail' && <MailSetup from={cfg.mail?.from} />}
          {s === 'oauth' && (
            <OauthSetup
              providers={cfg.oauth?.providers || []}
              onChange={(v) => setCfg({ ...cfg, oauth: { ...cfg.oauth, providers: v } })}
              deepLink={features.oauthDeepLink}
              redirect={cfg.oauth?.redirect}
            />
          )}
          {s === 'plugins' && (
            <PluginList
              settings={cfg.plugins?.settings || {}}
              onChange={(v) => setCfg({ ...cfg, plugins: { ...cfg.plugins, settings: v } })}
            />
          )}
          {s === 'jobs' && <JobsPanel />}
          {s === 'rclone' && <RcloneSetup />}
          {s === 'tailscale' && <TailscaleStatus />}
          {s === 'service' && <ServiceSetup />}
          {s === 'httpApi' && <HttpApiInfo />}
          {s === 'kv' && <KvInfo />}
          {s === 'cache' && <CacheStats />}
          {s === 'webview' && <WebviewTry />}
          {s === 'tray' && <TrayShortcuts cfg={cfg} setCfg={setCfg} />}
          {s === 'updates' && <UpdateChannel />}
          {s === 'about' && <Licences />}
        </div>
      </div>
      <div className="settings-actions">
        <button onClick={save}>Save settings</button>
        <button className="link" onClick={reset}>
          Reset to defaults
        </button>
        <span className="status" role="status">
          {msg}
        </span>
      </div>
    </section>
  );
}
