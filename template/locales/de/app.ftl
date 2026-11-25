# German translation of locales/en/app.ftl. Keep the ids; translate the text.
# `kitchen i18n check` lists messages missing here or no longer in English.

## The language picker shows this name (in this language).

language-name = Deutsch

## Tray, app menu and global shortcuts (Rust: src-tauri/src/tray.rs); action-done-*: what an action reports back.

action-show-hide = Fenster zeigen/verbergen
action-show = Fenster zeigen
action-hide = Fenster verbergen
action-search = Suchen…
action-ingest-clipboard = Text aus der Zwischenablage hinzufügen
action-ingest-drop-folder = Ablageordner importieren
action-pause-resume = Hintergrundarbeit anhalten
action-resume = Hintergrundarbeit fortsetzen
action-settings = Einstellungen…
action-quit = Beenden
action-done-clipboard-empty = Die Zwischenablage enthält keinen Text.
action-done-clipboard = { $chunks ->
    [one] Text aus der Zwischenablage hinzugefügt (ein Abschnitt).
   *[other] Text aus der Zwischenablage hinzugefügt ({ $chunks } Abschnitte).
}
action-done-no-drop-folder = Den Ablageordner { $folder } gibt es noch nicht.
action-done-imported = Ablageordner importiert: { $files ->
    [one] eine Datei
   *[other] { $files } Dateien
}, { $chunks ->
    [one] ein Abschnitt
   *[other] { $chunks } Abschnitte
}.
action-done-paused = Hintergrundarbeit angehalten.
action-done-resumed = Hintergrundarbeit fortgesetzt.
menu-actions = Aktionen

## The test email (Rust: src-tauri/src/service.rs).

mail-test-subject = Test-E-Mail
mail-test-body = E-Mails aus dieser App funktionieren.
mail-test-login = Anmeldung testen
mail-test-send = Test-E-Mail an mich senden

## Error messages from the backend (Rust: src-tauri/src/error.rs). The error code stays the same in every language.

error-not-ready = Startet noch; bitte gleich noch einmal versuchen.
error-embedding = Einbettung: { $detail }
error-store = Speicher: { $detail }
error-ai = KI: { $detail }
error-not-built = { $part } ist in diesem Build nicht enthalten

## Main screen (ui-react/src/App.jsx).

starting = Wird gestartet…
setup-failed = Einrichtung fehlgeschlagen: { $error }
upgrade-label = Datenbank-Upgrade
upgrade-go = Datenbank aktualisieren
upgrade-later = Nicht jetzt
footer-status = { $app } · { $chunks ->
    [one] { $chunks } Abschnitt
   *[other] { $chunks } Abschnitte
} · Speicher { $backend } · { $embedder }
footer-keyword-search = Stichwortsuche
footer-env = { $env }-Build
clear-confirm = Zum Löschen von allem noch einmal klicken
clear = Alle Daten löschen

## First-run setup wizard.

wizard-title = Willkommen
wizard-intro-device = Eine einmalige Einrichtung. Alles bleibt auf diesem Gerät.
wizard-intro = Eine einmalige Einrichtung. Alles bleibt auf diesem Computer.
wizard-store = 1. Wo deine Daten liegen
wizard-store-file = Eine einfache Datei (schnell, kein Download)
wizard-store-postgres = Eine private Postgres-Datenbank mit Vektorsuche (lädt beim ersten Mal etwa 30 MB)
wizard-no-postgres = Dieser Prozessor kann die mitgelieferte Datenbank nicht ausführen; der Dateispeicher funktioniert genauso.
wizard-model = 2. Intelligente Suche
wizard-model-embed-bundled = Nach Bedeutung suchen mit einem lokalen KI-Modell (in die App eingebaut, kein Download; nichts wird gesendet)
wizard-model-embed = Nach Bedeutung suchen mit einem lokalen KI-Modell (einmaliger Download, nichts wird gesendet)
wizard-go-title = 3. Einrichten
wizard-go = Einrichten

## Update banner.

update-downloading-percent = Wird geladen… { $percent } %
update-downloading = Wird geladen…
update-failed = Aktualisierung fehlgeschlagen: { $error }
update-available = Version { $version } ist verfügbar (installiert: { $current }).
update-install = Installieren und neu starten

## Add documents.

add-reading = { $path } wird gelesen…
add-file-done = { $count ->
    [one] Ein Abschnitt aus { $path } hinzugefügt.
   *[other] { $count } Abschnitte aus { $path } hinzugefügt.
}
add-reading-folder = Alle Dokumente in { $path } werden gelesen…
add-folder-failed = { $files } Dateien hinzugefügt ({ $chunks } Abschnitte), { $failed } fehlgeschlagen: { $error }.
add-folder-done = { $files ->
    [one] Eine Datei
   *[other] { $files } Dateien
} hinzugefügt ({ $chunks } Abschnitte).
add-text-done = { $count ->
    [one] Ein Abschnitt hinzugefügt.
   *[other] { $count } Abschnitte hinzugefügt.
}
add-title = Dokumente hinzufügen
add-hint-mobile = Text einfügen oder den Pfad einer Datei in dieser App angeben.
add-hint = Dateien auf dieses Fenster ziehen (PDF, .txt, .md), einen Dateipfad angeben oder Text einfügen.
add-path-placeholder = /pfad/zur/datei.pdf
add-path-label = Datei- oder Ordnerpfad
add-file = Datei hinzufügen
add-folder = Ordner hinzufügen
add-paste-placeholder = Text hier einfügen
add-paste-label = Hinzuzufügender Text
add-name = Name für den eingefügten Text
add-text = Text hinzufügen

