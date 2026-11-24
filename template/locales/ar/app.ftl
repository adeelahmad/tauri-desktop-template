# Arabic translation of locales/en/app.ftl. Keep the ids; translate the text.
# `kitchen i18n check` lists messages missing here or no longer in English.

## The language picker shows this name (in this language).

language-name = العربية

## Tray, app menu and global shortcuts (Rust: src-tauri/src/tray.rs); action-done-*: what an action reports back.

action-show-hide = إظهار النافذة أو إخفاؤها
action-show = إظهار النافذة
action-hide = إخفاء النافذة
action-search = بحث…
action-ingest-clipboard = إضافة نص الحافظة
action-ingest-drop-folder = استيراد مجلد الإسقاط
action-pause-resume = إيقاف العمل في الخلفية مؤقتًا
action-resume = استئناف العمل في الخلفية
action-settings = الإعدادات…
action-quit = إنهاء
action-done-clipboard-empty = لا يوجد نص في الحافظة.
action-done-clipboard = { $chunks ->
    [zero] أُضيف نص الحافظة (لا مقاطع).
    [one] أُضيف نص الحافظة (مقطع واحد).
    [two] أُضيف نص الحافظة (مقطعان).
    [few] أُضيف نص الحافظة ({ $chunks } مقاطع).
    [many] أُضيف نص الحافظة ({ $chunks } مقطعًا).
   *[other] أُضيف نص الحافظة ({ $chunks } مقطع).
}
action-done-no-drop-folder = مجلد الإسقاط { $folder } غير موجود بعد.
action-done-imported = تم استيراد مجلد الإسقاط: { $files ->
    [one] ملف واحد
    [two] ملفان
    [few] { $files } ملفات
    [many] { $files } ملفًا
   *[other] { $files } ملف
}، { $chunks ->
    [one] مقطع واحد
    [two] مقطعان
    [few] { $chunks } مقاطع
    [many] { $chunks } مقطعًا
   *[other] { $chunks } مقطع
}.
action-done-paused = تم إيقاف العمل في الخلفية مؤقتًا.
action-done-resumed = تم استئناف العمل في الخلفية.
menu-actions = إجراءات

## The test email (Rust: src-tauri/src/service.rs).

mail-test-subject = بريد تجريبي
mail-test-body = إرسال البريد من هذا التطبيق يعمل.
mail-test-login = اختبار تسجيل الدخول
mail-test-send = إرسال بريد تجريبي إلى نفسي

## Error messages from the backend (Rust: src-tauri/src/error.rs). The error code stays the same in every language.

error-not-ready = ما زال التطبيق يبدأ؛ حاول مرة أخرى بعد لحظة.
error-embedding = التضمين: { $detail }
error-store = المخزن: { $detail }
error-ai = الذكاء الاصطناعي: { $detail }
error-not-built = { $part } غير مضمّن في هذا الإصدار

## Main screen (ui-react/src/App.jsx).

starting = جارٍ البدء…
setup-failed = فشل الإعداد: { $error }
upgrade-label = ترقية قاعدة البيانات
upgrade-go = ترقية قاعدة البيانات
upgrade-later = ليس الآن
footer-status = { $app } · { $chunks ->
    [zero] لا مقاطع
    [one] مقطع واحد
    [two] مقطعان
    [few] { $chunks } مقاطع
    [many] { $chunks } مقطعًا
   *[other] { $chunks } مقطع
} · مخزن { $backend } · { $embedder }
footer-keyword-search = البحث بالكلمات
footer-env = إصدار { $env }
clear-confirm = انقر مرة أخرى لمسح كل شيء
clear = مسح كل البيانات

## First-run setup wizard.

wizard-title = مرحبًا
wizard-intro-device = إعداد لمرة واحدة. كل شيء يبقى على هذا الجهاز.
wizard-intro = إعداد لمرة واحدة. كل شيء يبقى على هذا الحاسوب.
wizard-store = 1. أين تُحفظ بياناتك
wizard-store-file = ملف بسيط (سريع، لا شيء للتنزيل)
wizard-store-postgres = قاعدة بيانات Postgres خاصة مع بحث متجهي (تنزيل نحو 30 ميغابايت في المرة الأولى)
wizard-no-postgres = لا يستطيع هذا المعالج تشغيل قاعدة البيانات المرفقة؛ مخزن الملفات يعمل بالطريقة نفسها.
wizard-model = 2. البحث الذكي
wizard-model-embed-bundled = البحث بالمعنى باستخدام نموذج ذكاء اصطناعي محلي (مدمج في التطبيق، بلا تنزيل؛ ولا يُرسَل شيء)
wizard-model-embed = البحث بالمعنى باستخدام نموذج ذكاء اصطناعي محلي (يُنزَّل مرة واحدة، ولا يُرسَل شيء)
wizard-go-title = 3. الإعداد
wizard-go = إعداد

