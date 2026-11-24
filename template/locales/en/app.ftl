# The source of every message in the app: the UI, the native menus and tray, the test email
# and the backend's error messages. Every other locale translates this file
# (`kitchen generate locale <code>` starts one; `kitchen i18n check` compares them).
#
# Syntax: https://projectfluent.org/fluent/guide/ . Variables are { $name }; plurals select
# on a number ({ $count -> [one] ... *[other] ... }), with the forms of each language.

## The language picker shows this name (in this language).

language-name = English

## Tray, app menu and global shortcuts (Rust: src-tauri/src/tray.rs); action-done-*: what an action reports back.

action-show-hide = Show/Hide window
action-show = Show window
action-hide = Hide window
action-search = Search…
action-ingest-clipboard = Add clipboard text
action-ingest-drop-folder = Import the drop folder
action-pause-resume = Pause background work
action-resume = Resume background work
action-settings = Settings…
action-quit = Quit
action-done-clipboard-empty = The clipboard has no text.
action-done-clipboard = { $chunks ->
    [one] Added the clipboard text (one chunk).
   *[other] Added the clipboard text ({ $chunks } chunks).
}
action-done-no-drop-folder = The drop folder { $folder } does not exist yet.
action-done-imported = Imported the drop folder: { $files ->
    [one] one file
   *[other] { $files } files
}, { $chunks ->
    [one] one chunk
   *[other] { $chunks } chunks
}.
action-done-paused = Background work paused.
action-done-resumed = Background work resumed.
menu-actions = Actions

## The test email (Rust: src-tauri/src/service.rs).

mail-test-subject = Test email
mail-test-body = Email from this app works.
mail-test-login = Test login
mail-test-send = Send a test email to myself

## Error messages from the backend (Rust: src-tauri/src/error.rs). The error code stays the same in every language.

error-not-ready = Still starting; try again in a moment.
error-embedding = embedding: { $detail }
error-store = store: { $detail }
error-ai = AI: { $detail }
error-not-built = { $part } is not in this build

## Main screen (ui-react/src/App.jsx).

starting = Starting…
setup-failed = Setup failed: { $error }
upgrade-label = Database upgrade
upgrade-go = Upgrade database
upgrade-later = Not now
footer-status = { $app } · { $chunks ->
    [one] { $chunks } chunk
   *[other] { $chunks } chunks
} · { $backend } store · { $embedder }
footer-keyword-search = keyword search
footer-env = { $env } build
clear-confirm = Click again to clear everything
clear = Clear all data

## First-run setup wizard.

wizard-title = Welcome
wizard-intro-device = A one-time setup. Everything stays on this device.
wizard-intro = A one-time setup. Everything stays on this computer.
wizard-store = 1. Where to keep your data
wizard-store-file = A simple file (fast, nothing to download)
wizard-store-postgres = A private Postgres database with vector search (downloads about 30 MB the first time)
wizard-no-postgres = This processor cannot run the bundled database; the file store works the same.
wizard-model = 2. Smart search
wizard-model-embed-bundled = Search by meaning with a local AI model (built into the app, no download; nothing is sent anywhere)
wizard-model-embed = Search by meaning with a local AI model (downloads once, nothing is sent anywhere)
wizard-go-title = 3. Set up
wizard-go = Set up

## Update banner.

update-downloading-percent = Downloading… { $percent }%
update-downloading = Downloading…
update-failed = Update failed: { $error }
update-available = Version { $version } is available (you have { $current }).
update-install = Install and restart

## Add documents.

add-reading = Reading { $path }…
add-file-done = { $count ->
    [one] Added one chunk from { $path }.
   *[other] Added { $count } chunks from { $path }.
}
add-reading-folder = Reading every document in { $path }…
add-folder-failed = Added { $files } files ({ $chunks } chunks), { $failed } failed: { $error }.
add-folder-done = Added { $files ->
    [one] one file
   *[other] { $files } files
} ({ $chunks } chunks).
add-text-done = { $count ->
    [one] Added one chunk.
   *[other] Added { $count } chunks.
}
add-title = Add documents
add-hint-mobile = Paste text, or give the path of a file in this app.
add-hint = Drop files on this window (PDF, .txt, .md), give a file path, or paste text.
add-path-placeholder = /path/to/file.pdf
add-path-label = File or folder path
add-file = Add file
add-folder = Add folder
add-paste-placeholder = Paste text here
add-paste-label = Text to add
add-name = Name for the pasted text
add-text = Add text