## Search.

search-title = Suche
search-placeholder = Wonach suchst du?
search-go = Suchen
search-none = Nichts gefunden.
search-page = , Seite { $page }
search-vector-score = { " " }· Vektor { $score }
search-keyword-score = { " " }· Stichwort { $score }

## Ask (AI).

ask-tokens = { $input } ein / { $output } aus Tokens
ask-cached-tokens = { $count } aus dem Cache
ask-failed-tries = { $count ->
    [one] nach einem gescheiterten Versuch
   *[other] nach { $count } gescheiterten Versuchen
}
ask-from-cache = aus dem Antwort-Cache
ask-title = Fragen
ask-placeholder = Frag etwas zu deinen Dokumenten
ask-question = Frage
ask-thinking = Denkt nach…
ask-go = Fragen

## Dictation and speech.

dictate-nothing = Keine Sprache erkannt.
dictate-mic-error = Mikrofon: { $error }
dictate-stop = ■ Stopp
dictate-stop-title = Stoppen und transkribieren
dictate = Diktieren
dictate-file-title = Eine Audiodatei transkribieren
dictate-file = Audiodatei
speak-preparing = Wird vorbereitet…
speak = Vorlesen

## Background agent, Python example and Claude Desktop.

agent-title = Hintergrund
agent-toggle = Das Backend nach dem Schließen des Fensters weiterlaufen lassen und bei der Anmeldung starten
agent-on = An: { $kind } unter { $location }
agent-off = Aus ({ $kind })
python-title = Python-Beispiel
python-hint = Sendet diesen Text an einen separaten Python-Prozess:
python-go = Textstatistik in Python
claude-registered-backup = Claude-Desktop-Konfiguration { $status } (Sicherung: { $backup }). Die Werkzeuge werden einmal vorbereitet…
claude-registered = Claude-Desktop-Konfiguration { $status }. Die Werkzeuge werden einmal vorbereitet…
claude-ready = Fertig. Starte Claude Desktop neu, um es zu verwenden.
claude-retry = Verbunden; der erste Start ist fehlgeschlagen und wird beim Start von Claude wiederholt.
claude-title = Claude Desktop
claude-hint = Fügt Claude Desktop die Werkzeuge dieser App hinzu (deine aktuellen Einstellungen werden vorher gesichert).
claude-go = Claude Desktop verbinden

## The app shell: custom titlebar.

titlebar-settings-keys = Einstellungen ({ $keys })
titlebar-settings = Einstellungen
titlebar-minimize = Minimieren
titlebar-restore = Wiederherstellen
titlebar-maximize = Maximieren
titlebar-close = Schließen

## Settings (ui-react/src/Settings.jsx).

settings-saved = Gespeichert.
settings-not-saved = Nicht gespeichert: { $error }
settings-title = Einstellungen
settings-sections = Bereiche der Einstellungen
settings-save = Einstellungen speichern
settings-reset = Auf Standard zurücksetzen
save-first = Zuerst die Einstellungen speichern.
remove = Entfernen
refresh = Aktualisieren

## Settings sections.

section-profiles = Profile
section-appearance = Darstellung
section-language = Sprache
section-search = Suche
section-postgres = Datenbank
section-ingest = Import
section-llm = KI
section-mcp = MCP-Server
section-tts = Sprachausgabe
section-stt = Diktat
section-telemetry = Datenschutz
section-log = Protokolle
section-resources = Leistung
section-net = Netzwerk
section-backup = Sicherungen
section-mail = E-Mail
section-oauth = Anmeldung
section-plugins = Plugins
section-jobs = Aufgaben
section-rclone = Cloud-Sync
section-tailscale = Tailnet
section-http-api = HTTP-API
section-webview = Webansichten
section-watch = Überwachte Ordner
section-service = Dienst
section-tray = Tray & Tastenkürzel
section-updates = Updates
section-about = Über
section-kv = Schlüssel-Wert-Speicher
section-cache = Cache

## Settings fields: setting-<section>-<key>, and setting-<section>-<key>-<option> for the choices of a list.

setting-ui-language = Sprache
    .help = Menüs und Meldungen wechseln sofort. Fehlende Übersetzungen erscheinen auf Englisch.