## Update banner.

update-downloading-percent = جارٍ التنزيل… { $percent }٪
update-downloading = جارٍ التنزيل…
update-failed = فشل التحديث: { $error }
update-available = الإصدار { $version } متاح (لديك { $current }).
update-install = التثبيت وإعادة التشغيل

## Add documents.

add-reading = جارٍ قراءة { $path }…
add-file-done = { $count ->
    [zero] لم يُضف أي مقطع من { $path }.
    [one] أُضيف مقطع واحد من { $path }.
    [two] أُضيف مقطعان من { $path }.
    [few] أُضيفت { $count } مقاطع من { $path }.
    [many] أُضيف { $count } مقطعًا من { $path }.
   *[other] أُضيف { $count } مقطع من { $path }.
}
add-reading-folder = جارٍ قراءة كل المستندات في { $path }…
add-folder-failed = أُضيفت الملفات: { $files } ({ $chunks } مقطع)، وفشل { $failed }: { $error }.
add-folder-done = { $files ->
    [one] أُضيف ملف واحد
    [two] أُضيف ملفان
    [few] أُضيفت { $files } ملفات
    [many] أُضيف { $files } ملفًا
   *[other] أُضيف { $files } ملف
} ({ $chunks } مقطع).
add-text-done = { $count ->
    [zero] لم يُضف أي مقطع.
    [one] أُضيف مقطع واحد.
    [two] أُضيف مقطعان.
    [few] أُضيفت { $count } مقاطع.
    [many] أُضيف { $count } مقطعًا.
   *[other] أُضيف { $count } مقطع.
}
add-title = إضافة مستندات
add-hint-mobile = الصق نصًا، أو أدخل مسار ملف في هذا التطبيق.
add-hint = أسقط الملفات على هذه النافذة (PDF و‎.txt و‎.md)، أو أدخل مسار ملف، أو الصق نصًا.
add-path-placeholder = /path/to/file.pdf
add-path-label = مسار الملف أو المجلد
add-file = إضافة ملف
add-folder = إضافة مجلد
add-paste-placeholder = الصق النص هنا
add-paste-label = النص المراد إضافته
add-name = اسم للنص الملصق
add-text = إضافة النص

## Search.

search-title = بحث
search-placeholder = عمّ تبحث؟
search-go = بحث
search-none = لم يُعثر على شيء.
search-page = ، الصفحة { $page }
search-vector-score = { " " }· متجه { $score }
search-keyword-score = { " " }· كلمات { $score }

## Ask (AI).

ask-tokens = { $input } داخل / { $output } خارج من الرموز
ask-cached-tokens = { $count } من الذاكرة المؤقتة
ask-failed-tries = { $count ->
    [one] بعد محاولة فاشلة واحدة
    [two] بعد محاولتين فاشلتين
    [few] بعد { $count } محاولات فاشلة
   *[other] بعد { $count } محاولة فاشلة
}
ask-from-cache = من ذاكرة الإجابات المؤقتة
ask-title = اسأل
ask-placeholder = اسأل عن مستنداتك
ask-question = السؤال
ask-thinking = جارٍ التفكير…
ask-go = اسأل

## Dictation and speech.

dictate-nothing = لم يُسمع أي كلام.
dictate-mic-error = الميكروفون: { $error }
dictate-stop = ■ إيقاف
dictate-stop-title = إيقاف وتفريغ
dictate = إملاء
dictate-file-title = تفريغ ملف صوتي
dictate-file = ملف صوتي
speak-preparing = جارٍ التحضير…
speak = قراءة بصوت عالٍ

## Background agent, Python example and Claude Desktop.