## Search.

search-title = Search
search-placeholder = What are you looking for?
search-go = Search
search-none = Nothing found.
search-page = , page { $page }
search-vector-score = { " " }· vector { $score }
search-keyword-score = { " " }· keyword { $score }

## Ask (AI).

ask-tokens = { $input } in / { $output } out tokens
ask-cached-tokens = { $count } cached
ask-failed-tries = { $count ->
    [one] after one failed try
   *[other] after { $count } failed tries
}
ask-from-cache = from the answer cache
ask-title = Ask
ask-placeholder = Ask about your documents
ask-question = Question
ask-thinking = Thinking…
ask-go = Ask

## Dictation and speech.

dictate-nothing = No speech heard.
dictate-mic-error = Microphone: { $error }
dictate-stop = ■ Stop
dictate-stop-title = Stop and transcribe
dictate = Dictate
dictate-file-title = Transcribe an audio file
dictate-file = Audio file
speak-preparing = Preparing…
speak = Read aloud

## Background agent, Python example and Claude Desktop.

agent-title = Background
agent-toggle = Keep the backend running after the window closes, and start it at login
agent-on = On: { $kind } at { $location }
agent-off = Off ({ $kind })
python-title = Python example
python-hint = Sends this text to a separate Python process:
python-go = Text stats in Python
claude-registered-backup = Claude Desktop config { $status } (backup: { $backup }). Preparing the tools once…
claude-registered = Claude Desktop config { $status }. Preparing the tools once…
claude-ready = Ready. Restart Claude Desktop to use it.
claude-retry = Connected; the first start failed and will retry when Claude starts.
claude-title = Claude Desktop
claude-hint = Adds this app’s tools to Claude Desktop (your current settings are backed up first).
claude-go = Connect Claude Desktop

## The app shell: custom titlebar.

titlebar-settings-keys = Settings ({ $keys })
titlebar-settings = Settings
titlebar-minimize = Minimize
titlebar-restore = Restore
titlebar-maximize = Maximize
titlebar-close = Close

## Settings (ui-react/src/Settings.jsx).

settings-saved = Saved.
settings-not-saved = Not saved: { $error }
settings-title = Settings
settings-sections = Settings sections
settings-save = Save settings
settings-reset = Reset to defaults
save-first = Save settings first.
remove = Remove
refresh = Refresh

## Settings sections.

section-profiles = Profiles
section-appearance = Appearance
section-language = Language
section-search = Search
section-postgres = Database
section-ingest = Ingest
section-llm = AI
section-mcp = MCP server
section-tts = Speech
section-stt = Dictation
section-telemetry = Privacy
section-log = Logs
section-resources = Performance
section-net = Network
section-backup = Backups
section-mail = Email
section-oauth = Sign-in
section-plugins = Plugins
section-jobs = Jobs
section-rclone = Cloud sync
section-tailscale = Tailnet
section-http-api = HTTP API
section-webview = Web views
section-watch = Watch folders
section-service = Service
section-tray = Tray & shortcuts
section-updates = Updates
section-about = About

## Settings fields: setting-<section>-<key>, and setting-<section>-<key>-<option> for the choices of a list.

setting-ui-language = Language
    .help = Menus and messages switch at once. Missing translations show in English.