setting-ui-language-system = Wie das System
setting-ui-digits = Ziffern
setting-ui-digits-latin = Lateinisch (0123456789)
setting-ui-digits-native = Die der Sprache, z. B. arabisch-indisch (٠١٢٣٤٥٦٧٨٩)
setting-search-mode = Suchmodus
setting-search-top-k = Ergebnisse
setting-search-candidates = Kandidaten pro Liste (hybrid)
setting-search-vector-weight = Vektorgewicht
setting-search-keyword-weight = Stichwortgewicht
setting-search-rrf-k = RRF k
setting-search-min-vector-score = Mindestwert Vektor (Kosinus)
setting-search-min-keyword-score = Mindestwert Stichwort
setting-search-min-score = Mindestwert gesamt
setting-search-keyword-engine = Stichwortsuche (Postgres)
setting-search-bm25-tokenizer = BM25-Tokenizer-Modell
setting-search-fulltext-language = Volltextsprache
setting-postgres-upgrade = Wenn eine neue Version ein neueres Postgres mitbringt
setting-postgres-upgrade-ask = Vorher fragen
setting-postgres-upgrade-auto = Beim Start aktualisieren
setting-postgres-upgrade-off = Alte Version behalten
setting-postgres-upgrade-copy = Durch Kopieren aktualisieren (langsamer, doppelter Speicherplatz)
setting-ingest-workers = Import-Threads (0 = einer pro Kern)
setting-ingest-chunk-chars = Abschnittsgröße (Zeichen)
setting-ingest-chunk-overlap = Abschnittsüberlappung (Zeichen)
setting-ingest-embed-batch = Abschnitte pro Einbettungsaufruf
setting-ingest-max-file-mb = Größte Datei (MB)
setting-ingest-extensions = Dateitypen
setting-ingest-pdf-isolate = PDFs in einem separaten, begrenzten Prozess lesen
setting-ingest-pdf-timeout-secs = Zeitlimit für PDFs (Sekunden)
setting-ingest-pdf-memory-mb = Speicherlimit für PDFs (MB)
setting-llm-enabled = KI-Antworten einschalten
setting-llm-provider = Anbieter
setting-llm-model = Modell (leer = Standard des Anbieters)
setting-llm-base-url = Server-URL (leer = Standard des Anbieters)
setting-llm-local-model-path = Lokale Modelldatei (.gguf)
setting-llm-local-model-url = Download-URL des lokalen Modells
setting-llm-local-context = Lokaler Kontext (Tokens)
setting-llm-local-gpu-layers = Schichten auf der GPU (0 = nur CPU)
setting-llm-local-threads = CPU-Threads (0 = einer pro Kern)
setting-llm-local-keep-loaded = Lokales Modell im Speicher halten
setting-llm-temperature = Temperatur (leer = Standard des Modells)
setting-llm-max-tokens = Längste Antwort (Tokens)
setting-llm-timeout-secs = Zeitlimit (Sekunden)
setting-llm-retries = Wiederholungen vor dem nächsten Anbieter
setting-llm-retry-base-ms = Erste Wartezeit (ms, verdoppelt sich je Versuch)
setting-llm-retry-max-secs = Längste Wartezeit (s; ein längeres Retry-After wechselt zum nächsten Anbieter)
setting-llm-daily-token-budget = Tägliches Token-Budget
setting-llm-monthly-token-budget = Monatliches Token-Budget
setting-llm-budget-daily-usd = Tagesbudget (USD)
setting-llm-budget-monthly-usd = Monatsbudget (USD)
setting-llm-budget-warn-percent = Warnen bei (% eines Budgets; 0 = nie)
setting-llm-budget-override-limits = Über das Budget hinaus weitermachen (Übersteuerung)
setting-llm-prompt-cache = Prompt-Caching (Anthropic cache_control; OpenAI und Gemini cachen selbst)
setting-llm-system-prompt = Systemprompt (leer = eingebaut)
setting-llm-http-hosts = Hosts, die einfaches HTTP verwenden dürfen
setting-llm-cache-enabled = Antworten auf wiederholte Fragen wiederverwenden (wird bei Dokumentänderungen geleert)
setting-llm-cache-ttl-hours = Zwischengespeicherte Antworten behalten (Stunden)
setting-llm-cache-max-entries = Höchstzahl zwischengespeicherter Antworten
setting-llm-cache-similarity = Ähnliche Fragen erkennen (Kosinus, 1 = nur exakt)
setting-mcp-enabled = MCP-Clients (Claude Desktop, Claude Code, …) diese Dokumente durchsuchen lassen
setting-mcp-allow-write = Auch Werkzeuge erlauben, die Daten ändern (hinzufügen, entfernen)
setting-mcp-tools = Nur diese Werkzeuge (leer = alle erlaubten)
setting-mcp-audit-log = Prüfprotokoll der Werkzeugaufrufe führen
setting-mcp-audit-arguments = Auch die Aufrufargumente protokollieren
setting-mcp-max-result-chars = Längste Antwort (Zeichen)
setting-mcp-approval-timeout-secs = Auf Freigabe von „Fragen“-Werkzeugen warten (Sekunden)
setting-tts-enabled = Antworten vorlesen (tiny-voca, auf diesem Computer)
setting-tts-voice = Stimme
setting-tts-voice-0 = F2 (weiblich)
setting-tts-voice-1 = M2 (männlich)
setting-tts-speed = Geschwindigkeit
setting-tts-pitch-shift = Tonhöhenverschiebung
setting-tts-pitch-range = Tonhöhenumfang
setting-tts-pause-ms = Pause zwischen Sätzen (ms)
setting-tts-threads = CPU-Threads (0 = automatisch)
setting-stt-enabled = Diktat und Audiodateien (Whisper, auf diesem Computer)
setting-stt-model = Modell
setting-stt-model-tiny = Tiny (40 MB, am schnellsten)
setting-stt-model-base = Base (75 MB)
setting-stt-model-small = Small (250 MB, am genauesten)
setting-stt-language = Sprachcode, z. B. en, de (leer = erkennen)
setting-stt-translate = Ins Englische übersetzen
setting-stt-max-seconds = Längste Aufnahme (Sekunden)
setting-stt-threads = CPU-Threads (0 = automatisch)
setting-resources-cpu-percent = CPU-Anteil fürs Indizieren (%)
setting-resources-background-priority = Mit Hintergrundpriorität indizieren (der Computer bleibt reaktionsfähig)
setting-resources-resume-interrupted = Einen unterbrochenen Import beim nächsten Start beenden
setting-resources-memory-budget-mb = Speicherbudget fürs Indizieren (MB, 0 = keins)
setting-resources-gpu-budget = GPU für das lokale Modell begrenzen
setting-resources-gpu-layers = GPU-Schichten bei Begrenzung (0 = nur CPU)
setting-resources-onnx-provider = ONNX-Beschleuniger (fällt auf die CPU zurück)
setting-resources-onnx-provider-cpu = CPU
setting-resources-onnx-provider-auto = Bester verfügbarer
setting-resources-onnx-provider-coreml = Core ML (macOS)
setting-resources-onnx-provider-directml = DirectML (Windows)
setting-resources-onnx-provider-cuda = CUDA (NVIDIA)
setting-backup-interval-hours = Sichern alle (Stunden; 0 = nur auf Anfrage)
setting-backup-keep = Aufzubewahrende Sicherungen (0 = alle)
setting-backup-encrypt = Sicherungen verschlüsseln (den Wiederherstellungsschlüssel unten sicher aufbewahren)
setting-backup-folder = Sicherungsordner (leer = App-Daten; ein synchronisierter Ordner hält eine Kopie außerhalb dieses Computers)
setting-net-offline = Offline-Modus (nichts verlässt diesen Computer)
setting-net-allow = Nur diese Hosts (*.example.com für Subdomains; leer = alle)
setting-net-deny = Niemals diese Hosts
setting-net-proxy = Proxy (leer = System, direct = keiner, oder eine http://- / socks5://-URL)
setting-net-no-proxy = Hosts ohne Proxy
setting-net-ca-files = Nur diesen CA-Zertifikatsdateien (PEM) vertrauen, z. B. eines Firmen-Proxys
setting-net-pins-off = Zertifikats-Pins ignorieren (hinter einem TLS-prüfenden Proxy)
setting-log-level = Protokollstufe
setting-log-file = Protokolldatei schreiben (Ordner logs in den App-Daten)
setting-log-max-mb = Neue Datei beginnen ab (MB)
setting-log-keep = Alte Protokolldateien behalten
setting-telemetry-crash-reports = Absturzberichte senden (Sentry)
setting-telemetry-sentry-dsn = Sentry-DSN
setting-telemetry-analytics = Anonyme Nutzungsstatistiken senden
setting-telemetry-analytics-provider = Analysedienst
setting-telemetry-analytics-provider-posthog = PostHog
setting-telemetry-analytics-provider-aptabase = Aptabase
setting-telemetry-analytics-key = Projektschlüssel für Analysen
setting-telemetry-analytics-host = Analyse-Host (leer = Standard des Dienstes)
setting-telemetry-traces = Traces senden (OpenTelemetry)
setting-telemetry-otlp-endpoint = OTLP-Endpunkt, z. B. http://localhost:4318
setting-mail-enabled = E-Mail senden (SMTP)
setting-mail-host = Mailserver, z. B. smtp.gmail.com
setting-mail-port = Port (465 für SSL/TLS, 587 für STARTTLS)
setting-mail-security = Verschlüsselung
setting-mail-security-tls = SSL/TLS
setting-mail-security-starttls = STARTTLS
setting-mail-security-none = Keine (nur dieser Computer)
setting-mail-username = Anmeldename (leer = ohne Anmeldung)
setting-mail-from = Absender, z. B. Ich <ich@example.com>
setting-mail-max-mb = Größte Nachricht (MB)
setting-mail-timeout-secs = Zeitlimit (Sekunden)
setting-oauth-enabled = Anmeldung erlauben (OAuth 2 / OpenID Connect im Browser)
setting-oauth-redirect = Der Browser kehrt zurück über
setting-oauth-redirect-loopback = 127.0.0.1 (funktioniert überall)
setting-oauth-redirect-deep-link = Den Deep Link der App (braucht das Deep-Link-Plugin)
setting-oauth-timeout-secs = Zeit für die Anmeldung (Sekunden)
setting-oauth-refresh-before-secs = Tokens so lange vor Ablauf erneuern (Sekunden)
setting-jobs-enabled = Hintergrundaufgaben und Zeitpläne ausführen
setting-jobs-workers = Gleichzeitige Aufgaben
setting-jobs-max-attempts = Versuche, bevor eine Aufgabe scheitert
setting-jobs-backoff-base-secs = Erste Wiederholung nach etwa (Sekunden, verdoppelnd)
setting-jobs-backoff-max-secs = Längste Wartezeit zwischen Wiederholungen (Sekunden)
setting-jobs-history-days = Verlauf aufbewahren (Tage)
setting-plugins-enabled = Plugins ausführen
setting-plugins-disabled = Diese Plugins auslassen (Namen)
setting-rclone-enabled = Mit rclone in die Cloud synchronisieren
setting-rclone-remote = Remote (sein Name in der rclone-Konfiguration)
setting-rclone-path = Ordner auf dem Remote (leer = sein Stammordner)
setting-rclone-folder = Was synchronisiert wird
setting-rclone-folder-backups = Sicherungsordner
setting-rclone-folder-data = Datenordner (nur hoch, keine Geheimnisse)
setting-rclone-direction = Richtung
setting-rclone-direction-push = Hoch (kopieren, nie löschen)
setting-rclone-direction-pull = Herunter (kopieren, nie löschen)
setting-rclone-direction-mirror = Hoch spiegeln (löscht dort überzählige Dateien)
setting-rclone-direction-bisync = In beide Richtungen (bisync)
setting-rclone-interval-minutes = Synchronisieren alle (Minuten, 0 = nur auf Anfrage)
setting-rclone-bandwidth = Bandbreitenlimit, z. B. 2M oder 10M:1M (leer = keins)
setting-tailscale-enabled = Tailscale verwenden
setting-tailscale-serve = Einen lokalen Port mit meinem Tailnet teilen (tailscale serve)
setting-tailscale-port = Zu teilender lokaler Port (auf 127.0.0.1)
setting-tailscale-https-port = HTTPS-Port im Tailnet
setting-webview-enabled = Erlaubte Webseiten in der App öffnen
setting-webview-allow = Erlaubte Hosts (example.com oder *.example.com auch für Subdomains)
setting-webview-blocked = Andere Seiten
setting-webview-blocked-refuse = Ablehnen
setting-webview-blocked-browser = Im Systembrowser öffnen
setting-webview-isolation = Cookies und Anmeldungen
setting-webview-isolation-profile = Behalten, getrennt pro Profil
setting-webview-isolation-incognito = Nie behalten (inkognito)
setting-webview-allow-http = Auch einfache HTTP-Seiten auf erlaubten Hosts zeigen
setting-webview-max-windows = Höchstzahl gleichzeitig offener Webansichten
setting-webview-bridge = Nachrichtenbrücke für vertrauenswürdige Seiten (window.kitchenBridge)
setting-webview-bridge-hosts = Hosts, die die Brücke nutzen dürfen (müssen oben erlaubt sein)
setting-http-api-enabled = Die lokale HTTP-API bereitstellen
setting-http-api-host = Lauschen auf (127.0.0.1 = nur dieser Computer)
setting-http-api-port = Port (0 = ein beliebiger freier Port)
setting-http-api-cors-origins = Erlaubte Browser-Ursprünge (CORS)
setting-http-api-max-body-kb = Größte Anfrage (KB)
setting-http-api-timeout-secs = Zeitlimit pro Anfrage (Sekunden)
setting-telemetry-otlp-headers = OTLP-Header (Schlüssel=Wert, durch Kommas getrennt)
setting-telemetry-environment = Name der Umgebung
setting-watch-enabled = Ordner überwachen und neue Dateien importieren
setting-watch-drop-folder = Den Ablageordner verwenden
setting-watch-drop-folder-path = Ablageordner (leer = Dokumente/<App> Eingang)
setting-watch-paths = Weitere überwachte Ordner
setting-watch-recursive = Unterordner einbeziehen
setting-watch-debounce-ms = Nach der letzten Änderung warten (ms)
setting-watch-ingest-existing = Bereits vorhandene Dateien importieren
setting-watch-remove-deleted = Gelöschte Dateien vergessen
setting-watch-ignore = Namen ignorieren (Platzhalter)
setting-appearance-mode = Hell oder dunkel
setting-appearance-mode-system = Wie das System
setting-appearance-mode-light = Hell
setting-appearance-mode-dark = Dunkel
setting-appearance-theme = Design
setting-appearance-theme-kitchen = Kitchen
setting-appearance-theme-nord = Nord
setting-appearance-theme-solarized = Solarized
setting-appearance-theme-forest = Wald
setting-appearance-theme-rose = Rosé
setting-appearance-theme-contrast = Hoher Kontrast
setting-appearance-ui-font = Schrift der Oberfläche (leer = System; eine Vorgabe oder jede installierte oder mitgelieferte Schrift)
setting-appearance-mono-font = Schrift für Code und Editor (leer = System)
setting-appearance-font-size = Textgröße (px)
setting-tray-enabled = Ein Symbol im Tray zeigen (Menüleiste unter macOS)
setting-tray-left-click-opens = Linksklick öffnet das Fenster (Rechtsklick: das Menü)
setting-tray-close-to-tray = Beim Schließen des Fensters bleibt die App im Tray
setting-tray-show-in-menu-bar-only = macOS: nur Menüleiste, kein Dock-Symbol
setting-updates-channel = Update-Kanal
setting-updates-channel-stable = Stabil
setting-updates-channel-beta = Beta (auch Vorabversionen)
setting-kv-ui-namespaces = Namensräume für das Fenster, die HTTP-API und die MCP-Tools
setting-kv-encrypted-namespaces = Verschlüsselt gespeicherte Namensräume
setting-kv-key-source = Schlüssel liegt
setting-kv-key-source-file = In einer Schlüsseldatei im Datenordner
setting-kv-key-source-keychain = Im Schlüsselbund des Systems (braucht Anmeldeunterstützung)
setting-kv-max-size-mb = Größter Speicher (MB)
setting-kv-max-value-kb = Größter Wert (KB)
setting-kv-sweep-minutes = Abgelaufene Schlüssel löschen alle (Minuten)
setting-kv-backup = Den Speicher in Sicherungen aufnehmen
setting-cache-max-mb = Größter Cache (MB)
setting-cache-ttl-secs = Einträge behalten für (Sekunden, 0 = bis verdrängt)
setting-cache-tti-secs = Ungenutzte Einträge verwerfen nach (Sekunden, 0 = nie)
setting-cache-persist = Einträge über Neustarts behalten (im Schlüssel-Wert-Speicher)

## Profiles.

profiles-open-now = { " " }(jetzt geöffnet)
profiles-switch = Wechseln
profiles-remove = Entfernen (Dateien bleiben)
profiles-new = Name des neuen Profils
profiles-add = Profil hinzufügen

## Backups.

backup-running = Sicherung läuft…
backup-done = { $chunks ->
    [one] Ein Abschnitt gesichert.
   *[other] { $chunks } Abschnitte gesichert.
}
backup-restoring = Wird wiederhergestellt…
backup-restored = { $chunks } Abschnitte wiederhergestellt. Der ersetzte Index wurde vorher gesichert.
backup-now = Jetzt sichern
backup-none = Noch keine Sicherungen.
backup-item = { $created }: { $chunks } Abschnitte aus { $sources } Quellen ({ $kb } KB)
backup-encrypted = , verschlüsselt
backup-restore = Wiederherstellen
backup-show-key = Wiederherstellungsschlüssel zeigen
backup-key-hint = Bewahre ihn sicher auf: Ohne ihn lässt sich eine verschlüsselte Sicherung auf einem anderen Computer nicht wiederherstellen.
backup-key-label = Wiederherstellungsschlüssel für eine Sicherung von einem anderen Computer (optional)

## Email.

mail-password-saved = Passwort gespeichert.
mail-sending = Wird an { $to } gesendet…
mail-connecting = Verbindung wird hergestellt…
mail-sent = Gesendet. Sieh in { $to } nach.
mail-logged-in = Verbunden und angemeldet.
mail-password = SMTP-Passwort
mail-password-kept = Passwort gespeichert (zum Ersetzen tippen)
mail-password-placeholder = Passwort oder App-Passwort
mail-password-save = Passwort speichern
mail-password-remove = Passwort entfernen

## AI keys.

ai-key-saved = Schlüssel gespeichert.
ai-key-removed = Schlüssel entfernt.
ai-key-provider = Schlüssel für Anbieter
ai-key = API-Schlüssel
ai-key-save = Schlüssel speichern
ai-keys-set = Hinterlegte Schlüssel: { $list }
ai-keys-none = Keine Schlüssel hinterlegt.

## AI usage.

usage-unpriced = { " " }(+{ $count } ohne Preis)
usage-tokens = { $input } ein / { $output } aus, { $cached } aus dem Cache
usage-today = Heute ({ $day })
usage-month = Dieser Monat ({ $month })
usage-title = KI-Nutzung
usage-period = Zeitraum
usage-calls = Aufrufe
usage-tokens-head = Tokens
usage-cost = Kosten
usage-of = { $used } von { $limit }
usage-over-override = überschritten (Übersteuerung an)
usage-used-up = aufgebraucht
usage-no-budgets = Keine Budgets festgelegt.

## Background jobs.

jobs-queued = { $kind } eingereiht.
jobs-off = Aufgaben bleiben erhalten, laufen aber erst, wenn du sie oben einschaltest und speicherst.
jobs-type = Aufgabentyp
jobs-payload = Nutzdaten (JSON)
jobs-run = Jetzt ausführen
jobs-queue = Warteschlange:
jobs-queue-empty = Die Warteschlange ist leer.
jobs-attempt = { $state }, Versuch { $attempt } von { $max },
jobs-now = jetzt
jobs-at = um { $time }
jobs-cancel = Abbrechen
jobs-dead = Gescheiterte Aufgaben (keine Versuche mehr):
jobs-dead-item = { $count ->
    [one] nach einem Versuch:
   *[other] nach { $count } Versuchen:
}
jobs-retry = Wiederholen
jobs-drop = Verwerfen
jobs-schedules = Zeitpläne:
jobs-schedules-none = Keine Zeitpläne.
jobs-every = alle { $secs } s
jobs-paused = angehalten
jobs-next = nächster Lauf { $time }
jobs-resume = Fortsetzen
jobs-pause = Anhalten
jobs-history = Letzte Läufe:
jobs-history-none = Noch keine Läufe.
jobs-run-item = Versuch { $attempt }:

## Local HTTP API.

http-copied = { $what } kopiert.
http-listening = Erreichbar unter { $url }.
http-error = Läuft nicht: { $error }
http-off = Läuft nicht. Schalte es ein und speichere die Einstellungen.
http-url = URL
http-copy-url = URL kopieren
http-token = Token
http-copy-token = Token kopieren
http-token-hint = Sende das Token als Authorization: Bearer <token>. Halte es geheim.

## Tray, menu and shortcuts.

keys-listening = Tasten drücken…
keys-placeholder = Klicken, dann Tasten drücken
tray-action = Aktion
tray-separator = — Trennlinie —
tray-label-placeholder = Beschriftung (leer = Standard)
tray-label = Menübeschriftung
tray-on = an
tray-up = Nach oben
tray-down = Nach unten
tray-add-item = Eintrag hinzufügen
tray-menu = Tray-Menü
tray-app-menu = App-Menü
tray-menu-title = Titel
tray-shortcuts = Globale Tastenkürzel
tray-shortcuts-hint = Sie funktionieren auch, wenn andere Apps im Vordergrund sind.
tray-shortcuts-none = Noch keine. Sie funktionieren auch, wenn andere Apps im Vordergrund sind.
tray-add-shortcut = Tastenkürzel hinzufügen

## The plain UI's settings (ui/index.html, ui/settings.html).

settings-json-hint = Such-, Import- und Ordnerüberwachungs-Einstellungen als JSON (fehlende Schlüssel nutzen die Standardwerte; alles Optionale ist aus).
settings-json-hint-all = Suche, Import und die übrigen Einstellungen als JSON (fehlende Schlüssel nutzen die Standardwerte; alles Optionale ist aus).
settings-everything-else = Alles andere
settings-bad-json = Kein gültiges JSON: { $error }
appearance-ui-font = Schrift der Oberfläche
appearance-mono-font = Schrift für Code und Editor
appearance-font-hint = Leer nutzt die Systemschrift. Eine Vorgabe oder der Name einer installierten oder mitgelieferten Schrift (siehe shell.css).
update-check = Nach Updates suchen
update-not-configured = Updates sind in diesem Build nicht eingerichtet (kein öffentlicher Updater-Schlüssel).
update-newest = Sie haben die neueste Version ({ $version }).
update-check-failed = Update-Prüfung fehlgeschlagen: { $error }
dictate-transcribing = Wird transkribiert…
dictate-listening = Hört zu…

## Watch folders.

watch-folders = Überwacht: { $folders }
watch-none = Keine Überwachung.
watch-ingested = { $files } Dateien hinzugefügt ({ $chunks } Abschnitte)
watch-ingested-failed = { $files } Dateien hinzugefügt ({ $chunks } Abschnitte), { $failed } fehlgeschlagen
watch-removed = { $path } vergessen
watch-error = Problem: { $message }
watch-started = { $count ->
    [one] Gestartet für einen Ordner
   *[other] Gestartet für { $count } Ordner
}

## Sign-in (OAuth).

oauth-kind-google = Google
oauth-kind-github = GitHub
oauth-kind-microsoft = Microsoft
oauth-kind-oidc = OpenID Connect (beliebiger Anbieter)
oauth-browser = Schließen Sie die Anmeldung im Browser ab…
oauth-pending = Schließen Sie die Anmeldung im Browser ab; danach öffnet sich die App wieder.
oauth-revoked = Abgemeldet, und das Token wurde widerrufen.
oauth-signed-out = Auf diesem Computer abgemeldet.
oauth-signed-in = Angemeldet.
oauth-signed-in-as = Angemeldet als { $who }.
oauth-not-signed-in = Nicht angemeldet.
oauth-unsaved = Speichern Sie die Einstellungen, um diesen Anbieter zu nutzen.
oauth-keychain = Schlüsselbund: { $error }
oauth-secret-saved = Client-Geheimnis im Schlüsselbund gespeichert.
oauth-secret-removed = Client-Geheimnis entfernt.
oauth-no-deep-link = Das Deep-Link-Plugin ist nicht eingebaut; die Weiterleitung an 127.0.0.1 wird genutzt.
oauth-provider = Anbieter
oauth-id = Name (zum Anmelden; z. B. google)
oauth-client-id = Client-ID
oauth-scopes = Bereiche (leer = die üblichen des Anbieters)
oauth-issuer = Aussteller-URL, z. B. https://id.example.com
oauth-tenant = Mandant (common, organizations, consumers oder eine Mandanten-ID)
oauth-secret = Client-Geheimnis
oauth-secret-kept = Client-Geheimnis gespeichert (zum Ersetzen tippen)
oauth-secret-placeholder = Client-Geheimnis, falls der Anbieter eines braucht
oauth-secret-save = Geheimnis speichern
oauth-secret-remove = Geheimnis entfernen
oauth-sign-out = Abmelden
oauth-sign-in-with = Mit { $provider } anmelden
oauth-remove = Anbieter entfernen
oauth-add = Anbieter hinzufügen
oauth-hint = Anmeldung einschalten, Einstellungen speichern, dann anmelden.

## Plugins.

plugins-none = Es sind keine Plugins eingebaut.
plugins-on = (an)
plugins-off = (aus)
plugins-methods = Methoden: { $list }.
json-invalid = (noch kein gültiges JSON)

## Cloud sync (rclone).

rclone-config-saved = Konfiguration gespeichert.
rclone-config-removed = Konfiguration entfernt.
rclone-synced = Synchronisiert.
rclone-ready = rclone ist bereit.
rclone-downloaded = rclone { $version }: heruntergeladen.
rclone-on-first-sync = rclone { $version }: wird bei der ersten Synchronisation geladen.
rclone-unsupported = rclone { $version }: für diesen Computer nicht verfügbar.
rclone-remotes = Remotes: { $list }.
rclone-remotes-none = keine
rclone-no-config = Noch keine Konfiguration.
rclone-last-ok = Letzte Synchronisation { $time } nach { $target }: ok.
rclone-last-failed = Letzte Synchronisation { $time } nach { $target }: fehlgeschlagen.
rclone-config-label = Remote-Konfiguration: Ausgabe von `rclone config show` einfügen (in einer privaten Datei gespeichert)
rclone-config-save = Konfiguration speichern
rclone-config-remove = Konfiguration entfernen
rclone-download = rclone herunterladen
rclone-sync = Jetzt synchronisieren

## Tailnet (Tailscale).

tailscale-done = Erledigt.
tailscale-missing = Tailscale ist nicht installiert. Laden Sie es hier und öffnen Sie dann die Einstellungen erneut:
tailscale-state = { $state }.
tailscale-state-as = { $state } als { $name }.
tailscale-state-in = { $state } in { $tailnet }.
tailscale-state-as-in = { $state } als { $name } in { $tailnet }.
tailscale-shared = Port { $port } ist freigegeben.
tailscale-shared-at = Port { $port } ist unter { $url } freigegeben.
tailscale-nothing = Nichts freigegeben.
tailscale-apply = Freigabe anwenden

## The backend as a service.

service-status = { $manager }: { $installed }, { $running }. Protokoll: { $log }.
service-installed = installiert
service-not-installed = nicht installiert
service-running = läuft
service-running-pid = läuft (PID { $pid })
service-not-running = läuft nicht
service-done-installed = Installiert.
service-done-removed = Entfernt.
service-toggle = Das Backend als Dienst ausführen: kein Fenster nötig, startet bei der Anmeldung, startet nach Fehlern neu
service-boot = Um es stattdessen beim Systemstart für den ganzen Computer zu starten, als Administrator ausführen:

## Web views.

webview-message = Nachricht von { $host }: { $data }
webview-opened = Geöffnet in { $label }.
webview-url = Zu öffnende Webseite
webview-open = Öffnen
webview-close = Schließen

## Key-value store and cache.

kv-empty = Noch keine Schlüssel.
kv-keys = { $count ->
    [one] ein Schlüssel
   *[other] { $count } Schlüssel
}
kv-encrypted = , verschlüsselt
kv-open-to-window = , für das Fenster freigegeben
cache-cleared = Cache geleert.
cache-stats = { $entries } Einträge, { $used } von { $max } · { $hits } Treffer, { $misses } Fehlschläge ({ $rate } % Trefferquote)
cache-clear = Cache leeren

## Downloads: voice, speech model, local AI model.

download-progress = Lade { $progress }
voice-ready = Stimme bereit.
voice-download = Stimme herunterladen (25 MB)
stt-ready = Modell bereit.
stt-download = Gewähltes Modell herunterladen (vorher Einstellungen speichern)
stt-download-whisper = Whisper-Modell herunterladen
model-progress = Lade: { $progress }
model-ready = Bereit: { $path }
model-download = Lokales Modell herunterladen

## Telemetry.

telemetry-off = Es wird nichts gesendet, solange Sie es oben nicht einschalten.
telemetry-crash-reports = Absturzberichte
telemetry-analytics = Nutzungsstatistiken
telemetry-traces = Traces
telemetry-nothing = nichts
telemetry-sending = Gesendet wird: { $what }. Anonyme ID { $id }…, { $sent } gesendet
telemetry-pending = { $count ->
    [one] , ein Absturzbericht auf diesem Computer aufbewahrt
   *[other] , { $count } Absturzberichte auf diesem Computer aufbewahrt
}
telemetry-last-error = . Letztes Problem: { $error }
telemetry-promise = . Gesendet werden nur Aufrufnamen, Zeiten und Zahlen, nie Ihre Dokumente oder Fragen.

## MCP server.

mcp-registered = Claude Desktop: { $result }. Starten Sie Claude Desktop neu, um es zu laden.
mcp-add-claude = Zu Claude Desktop hinzufügen
mcp-claude-code = Claude Code:
mcp-save-first = Speichern Sie zuerst die Einstellungen mit eingeschaltetem Server.
mcp-turn-on = Dies in den Einstellungen einschalten und speichern, dann einen Client verbinden:
mcp-every-other = Jedes andere Tool
mcp-tool = Tool { $tool }
mcp-default-allow = erlauben (Standard)
mcp-as-others = wie jedes andere Tool
mcp-allow = erlauben
mcp-ask = erst fragen
mcp-deny = ablehnen
mcp-waiting = { $count ->
    [one] Ein MCP-Aufruf wartet auf Sie:
   *[other] { $count } MCP-Aufrufe warten auf Sie:
}
mcp-none-waiting = Keine MCP-Aufrufe warten auf Freigabe.
mcp-approve = Erlauben
mcp-deny-call = Ablehnen

## Updates (release channels).

updates-going-back = Zurück zu { $version }…
updates-rollback-failed = Zurücksetzen fehlgeschlagen: { $error }
updates-save-hint = Speichern, um den Kanal zu wechseln; die nächste Update-Prüfung nutzt den neuen.
updates-bucket = Gestaffelte Versionen erreichen diese Installation, sobald sie an mehr als { $bucket } % verteilt sind.
updates-previous = Vorherige Version: { $version }
updates-kept = { " " }(auf diesem Computer aufbewahrt)
updates-download-again = { " " }(wird erneut aus ihrem Release geladen)
updates-rollback = Zurück zu { $version }
updates-none = Noch keine frühere Version zum Zurücksetzen.

## About: licences.

licences-title = { $count ->
    [one] Lizenzen (ein Paket)
   *[other] Lizenzen ({ $count } Pakete)
}
licences-filter = Nach Name oder Lizenz filtern
licences-package = Paket
licences-version = Version
licences-licence = Lizenz
licences-show = Lizenztexte zeigen
licences-hide = Lizenztexte ausblenden

## Web-App-Hülle: natives Menü und Einstellungsfenster.

wrapper-menu-preferences = Einstellungen…
wrapper-menu-edit = Bearbeiten
wrapper-menu-view = Darstellung
wrapper-menu-page = Seite
wrapper-menu-reload = Neu laden
wrapper-menu-back = Zurück
wrapper-menu-forward = Vorwärts
wrapper-menu-zoom-in = Vergrößern
wrapper-menu-zoom-out = Verkleinern
wrapper-menu-actual-size = Originalgröße
wrapper-menu-capture = Tastenerfassung ein/aus
wrapper-menu-mute = Stumm / Ton an
wrapper-menu-fill = Formular ausfüllen
wrapper-menu-open-in-browser = Im Browser öffnen…
wrapper-prefs-title = Einstellungen
wrapper-prefs-permissions = Berechtigungen
wrapper-prefs-site = Website
wrapper-prefs-certificate = Zertifikat
wrapper-prefs-keys = Tasten
wrapper-prefs-media = Medien
wrapper-prefs-features = Browserfunktionen
wrapper-prefs-headers = Anfrage-Header
wrapper-prefs-autofill = Formulare automatisch ausfüllen