agent-title = الخلفية
agent-toggle = إبقاء الخادم الخلفي يعمل بعد إغلاق النافذة، وتشغيله عند تسجيل الدخول
agent-on = يعمل: { $kind } في { $location }
agent-off = متوقف ({ $kind })
python-title = مثال بايثون
python-hint = يرسل هذا النص إلى عملية بايثون منفصلة:
python-go = إحصاءات النص في بايثون
claude-registered-backup = إعدادات Claude Desktop: { $status } (نسخة احتياطية: { $backup }). جارٍ تجهيز الأدوات مرة واحدة…
claude-registered = إعدادات Claude Desktop: { $status }. جارٍ تجهيز الأدوات مرة واحدة…
claude-ready = جاهز. أعد تشغيل Claude Desktop لاستخدامه.
claude-retry = تم الاتصال؛ فشل التشغيل الأول وستُعاد المحاولة عند تشغيل Claude.
claude-title = Claude Desktop
claude-hint = يضيف أدوات هذا التطبيق إلى Claude Desktop (تُحفظ نسخة من إعداداتك الحالية أولًا).
claude-go = ربط Claude Desktop

## The app shell: custom titlebar.

titlebar-settings-keys = الإعدادات ({ $keys })
titlebar-settings = الإعدادات
titlebar-minimize = تصغير
titlebar-restore = استعادة
titlebar-maximize = تكبير
titlebar-close = إغلاق

## Settings (ui-react/src/Settings.jsx).

settings-saved = تم الحفظ.
settings-not-saved = لم يُحفظ: { $error }
settings-title = الإعدادات
settings-sections = أقسام الإعدادات
settings-save = حفظ الإعدادات
settings-reset = استعادة الافتراضيات
save-first = احفظ الإعدادات أولًا.
remove = إزالة
refresh = تحديث

## Settings sections.

section-profiles = الملفات الشخصية
section-appearance = المظهر
section-language = اللغة
section-search = البحث
section-postgres = قاعدة البيانات
section-ingest = الاستيراد
section-llm = الذكاء الاصطناعي
section-mcp = خادم MCP
section-tts = النطق
section-stt = الإملاء
section-telemetry = الخصوصية
section-log = السجلات
section-resources = الأداء
section-net = الشبكة
section-backup = النسخ الاحتياطية
section-mail = البريد الإلكتروني
section-oauth = تسجيل الدخول
section-plugins = الإضافات
section-jobs = المهام
section-rclone = المزامنة السحابية
section-tailscale = شبكة Tailnet
section-http-api = واجهة HTTP
section-webview = عروض الويب
section-watch = المجلدات المراقَبة
section-service = الخدمة
section-tray = شريط النظام والاختصارات
section-updates = التحديثات
section-about = حول

## Settings fields: setting-<section>-<key>, and setting-<section>-<key>-<option> for the choices of a list.

setting-ui-language = اللغة
    .help = تتغير القوائم والرسائل فورًا. الترجمات الناقصة تظهر بالإنجليزية.