setting-ui-language-system = Same as the system
setting-search-mode = Search mode
setting-search-top-k = Results
setting-search-candidates = Candidates per list (hybrid)
setting-search-vector-weight = Vector weight
setting-search-keyword-weight = Keyword weight
setting-search-rrf-k = RRF k
setting-search-min-vector-score = Minimum vector score (cosine)
setting-search-min-keyword-score = Minimum keyword score
setting-search-min-score = Minimum final score
setting-search-keyword-engine = Keyword engine (Postgres)
setting-search-bm25-tokenizer = BM25 tokenizer model
setting-search-fulltext-language = Full-text language
setting-postgres-upgrade = When a new version brings a newer Postgres
setting-postgres-upgrade-ask = Ask first
setting-postgres-upgrade-auto = Upgrade at start
setting-postgres-upgrade-off = Keep the old version
setting-postgres-upgrade-copy = Upgrade by copying (slower, twice the disk space)
setting-ingest-workers = Ingest workers (0 = one per core)
setting-ingest-chunk-chars = Chunk size (characters)
setting-ingest-chunk-overlap = Chunk overlap (characters)
setting-ingest-embed-batch = Chunks per embedding call
setting-ingest-max-file-mb = Largest file (MB)
setting-ingest-extensions = File types
setting-ingest-pdf-isolate = Read PDFs in a separate, limited process
setting-ingest-pdf-timeout-secs = PDF time limit (seconds)
setting-ingest-pdf-memory-mb = PDF memory limit (MB)
setting-llm-enabled = Turn on AI answers
setting-llm-provider = Provider
setting-llm-model = Model (empty = provider default)
setting-llm-base-url = Server URL (empty = provider default)
setting-llm-local-model-path = Local model file (.gguf)
setting-llm-local-model-url = Local model download URL
setting-llm-local-context = Local context (tokens)
setting-llm-local-gpu-layers = Layers on the GPU (0 = CPU only)
setting-llm-local-threads = CPU threads (0 = one per core)
setting-llm-local-keep-loaded = Keep the local model in memory
setting-llm-temperature = Temperature (empty = model default)
setting-llm-max-tokens = Longest answer (tokens)
setting-llm-timeout-secs = Timeout (seconds)
setting-llm-retries = Retries before the next provider
setting-llm-retry-base-ms = First retry wait (ms, doubles each retry)
setting-llm-retry-max-secs = Longest retry wait (s; a longer Retry-After moves to the next provider)
setting-llm-daily-token-budget = Daily token budget
setting-llm-monthly-token-budget = Monthly token budget
setting-llm-budget-daily-usd = Daily budget (USD)
setting-llm-budget-monthly-usd = Monthly budget (USD)
setting-llm-budget-warn-percent = Warn at (% of a budget; 0 = never)
setting-llm-budget-override-limits = Keep going over budget (override)
setting-llm-prompt-cache = Prompt caching (Anthropic cache_control; OpenAI and Gemini cache on their own)
setting-llm-system-prompt = System prompt (empty = built in)
setting-llm-http-hosts = Hosts allowed over plain HTTP
setting-llm-cache-enabled = Reuse answers to repeated questions (cleared when documents change)
setting-llm-cache-ttl-hours = Keep cached answers for (hours)
setting-llm-cache-max-entries = Most cached answers
setting-llm-cache-similarity = Similar question match (cosine, 1 = exact only)
setting-mcp-enabled = Let MCP clients (Claude Desktop, Claude Code, ...) search these documents
setting-mcp-allow-write = Also allow tools that change data (add, remove)
setting-mcp-tools = Only these tools (empty = all allowed)
setting-mcp-audit-log = Keep an audit log of tool calls
setting-mcp-audit-arguments = Log the call arguments too
setting-mcp-max-result-chars = Longest reply (characters)
setting-mcp-approval-timeout-secs = Wait for approval of "ask" tools (seconds)
setting-tts-enabled = Read answers aloud (tiny-voca, on this computer)
setting-tts-voice = Voice
setting-tts-voice-0 = F2 (female)
setting-tts-voice-1 = M2 (male)
setting-tts-speed = Speed
setting-tts-pitch-shift = Pitch shift
setting-tts-pitch-range = Pitch range
setting-tts-pause-ms = Pause between sentences (ms)
setting-tts-threads = CPU threads (0 = automatic)
setting-stt-enabled = Dictation and audio files (Whisper, on this computer)
setting-stt-model = Model
setting-stt-model-tiny = Tiny (40 MB, fastest)
setting-stt-model-base = Base (75 MB)
setting-stt-model-small = Small (250 MB, most accurate)
setting-stt-language = Language code, e.g. en, de (empty = detect)
setting-stt-translate = Translate into English
setting-stt-max-seconds = Longest recording (seconds)
setting-stt-threads = CPU threads (0 = automatic)
setting-resources-cpu-percent = CPU share for indexing (%)
setting-resources-background-priority = Index at background priority (keeps the computer responsive)
setting-resources-resume-interrupted = Finish an interrupted import at the next start
setting-resources-memory-budget-mb = Memory budget for indexing (MB, 0 = none)
setting-resources-gpu-budget = Limit the GPU for the local model
setting-resources-gpu-layers = GPU layers while limited (0 = CPU only)
setting-resources-onnx-provider = ONNX accelerator (falls back to the CPU)
setting-resources-onnx-provider-cpu = CPU
setting-resources-onnx-provider-auto = Best available
setting-resources-onnx-provider-coreml = Core ML (macOS)
setting-resources-onnx-provider-directml = DirectML (Windows)
setting-resources-onnx-provider-cuda = CUDA (NVIDIA)
setting-backup-interval-hours = Back up every (hours; 0 = only when asked)
setting-backup-keep = Backups to keep (0 = all)
setting-backup-encrypt = Encrypt backups (keep the recovery key below somewhere safe)
setting-backup-folder = Backup folder (empty = app data; a synced folder keeps a copy off this computer)
setting-net-offline = Offline mode (nothing leaves this computer)
setting-net-allow = Only these hosts (*.example.com for subdomains; empty = any)
setting-net-deny = Never these hosts
setting-net-proxy = Proxy (empty = system, direct = none, or http:// / socks5:// URL)
setting-net-no-proxy = Hosts that skip the proxy
setting-net-ca-files = Trust only these CA certificate files (PEM), e.g. a company proxy
setting-net-pins-off = Ignore certificate pins (behind a TLS-inspecting proxy)
setting-log-level = Log level
setting-log-file = Write a log file (logs folder in the app data)
setting-log-max-mb = Start a new file at (MB)
setting-log-keep = Old log files to keep
setting-telemetry-crash-reports = Send crash reports (Sentry)
setting-telemetry-sentry-dsn = Sentry DSN
setting-telemetry-analytics = Send anonymous usage statistics
setting-telemetry-analytics-provider = Analytics service
setting-telemetry-analytics-provider-posthog = PostHog
setting-telemetry-analytics-provider-aptabase = Aptabase
setting-telemetry-analytics-key = Analytics project key
setting-telemetry-analytics-host = Analytics host (empty = the service default)
setting-telemetry-traces = Send traces (OpenTelemetry)
setting-telemetry-otlp-endpoint = OTLP endpoint, e.g. http://localhost:4318
setting-mail-enabled = Send email (SMTP)
setting-mail-host = Mail server, e.g. smtp.gmail.com
setting-mail-port = Port (465 for SSL/TLS, 587 for STARTTLS)
setting-mail-security = Encryption
setting-mail-security-tls = SSL/TLS
setting-mail-security-starttls = STARTTLS
setting-mail-security-none = None (this computer only)
setting-mail-username = Login name (empty = no login)
setting-mail-from = Sender, e.g. Me <me@example.com>
setting-mail-max-mb = Largest message (MB)
setting-mail-timeout-secs = Timeout (seconds)
setting-oauth-enabled = Allow signing in (OAuth 2 / OpenID Connect in the browser)
setting-oauth-redirect = The browser comes back through
setting-oauth-redirect-loopback = 127.0.0.1 (works everywhere)
setting-oauth-redirect-deep-link = The app's deep link (needs the deep-link plugin)
setting-oauth-timeout-secs = Time to finish signing in (seconds)
setting-oauth-refresh-before-secs = Refresh tokens this long before they expire (seconds)
setting-jobs-enabled = Run background jobs and schedules
setting-jobs-workers = Jobs at once
setting-jobs-max-attempts = Attempts before a job is dead
setting-jobs-backoff-base-secs = First retry after about (seconds, doubling)
setting-jobs-backoff-max-secs = Longest wait between retries (seconds)
setting-jobs-history-days = Keep the run history for (days)
setting-plugins-enabled = Run plugins
setting-plugins-disabled = Leave out these plugins (names)
setting-rclone-enabled = Sync to the cloud with rclone
setting-rclone-remote = Remote (its name in the rclone config)
setting-rclone-path = Folder on the remote (empty = its root)
setting-rclone-folder = What to sync
setting-rclone-folder-backups = Backups folder
setting-rclone-folder-data = Data folder (up only, no secrets)
setting-rclone-direction = Direction
setting-rclone-direction-push = Up (copy, never delete)
setting-rclone-direction-pull = Down (copy, never delete)
setting-rclone-direction-mirror = Mirror up (deletes extra files there)
setting-rclone-direction-bisync = Both ways (bisync)
setting-rclone-interval-minutes = Sync every (minutes, 0 = only when asked)
setting-rclone-bandwidth = Bandwidth limit, e.g. 2M or 10M:1M (empty = none)
setting-tailscale-enabled = Use Tailscale
setting-tailscale-serve = Share a local port with my tailnet (tailscale serve)
setting-tailscale-port = Local port to share (on 127.0.0.1)
setting-tailscale-https-port = HTTPS port on the tailnet
setting-webview-enabled = Open allowed web pages inside the app
setting-webview-allow = Allowed hosts (example.com, or *.example.com for subdomains too)
setting-webview-blocked = Other pages
setting-webview-blocked-refuse = Refuse them
setting-webview-blocked-browser = Open them in the system browser
setting-webview-isolation = Cookies and sign-ins
setting-webview-isolation-profile = Kept, separately for each profile
setting-webview-isolation-incognito = Never kept (incognito)
setting-webview-allow-http = Also show plain HTTP pages on allowed hosts
setting-webview-max-windows = Most web view windows open at once
setting-webview-bridge = Message bridge for trusted pages (window.kitchenBridge)
setting-webview-bridge-hosts = Hosts that may use the bridge (must be allowed above)
setting-http-api-enabled = Serve the local HTTP API
setting-http-api-host = Listen on (127.0.0.1 = this computer only)
setting-http-api-port = Port (0 = any free port)
setting-http-api-cors-origins = Browser origins allowed (CORS)
setting-http-api-max-body-kb = Largest request (KB)
setting-http-api-timeout-secs = Request timeout (seconds)
setting-telemetry-otlp-headers = OTLP headers (key=value, comma separated)
setting-telemetry-environment = Environment name
setting-watch-enabled = Watch folders and ingest new files
setting-watch-drop-folder = Use the drop folder
setting-watch-drop-folder-path = Drop folder (empty = Documents/<app> Inbox)
setting-watch-paths = More folders to watch
setting-watch-recursive = Include subfolders
setting-watch-debounce-ms = Wait after the last change (ms)
setting-watch-ingest-existing = Ingest files already there
setting-watch-remove-deleted = Forget deleted files
setting-watch-ignore = Ignore names (wildcards)
setting-appearance-mode = Light or dark
setting-appearance-mode-system = Follow the system
setting-appearance-mode-light = Light
setting-appearance-mode-dark = Dark
setting-appearance-theme = Theme
setting-appearance-theme-kitchen = Kitchen
setting-appearance-theme-nord = Nord
setting-appearance-theme-solarized = Solarized
setting-appearance-theme-forest = Forest
setting-appearance-theme-rose = Rosé
setting-appearance-theme-contrast = High contrast
setting-appearance-ui-font = Interface font (empty = system; a preset or any installed or bundled font)
setting-appearance-mono-font = Code and editor font (empty = system)
setting-appearance-font-size = Text size (px)
setting-tray-enabled = Show an icon in the tray (menu bar on macOS)
setting-tray-left-click-opens = Left click opens the window (right click: the menu)
setting-tray-close-to-tray = Closing the window keeps the app in the tray
setting-tray-show-in-menu-bar-only = macOS: menu bar only, no Dock icon
setting-updates-channel = Update channel
setting-updates-channel-stable = Stable
setting-updates-channel-beta = Beta (pre-releases as well)

## Profiles.

profiles-open-now = { " " }(open now)
profiles-switch = Switch
profiles-remove = Remove (keeps its files)
profiles-new = New profile name
profiles-add = Add profile

## Backups.

backup-running = Backing up…
backup-done = { $chunks ->
    [one] Backed up one chunk.
   *[other] Backed up { $chunks } chunks.
}
backup-restoring = Restoring…
backup-restored = Restored { $chunks } chunks. The index it replaced was backed up first.
backup-now = Back up now
backup-none = No backups yet.
backup-item = { $created }: { $chunks } chunks from { $sources } sources ({ $kb } KB)
backup-encrypted = , encrypted
backup-restore = Restore
backup-show-key = Show the recovery key
backup-key-hint = Keep it somewhere safe: an encrypted backup cannot be restored on another computer without it.
backup-key-label = Recovery key for a backup from another computer (optional)

## Email.

mail-password-saved = Password saved.
mail-sending = Sending to { $to }…
mail-connecting = Connecting…
mail-sent = Sent. Check { $to }.
mail-logged-in = Connected and logged in.
mail-password = SMTP password
mail-password-kept = Password saved (type to replace)
mail-password-placeholder = Password or app password
mail-password-save = Save password
mail-password-remove = Remove password

## AI keys.

ai-key-saved = Key saved.
ai-key-removed = Key removed.
ai-key-provider = Key for provider
ai-key = API key
ai-key-save = Save key
ai-keys-set = Keys set: { $list }
ai-keys-none = No keys set.

## AI usage.

usage-unpriced = { " " }(+{ $count } unpriced)
usage-tokens = { $input } in / { $output } out, { $cached } cached
usage-today = Today ({ $day })
usage-month = This month ({ $month })
usage-title = AI usage
usage-period = Period
usage-calls = Calls
usage-tokens-head = Tokens
usage-cost = Cost
usage-of = { $used } of { $limit }
usage-over-override = over (override on)
usage-used-up = used up
usage-no-budgets = No budgets set.

## Background jobs.

jobs-queued = Queued { $kind }.
jobs-off = Jobs are kept but not run until you turn them on above and save.
jobs-type = Job type
jobs-payload = Payload (JSON)
jobs-run = Run now
jobs-queue = Queue:
jobs-queue-empty = The queue is empty.
jobs-attempt = { $state }, attempt { $attempt } of { $max },
jobs-now = now
jobs-at = at { $time }
jobs-cancel = Cancel
jobs-dead = Dead letters (out of attempts):
jobs-dead-item = { $count ->
    [one] after one attempt:
   *[other] after { $count } attempts:
}
jobs-retry = Retry
jobs-drop = Drop
jobs-schedules = Schedules:
jobs-schedules-none = No schedules.
jobs-every = every { $secs } s
jobs-paused = paused
jobs-next = next { $time }
jobs-resume = Resume
jobs-pause = Pause
jobs-history = Recent runs:
jobs-history-none = No runs yet.
jobs-run-item = attempt { $attempt }:

## Local HTTP API.

http-copied = { $what } copied.
http-listening = Listening on { $url }.
http-error = Not running: { $error }
http-off = Not running. Turn it on and save settings.
http-url = URL
http-copy-url = Copy URL
http-token = Token
http-copy-token = Copy token
http-token-hint = Send the token as Authorization: Bearer <token>. Keep it secret.

## Tray, menu and shortcuts.

keys-listening = Press keys…
keys-placeholder = Click, then press keys
tray-action = Action
tray-separator = — separator —
tray-label-placeholder = Label (empty = default)
tray-label = Menu label
tray-on = on
tray-up = Move up
tray-down = Move down
tray-add-item = Add an item
tray-menu = Tray menu
tray-app-menu = App menu
tray-menu-title = Title
tray-shortcuts = Global shortcuts
tray-shortcuts-hint = They work while other apps are in front.
tray-shortcuts-none = None yet. They work while other apps are in front.
tray-add-shortcut = Add a shortcut