setting-ui-language-system = مثل النظام
setting-search-mode = وضع البحث
setting-search-top-k = النتائج
setting-search-candidates = المرشحون لكل قائمة (مختلط)
setting-search-vector-weight = وزن المتجه
setting-search-keyword-weight = وزن الكلمات
setting-search-rrf-k = معامل RRF k
setting-search-min-vector-score = أدنى درجة للمتجه (جيب التمام)
setting-search-min-keyword-score = أدنى درجة للكلمات
setting-search-min-score = أدنى درجة نهائية
setting-search-keyword-engine = محرك الكلمات (Postgres)
setting-search-bm25-tokenizer = نموذج تقطيع BM25
setting-search-fulltext-language = لغة النص الكامل
setting-postgres-upgrade = عندما يأتي إصدار جديد بنسخة أحدث من Postgres
setting-postgres-upgrade-ask = اسأل أولًا
setting-postgres-upgrade-auto = الترقية عند البدء
setting-postgres-upgrade-off = الإبقاء على الإصدار القديم
setting-postgres-upgrade-copy = الترقية بالنسخ (أبطأ، ضعف مساحة القرص)
setting-ingest-workers = عمّال الاستيراد (0 = واحد لكل نواة)
setting-ingest-chunk-chars = حجم المقطع (حروف)
setting-ingest-chunk-overlap = تداخل المقاطع (حروف)
setting-ingest-embed-batch = المقاطع في كل استدعاء تضمين
setting-ingest-max-file-mb = أكبر ملف (ميغابايت)
setting-ingest-extensions = أنواع الملفات
setting-ingest-pdf-isolate = قراءة ملفات PDF في عملية منفصلة ومحدودة
setting-ingest-pdf-timeout-secs = المهلة لملفات PDF (ثوانٍ)
setting-ingest-pdf-memory-mb = حد الذاكرة لملفات PDF (ميغابايت)
setting-llm-enabled = تشغيل إجابات الذكاء الاصطناعي
setting-llm-provider = المزوّد
setting-llm-model = النموذج (فارغ = افتراضي المزوّد)
setting-llm-base-url = رابط الخادم (فارغ = افتراضي المزوّد)
setting-llm-local-model-path = ملف النموذج المحلي (‎.gguf)
setting-llm-local-model-url = رابط تنزيل النموذج المحلي
setting-llm-local-context = السياق المحلي (رموز)
setting-llm-local-gpu-layers = الطبقات على معالج الرسوميات (0 = المعالج فقط)
setting-llm-local-threads = خيوط المعالج (0 = واحد لكل نواة)
setting-llm-local-keep-loaded = إبقاء النموذج المحلي في الذاكرة
setting-llm-temperature = درجة الحرارة (فارغ = افتراضي النموذج)
setting-llm-max-tokens = أطول إجابة (رموز)
setting-llm-timeout-secs = المهلة (ثوانٍ)
setting-llm-retries = المحاولات قبل المزوّد التالي
setting-llm-retry-base-ms = انتظار أول إعادة (مللي ثانية، يتضاعف مع كل محاولة)
setting-llm-retry-max-secs = أطول انتظار (ث؛ Retry-After أطول ينقل إلى المزوّد التالي)
setting-llm-daily-token-budget = ميزانية الرموز اليومية
setting-llm-monthly-token-budget = ميزانية الرموز الشهرية
setting-llm-budget-daily-usd = الميزانية اليومية (دولار)
setting-llm-budget-monthly-usd = الميزانية الشهرية (دولار)
setting-llm-budget-warn-percent = التنبيه عند (٪ من الميزانية؛ 0 = أبدًا)
setting-llm-budget-override-limits = المتابعة بعد تجاوز الميزانية (تجاوز)
setting-llm-prompt-cache = تخزين الموجّه مؤقتًا (cache_control لدى Anthropic؛ OpenAI وGemini يخزّنان تلقائيًا)
setting-llm-system-prompt = موجّه النظام (فارغ = المدمج)
setting-llm-http-hosts = المضيفون المسموح لهم عبر HTTP العادي
setting-llm-cache-enabled = إعادة استخدام الإجابات للأسئلة المكررة (تُمسح عند تغيّر المستندات)
setting-llm-cache-ttl-hours = الاحتفاظ بالإجابات المخزنة لمدة (ساعات)
setting-llm-cache-max-entries = أقصى عدد للإجابات المخزنة
setting-llm-cache-similarity = تطابق الأسئلة المتشابهة (جيب التمام، 1 = المطابق فقط)
setting-mcp-enabled = السماح لعملاء MCP (Claude Desktop وClaude Code و…) بالبحث في هذه المستندات
setting-mcp-allow-write = السماح أيضًا بالأدوات التي تغيّر البيانات (إضافة، إزالة)
setting-mcp-tools = هذه الأدوات فقط (فارغ = كل المسموح)
setting-mcp-audit-log = الاحتفاظ بسجل تدقيق لاستدعاءات الأدوات
setting-mcp-audit-arguments = تسجيل وسائط الاستدعاء أيضًا
setting-mcp-max-result-chars = أطول رد (حروف)
setting-mcp-approval-timeout-secs = انتظار الموافقة على أدوات «اسأل» (ثوانٍ)
setting-tts-enabled = قراءة الإجابات بصوت عالٍ (tiny-voca، على هذا الحاسوب)
setting-tts-voice = الصوت
setting-tts-voice-0 = F2 (أنثى)
setting-tts-voice-1 = M2 (ذكر)
setting-tts-speed = السرعة
setting-tts-pitch-shift = إزاحة طبقة الصوت
setting-tts-pitch-range = مدى طبقة الصوت
setting-tts-pause-ms = التوقف بين الجمل (مللي ثانية)
setting-tts-threads = خيوط المعالج (0 = تلقائي)
setting-stt-enabled = الإملاء والملفات الصوتية (Whisper، على هذا الحاسوب)
setting-stt-model = النموذج
setting-stt-model-tiny = Tiny (40 ميغابايت، الأسرع)
setting-stt-model-base = Base (75 ميغابايت)
setting-stt-model-small = Small (250 ميغابايت، الأدق)
setting-stt-language = رمز اللغة، مثل en أو ar (فارغ = اكتشاف)
setting-stt-translate = الترجمة إلى الإنجليزية
setting-stt-max-seconds = أطول تسجيل (ثوانٍ)
setting-stt-threads = خيوط المعالج (0 = تلقائي)
setting-resources-cpu-percent = حصة المعالج للفهرسة (٪)
setting-resources-background-priority = الفهرسة بأولوية الخلفية (يبقى الحاسوب سريع الاستجابة)
setting-resources-resume-interrupted = إكمال استيراد متقطع عند البدء التالي
setting-resources-memory-budget-mb = ميزانية الذاكرة للفهرسة (ميغابايت، 0 = بلا حد)
setting-resources-gpu-budget = تقييد معالج الرسوميات للنموذج المحلي
setting-resources-gpu-layers = طبقات معالج الرسوميات عند التقييد (0 = المعالج فقط)
setting-resources-onnx-provider = مسرّع ONNX (يعود إلى المعالج عند الحاجة)
setting-resources-onnx-provider-cpu = المعالج
setting-resources-onnx-provider-auto = الأفضل المتاح
setting-resources-onnx-provider-coreml = Core ML (macOS)
setting-resources-onnx-provider-directml = DirectML (Windows)
setting-resources-onnx-provider-cuda = CUDA (NVIDIA)
setting-backup-interval-hours = النسخ الاحتياطي كل (ساعات؛ 0 = عند الطلب فقط)
setting-backup-keep = النسخ المحتفظ بها (0 = الكل)
setting-backup-encrypt = تشفير النسخ الاحتياطية (احتفظ بمفتاح الاستعادة أدناه في مكان آمن)
setting-backup-folder = مجلد النسخ الاحتياطية (فارغ = بيانات التطبيق؛ المجلد المتزامن يحفظ نسخة خارج هذا الحاسوب)
setting-net-offline = وضع عدم الاتصال (لا يغادر شيء هذا الحاسوب)
setting-net-allow = هؤلاء المضيفون فقط (‎*.example.com للنطاقات الفرعية؛ فارغ = أي مضيف)
setting-net-deny = ليس هؤلاء المضيفين أبدًا
setting-net-proxy = الوكيل (فارغ = النظام، direct = بلا وكيل، أو رابط http://‎ أو socks5://‎)
setting-net-no-proxy = مضيفون يتجاوزون الوكيل
setting-net-ca-files = الوثوق بملفات شهادات CA هذه فقط (PEM)، مثل وكيل الشركة
setting-net-pins-off = تجاهل تثبيت الشهادات (خلف وكيل يفحص TLS)
setting-log-level = مستوى السجل
setting-log-file = كتابة ملف سجل (مجلد logs في بيانات التطبيق)
setting-log-max-mb = بدء ملف جديد عند (ميغابايت)
setting-log-keep = ملفات السجل القديمة المحتفظ بها
setting-telemetry-crash-reports = إرسال تقارير الأعطال (Sentry)
setting-telemetry-sentry-dsn = Sentry DSN
setting-telemetry-analytics = إرسال إحصاءات استخدام مجهولة
setting-telemetry-analytics-provider = خدمة التحليلات
setting-telemetry-analytics-provider-posthog = PostHog
setting-telemetry-analytics-provider-aptabase = Aptabase
setting-telemetry-analytics-key = مفتاح مشروع التحليلات
setting-telemetry-analytics-host = مضيف التحليلات (فارغ = افتراضي الخدمة)
setting-telemetry-traces = إرسال التتبعات (OpenTelemetry)
setting-telemetry-otlp-endpoint = نقطة OTLP، مثل http://localhost:4318
setting-mail-enabled = إرسال البريد (SMTP)
setting-mail-host = خادم البريد، مثل smtp.gmail.com
setting-mail-port = المنفذ (465 لـ SSL/TLS، و587 لـ STARTTLS)
setting-mail-security = التشفير
setting-mail-security-tls = SSL/TLS
setting-mail-security-starttls = STARTTLS
setting-mail-security-none = بلا تشفير (هذا الحاسوب فقط)
setting-mail-username = اسم الدخول (فارغ = بلا تسجيل دخول)
setting-mail-from = المرسل، مثل Me <me@example.com>
setting-mail-max-mb = أكبر رسالة (ميغابايت)
setting-mail-timeout-secs = المهلة (ثوانٍ)
setting-oauth-enabled = السماح بتسجيل الدخول (OAuth 2 / OpenID Connect في المتصفح)
setting-oauth-redirect = يعود المتصفح عبر
setting-oauth-redirect-loopback = 127.0.0.1 (يعمل في كل مكان)
setting-oauth-redirect-deep-link = الرابط العميق للتطبيق (يحتاج إضافة deep-link)
setting-oauth-timeout-secs = وقت إكمال تسجيل الدخول (ثوانٍ)
setting-oauth-refresh-before-secs = تحديث الرموز قبل انتهائها بهذه المدة (ثوانٍ)
setting-jobs-enabled = تشغيل مهام الخلفية والجداول
setting-jobs-workers = المهام في وقت واحد
setting-jobs-max-attempts = المحاولات قبل فشل المهمة
setting-jobs-backoff-base-secs = أول إعادة بعد نحو (ثوانٍ، تتضاعف)
setting-jobs-backoff-max-secs = أطول انتظار بين المحاولات (ثوانٍ)
setting-jobs-history-days = الاحتفاظ بسجل التشغيل لمدة (أيام)
setting-plugins-enabled = تشغيل الإضافات
setting-plugins-disabled = استبعاد هذه الإضافات (الأسماء)
setting-rclone-enabled = المزامنة مع السحابة عبر rclone
setting-rclone-remote = الوجهة البعيدة (اسمها في إعدادات rclone)
setting-rclone-path = المجلد على الوجهة (فارغ = جذرها)
setting-rclone-folder = ما تتم مزامنته
setting-rclone-folder-backups = مجلد النسخ الاحتياطية
setting-rclone-folder-data = مجلد البيانات (رفع فقط، بلا أسرار)
setting-rclone-direction = الاتجاه
setting-rclone-direction-push = رفع (نسخ، بلا حذف)
setting-rclone-direction-pull = تنزيل (نسخ، بلا حذف)
setting-rclone-direction-mirror = رفع مطابق (يحذف الملفات الزائدة هناك)
setting-rclone-direction-bisync = في الاتجاهين (bisync)
setting-rclone-interval-minutes = المزامنة كل (دقائق، 0 = عند الطلب فقط)
setting-rclone-bandwidth = حد عرض النطاق، مثل 2M أو 10M:1M (فارغ = بلا حد)
setting-tailscale-enabled = استخدام Tailscale
setting-tailscale-serve = مشاركة منفذ محلي مع شبكتي (tailscale serve)
setting-tailscale-port = المنفذ المحلي للمشاركة (على 127.0.0.1)
setting-tailscale-https-port = منفذ HTTPS على الشبكة
setting-webview-enabled = فتح صفحات الويب المسموحة داخل التطبيق
setting-webview-allow = المضيفون المسموحون (example.com، أو ‎*.example.com للنطاقات الفرعية أيضًا)
setting-webview-blocked = الصفحات الأخرى
setting-webview-blocked-refuse = رفضها
setting-webview-blocked-browser = فتحها في متصفح النظام
setting-webview-isolation = ملفات تعريف الارتباط وتسجيلات الدخول
setting-webview-isolation-profile = تُحفظ، بشكل منفصل لكل ملف شخصي
setting-webview-isolation-incognito = لا تُحفظ أبدًا (التصفح الخفي)
setting-webview-allow-http = إظهار صفحات HTTP العادية أيضًا على المضيفين المسموحين
setting-webview-max-windows = أقصى عدد لنوافذ الويب المفتوحة معًا
setting-webview-bridge = جسر الرسائل للصفحات الموثوقة (window.kitchenBridge)
setting-webview-bridge-hosts = المضيفون الذين يمكنهم استخدام الجسر (يجب أن يكونوا مسموحين أعلاه)
setting-http-api-enabled = تقديم واجهة HTTP المحلية
setting-http-api-host = الاستماع على (127.0.0.1 = هذا الحاسوب فقط)
setting-http-api-port = المنفذ (0 = أي منفذ متاح)
setting-http-api-cors-origins = أصول المتصفح المسموحة (CORS)
setting-http-api-max-body-kb = أكبر طلب (كيلوبايت)
setting-http-api-timeout-secs = مهلة الطلب (ثوانٍ)
setting-telemetry-otlp-headers = ترويسات OTLP (مفتاح=قيمة، مفصولة بفواصل)
setting-telemetry-environment = اسم البيئة
setting-watch-enabled = مراقبة المجلدات واستيراد الملفات الجديدة
setting-watch-drop-folder = استخدام مجلد الإسقاط
setting-watch-drop-folder-path = مجلد الإسقاط (فارغ = Documents/<app> Inbox)
setting-watch-paths = مجلدات أخرى للمراقبة
setting-watch-recursive = تضمين المجلدات الفرعية
setting-watch-debounce-ms = الانتظار بعد آخر تغيير (مللي ثانية)
setting-watch-ingest-existing = استيراد الملفات الموجودة مسبقًا
setting-watch-remove-deleted = نسيان الملفات المحذوفة
setting-watch-ignore = تجاهل الأسماء (أحرف بدل)
setting-appearance-mode = فاتح أو داكن
setting-appearance-mode-system = مثل النظام
setting-appearance-mode-light = فاتح
setting-appearance-mode-dark = داكن
setting-appearance-theme = السمة
setting-appearance-theme-kitchen = Kitchen
setting-appearance-theme-nord = Nord
setting-appearance-theme-solarized = Solarized
setting-appearance-theme-forest = الغابة
setting-appearance-theme-rose = وردي
setting-appearance-theme-contrast = تباين عالٍ
setting-appearance-ui-font = خط الواجهة (فارغ = النظام؛ خط جاهز أو أي خط مثبت أو مرفق)
setting-appearance-mono-font = خط الشيفرة والمحرر (فارغ = النظام)
setting-appearance-font-size = حجم النص (بكسل)
setting-tray-enabled = إظهار أيقونة في شريط النظام (شريط القوائم في macOS)
setting-tray-left-click-opens = النقر الأيسر يفتح النافذة (الأيمن: القائمة)
setting-tray-close-to-tray = إغلاق النافذة يبقي التطبيق في شريط النظام
setting-tray-show-in-menu-bar-only = macOS: شريط القوائم فقط، بلا أيقونة في Dock
setting-updates-channel = قناة التحديث
setting-updates-channel-stable = مستقرة
setting-updates-channel-beta = تجريبية (والإصدارات التمهيدية أيضًا)

## Profiles.

profiles-open-now = { " " }(مفتوح الآن)
profiles-switch = تبديل
profiles-remove = إزالة (تبقى ملفاته)
profiles-new = اسم الملف الشخصي الجديد
profiles-add = إضافة ملف شخصي

## Backups.

backup-running = جارٍ النسخ الاحتياطي…
backup-done = { $chunks ->
    [zero] لم يُنسخ أي مقطع.
    [one] نُسخ مقطع واحد احتياطيًا.
    [two] نُسخ مقطعان احتياطيًا.
    [few] نُسخت { $chunks } مقاطع احتياطيًا.
    [many] نُسخ { $chunks } مقطعًا احتياطيًا.
   *[other] نُسخ { $chunks } مقطع احتياطيًا.
}
backup-restoring = جارٍ الاستعادة…
backup-restored = تمت استعادة المقاطع: { $chunks }. نُسخ الفهرس المستبدَل احتياطيًا أولًا.
backup-now = نسخ احتياطي الآن
backup-none = لا توجد نسخ احتياطية بعد.
backup-item = { $created }: المقاطع { $chunks } من المصادر { $sources } ({ $kb } كيلوبايت)
backup-encrypted = ، مشفّرة
backup-restore = استعادة
backup-show-key = إظهار مفتاح الاستعادة
backup-key-hint = احتفظ به في مكان آمن: لا يمكن استعادة نسخة احتياطية مشفّرة على حاسوب آخر من دونه.
backup-key-label = مفتاح الاستعادة لنسخة احتياطية من حاسوب آخر (اختياري)

## Email.

mail-password-saved = تم حفظ كلمة المرور.
mail-sending = جارٍ الإرسال إلى { $to }…
mail-connecting = جارٍ الاتصال…
mail-sent = تم الإرسال. تحقق من { $to }.
mail-logged-in = تم الاتصال وتسجيل الدخول.
mail-password = كلمة مرور SMTP
mail-password-kept = كلمة المرور محفوظة (اكتب لاستبدالها)
mail-password-placeholder = كلمة المرور أو كلمة مرور التطبيق
mail-password-save = حفظ كلمة المرور
mail-password-remove = إزالة كلمة المرور

## AI keys.

ai-key-saved = تم حفظ المفتاح.
ai-key-removed = تمت إزالة المفتاح.
ai-key-provider = مفتاح المزوّد
ai-key = مفتاح API
ai-key-save = حفظ المفتاح
ai-keys-set = المفاتيح المحفوظة: { $list }
ai-keys-none = لا توجد مفاتيح محفوظة.

## AI usage.

usage-unpriced = { " " }(+{ $count } بلا سعر)
usage-tokens = { $input } داخل / { $output } خارج، { $cached } من الذاكرة المؤقتة
usage-today = اليوم ({ $day })
usage-month = هذا الشهر ({ $month })
usage-title = استخدام الذكاء الاصطناعي
usage-period = الفترة
usage-calls = الاستدعاءات
usage-tokens-head = الرموز
usage-cost = التكلفة
usage-of = { $used } من { $limit }
usage-over-override = متجاوز (التجاوز مفعّل)
usage-used-up = مستنفد
usage-no-budgets = لا توجد ميزانيات.

## Background jobs.

jobs-queued = أُضيفت { $kind } إلى الطابور.
jobs-off = تُحفظ المهام لكنها لا تعمل حتى تشغّلها أعلاه وتحفظ.
jobs-type = نوع المهمة
jobs-payload = البيانات (JSON)
jobs-run = التشغيل الآن
jobs-queue = الطابور:
jobs-queue-empty = الطابور فارغ.
jobs-attempt = { $state }، المحاولة { $attempt } من { $max }،
jobs-now = الآن
jobs-at = في { $time }
jobs-cancel = إلغاء
jobs-dead = المهام الفاشلة (نفدت المحاولات):
jobs-dead-item = { $count ->
    [one] بعد محاولة واحدة:
    [two] بعد محاولتين:
    [few] بعد { $count } محاولات:
   *[other] بعد { $count } محاولة:
}
jobs-retry = إعادة المحاولة
jobs-drop = تجاهل
jobs-schedules = الجداول:
jobs-schedules-none = لا توجد جداول.
jobs-every = كل { $secs } ث
jobs-paused = متوقفة مؤقتًا
jobs-next = التالية { $time }
jobs-resume = استئناف
jobs-pause = إيقاف مؤقت
jobs-history = آخر التشغيلات:
jobs-history-none = لا تشغيلات بعد.
jobs-run-item = المحاولة { $attempt }:

## Local HTTP API.

http-copied = تم نسخ { $what }.
http-listening = يستمع على { $url }.
http-error = لا يعمل: { $error }
http-off = لا يعمل. شغّله واحفظ الإعدادات.
http-url = الرابط
http-copy-url = نسخ الرابط
http-token = الرمز
http-copy-token = نسخ الرمز
http-token-hint = أرسل الرمز بصيغة Authorization: Bearer <token>. أبقِه سرًا.

## Tray, menu and shortcuts.

keys-listening = اضغط المفاتيح…
keys-placeholder = انقر ثم اضغط المفاتيح
tray-action = الإجراء
tray-separator = — فاصل —
tray-label-placeholder = التسمية (فارغة = الافتراضية)
tray-label = تسمية القائمة
tray-on = مفعّل
tray-up = نقل لأعلى
tray-down = نقل لأسفل
tray-add-item = إضافة عنصر
tray-menu = قائمة شريط النظام
tray-app-menu = قائمة التطبيق
tray-menu-title = العنوان
tray-shortcuts = اختصارات عامة
tray-shortcuts-hint = تعمل حتى عندما تكون تطبيقات أخرى في المقدمة.
tray-shortcuts-none = لا شيء بعد. تعمل حتى عندما تكون تطبيقات أخرى في المقدمة.
tray-add-shortcut = إضافة اختصار

## غلاف تطبيق الويب: القائمة الأصلية ونافذة التفضيلات.

wrapper-menu-preferences = التفضيلات…
wrapper-menu-edit = تحرير
wrapper-menu-view = عرض
wrapper-menu-page = الصفحة
wrapper-menu-reload = إعادة التحميل
wrapper-menu-back = رجوع
wrapper-menu-forward = تقدّم
wrapper-menu-zoom-in = تكبير
wrapper-menu-zoom-out = تصغير
wrapper-menu-actual-size = الحجم الفعلي
wrapper-menu-capture = تشغيل/إيقاف التقاط المفاتيح
wrapper-menu-mute = كتم / إلغاء الكتم
wrapper-menu-fill = ملء النموذج
wrapper-prefs-title = التفضيلات
wrapper-prefs-permissions = الأذونات
wrapper-prefs-site = الموقع
wrapper-prefs-certificate = الشهادة
wrapper-prefs-keys = المفاتيح
wrapper-prefs-media = الوسائط
wrapper-prefs-features = ميزات المتصفح
wrapper-prefs-headers = ترويسات الطلبات
wrapper-prefs-autofill = الملء التلقائي للنماذج
