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
action-wrapper-release = تحرير المفاتيح
action-wrapper-mute = كتم / إلغاء الكتم
action-wrapper-prefs = التفضيلات…
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
section-kv = مخزن المفاتيح والقيم
section-cache = الذاكرة المؤقتة

## Settings fields: setting-<section>-<key>, and setting-<section>-<key>-<option> for the choices of a list.

setting-ui-language = اللغة
    .help = تتغير القوائم والرسائل فورًا. الترجمات الناقصة تظهر بالإنجليزية.
setting-ui-language-system = مثل النظام
setting-ui-digits = الأرقام
setting-ui-digits-latin = لاتينية (0123456789)
setting-ui-digits-native = أرقام اللغة نفسها، مثل الهندية العربية (٠١٢٣٤٥٦٧٨٩)
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
setting-kv-ui-namespaces = نطاقات الأسماء المتاحة للنافذة وواجهة HTTP وأدوات MCP
setting-kv-encrypted-namespaces = نطاقات أسماء مشفّرة عند التخزين
setting-kv-key-source = مكان مفتاح التشفير
setting-kv-key-source-file = ملف مفتاح في مجلد البيانات
setting-kv-key-source-keychain = سلسلة مفاتيح النظام (تحتاج دعم تسجيل الدخول)
setting-kv-max-size-mb = أكبر حجم للمخزن (ميغابايت)
setting-kv-max-value-kb = أكبر قيمة (كيلوبايت)
setting-kv-sweep-minutes = حذف المفاتيح المنتهية كل (دقيقة)
setting-kv-backup = تضمين المخزن في النسخ الاحتياطية
setting-cache-max-mb = أكبر حجم للذاكرة المؤقتة (ميغابايت)
setting-cache-ttl-secs = الاحتفاظ بالعناصر لمدة (ثانية، 0 = حتى الإزاحة)
setting-cache-tti-secs = إسقاط العناصر غير المستخدمة بعد (ثانية، 0 = أبدًا)
setting-cache-persist = الاحتفاظ بالعناصر بعد إعادة التشغيل (في مخزن المفاتيح والقيم)

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

## The plain UI's settings (ui/index.html, ui/settings.html).

settings-json-hint = إعدادات البحث والاستيراد ومراقبة المجلدات بصيغة JSON (المفاتيح الناقصة تأخذ القيم الافتراضية؛ كل ما هو اختياري معطّل).
settings-json-hint-all = البحث والاستيراد وبقية الإعدادات بصيغة JSON (المفاتيح الناقصة تأخذ القيم الافتراضية؛ كل ما هو اختياري معطّل).
settings-everything-else = كل ما عدا ذلك
settings-bad-json = ليس JSON صالحًا: { $error }
appearance-ui-font = خط الواجهة
appearance-mono-font = خط الشيفرة والمحرر
appearance-font-hint = الفارغ يستخدم خط النظام. اختر إعدادًا جاهزًا أو اسم أي خط مثبّت أو مضمّن (انظر shell.css).
update-check = البحث عن تحديثات
update-not-configured = التحديثات غير مُعدّة في هذا الإصدار (لا يوجد مفتاح عام للمحدّث).
update-newest = لديك أحدث إصدار ({ $version }).
update-check-failed = تعذّر البحث عن تحديثات: { $error }
dictate-transcribing = جارٍ التفريغ…
dictate-listening = جارٍ الاستماع…

## Watch folders.

watch-folders = يراقب: { $folders }
watch-none = لا يراقب شيئًا.
watch-ingested = أُضيفت { $files } ملفات ({ $chunks } مقاطع)
watch-ingested-failed = أُضيفت { $files } ملفات ({ $chunks } مقاطع)، وفشل { $failed }
watch-removed = نُسي { $path }
watch-error = مشكلة: { $message }
watch-started = { $count ->
    [zero] بدأ على لا مجلد
    [one] بدأ على مجلد واحد
    [two] بدأ على مجلدين
    [few] بدأ على { $count } مجلدات
   *[other] بدأ على { $count } مجلدًا
}

## Sign-in (OAuth).

oauth-kind-google = Google
oauth-kind-github = GitHub
oauth-kind-microsoft = Microsoft
oauth-kind-oidc = OpenID Connect (أي مزوّد)
oauth-browser = أكمل تسجيل الدخول في المتصفح…
oauth-pending = أكمل تسجيل الدخول في المتصفح؛ سيُفتح التطبيق مجددًا عند الانتهاء.
oauth-revoked = تم تسجيل الخروج وإلغاء الرمز.
oauth-signed-out = تم تسجيل الخروج على هذا الحاسوب.
oauth-signed-in = تم تسجيل الدخول.
oauth-signed-in-as = تم تسجيل الدخول باسم { $who }.
oauth-not-signed-in = لم يتم تسجيل الدخول.
oauth-unsaved = احفظ الإعدادات لاستخدام هذا المزوّد.
oauth-keychain = سلسلة المفاتيح: { $error }
oauth-secret-saved = حُفظ سر العميل في سلسلة المفاتيح.
oauth-secret-removed = أُزيل سر العميل.
oauth-no-deep-link = إضافة الروابط العميقة غير مضمّنة؛ تُستخدم إعادة التوجيه إلى 127.0.0.1.
oauth-provider = المزوّد
oauth-id = الاسم (يُستخدم لتسجيل الدخول؛ مثل google)
oauth-client-id = معرّف العميل
oauth-scopes = النطاقات (فارغة = النطاقات المعتادة للمزوّد)
oauth-issuer = رابط المُصدِر، مثل https://id.example.com
oauth-tenant = المستأجر (common أو organizations أو consumers أو معرّف مستأجر)
oauth-secret = سر العميل
oauth-secret-kept = سر العميل محفوظ (اكتب لاستبداله)
oauth-secret-placeholder = سر العميل، إن كان المزوّد يحتاجه
oauth-secret-save = حفظ السر
oauth-secret-remove = إزالة السر
oauth-sign-out = تسجيل الخروج
oauth-sign-in-with = تسجيل الدخول عبر { $provider }
oauth-remove = إزالة المزوّد
oauth-add = إضافة مزوّد
oauth-hint = فعّل تسجيل الدخول واحفظ الإعدادات، ثم سجّل الدخول.

## Plugins.

plugins-none = لا توجد إضافات مضمّنة.
plugins-on = (مفعّلة)
plugins-off = (معطّلة)
plugins-methods = الطرق: { $list }.
json-invalid = (ليس JSON صالحًا بعد)

## Cloud sync (rclone).

rclone-config-saved = حُفظ الإعداد.
rclone-config-removed = أُزيل الإعداد.
rclone-synced = تمت المزامنة.
rclone-ready = rclone جاهز.
rclone-downloaded = rclone { $version }: تم تنزيله.
rclone-on-first-sync = rclone { $version }: يُنزَّل عند أول مزامنة.
rclone-unsupported = rclone { $version }: غير متاح لهذا الحاسوب.
rclone-remotes = الوجهات البعيدة: { $list }.
rclone-remotes-none = لا شيء
rclone-no-config = لا يوجد إعداد بعد.
rclone-last-ok = آخر مزامنة { $time } إلى { $target }: نجحت.
rclone-last-failed = آخر مزامنة { $time } إلى { $target }: فشلت.
rclone-config-label = إعداد الوجهة البعيدة: الصق ما يطبعه `rclone config show` (يُحفظ في ملف خاص)
rclone-config-save = حفظ الإعداد
rclone-config-remove = إزالة الإعداد
rclone-download = تنزيل rclone
rclone-sync = زامن الآن

## Tailnet (Tailscale).

tailscale-done = تم.
tailscale-missing = Tailscale غير مثبّت. نزّله من هنا، ثم أعد فتح الإعدادات:
tailscale-state = { $state }.
tailscale-state-as = { $state } باسم { $name }.
tailscale-state-in = { $state } في { $tailnet }.
tailscale-state-as-in = { $state } باسم { $name } في { $tailnet }.
tailscale-shared = المنفذ { $port } مُشارَك.
tailscale-shared-at = المنفذ { $port } مُشارَك على { $url }.
tailscale-nothing = لا شيء مُشارَك.
tailscale-apply = تطبيق المشاركة

## The backend as a service.

service-status = { $manager }: { $installed }، { $running }. السجل: { $log }.
service-installed = مثبّتة
service-not-installed = غير مثبّتة
service-running = تعمل
service-running-pid = تعمل (pid { $pid })
service-not-running = لا تعمل
service-done-installed = تم التثبيت.
service-done-removed = تمت الإزالة.
service-toggle = تشغيل الخادم الخلفي كخدمة: بلا نافذة، يبدأ عند تسجيل الدخول، ويُعاد تشغيله إذا تعطّل
service-boot = لتشغيله عند إقلاع الجهاز لجميع المستخدمين بدلًا من ذلك، نفّذ بصلاحيات المسؤول:

## Web views.

webview-message = رسالة من { $host }: { $data }
webview-opened = فُتحت في { $label }.
webview-url = صفحة الويب المراد فتحها
webview-open = فتح
webview-close = إغلاق

## Key-value store and cache.

kv-empty = لا توجد مفاتيح بعد.
kv-keys = { $count ->
    [zero] لا مفاتيح
    [one] مفتاح واحد
    [two] مفتاحان
    [few] { $count } مفاتيح
   *[other] { $count } مفتاحًا
}
kv-encrypted = ، مشفّر
kv-open-to-window = ، متاح للنافذة
cache-cleared = تم مسح الذاكرة المؤقتة.
cache-stats = { $entries } عنصر، { $used } من { $max } · { $hits } إصابة، { $misses } إخفاق (نسبة الإصابة { $rate }%)
cache-clear = مسح الذاكرة المؤقتة

## Downloads: voice, speech model, local AI model.

download-progress = جارٍ تنزيل { $progress }
voice-ready = الصوت جاهز.
voice-download = تنزيل الصوت (25 ميغابايت)
stt-ready = النموذج جاهز.
stt-download = تنزيل النموذج المختار (احفظ الإعدادات أولًا)
stt-download-whisper = تنزيل نموذج Whisper
model-progress = جارٍ التنزيل: { $progress }
model-ready = جاهز: { $path }
model-download = تنزيل النموذج المحلي

## Telemetry.

telemetry-off = لا يُرسل أي شيء ما لم تفعّله أعلاه.
telemetry-crash-reports = تقارير الأعطال
telemetry-analytics = إحصاءات الاستخدام
telemetry-traces = التتبعات
telemetry-nothing = لا شيء
telemetry-sending = يُرسل: { $what }. المعرّف المجهول { $id }…، أُرسل { $sent }
telemetry-pending = { $count ->
    [one] ، تقرير عطل واحد محفوظ على هذا الحاسوب
    [two] ، تقريرا عطل محفوظان على هذا الحاسوب
    [few] ، { $count } تقارير أعطال محفوظة على هذا الحاسوب
   *[other] ، { $count } تقرير عطل محفوظ على هذا الحاسوب
}
telemetry-last-error = . آخر مشكلة: { $error }
telemetry-promise = . تُرسل أسماء الاستدعاءات والأزمنة والأعداد فقط، ولا تُرسل مستنداتك أو أسئلتك أبدًا.

## MCP server.

mcp-registered = Claude Desktop: { $result }. أعد تشغيل Claude Desktop لتحميله.
mcp-add-claude = إضافة إلى Claude Desktop
mcp-claude-code = Claude Code:
mcp-save-first = احفظ الإعدادات والخادم مفعّل أولًا.
mcp-turn-on = فعّل هذا في الإعدادات واحفظ، ثم اربط عميلًا:
mcp-every-other = كل أداة أخرى
mcp-tool = الأداة { $tool }
mcp-default-allow = سماح (افتراضي)
mcp-as-others = مثل كل أداة أخرى
mcp-allow = سماح
mcp-ask = اسألني أولًا
mcp-deny = رفض
mcp-waiting = { $count ->
    [one] استدعاء MCP واحد ينتظرك:
    [two] استدعاءا MCP ينتظرانك:
    [few] { $count } استدعاءات MCP تنتظرك:
   *[other] { $count } استدعاء MCP ينتظرك:
}
mcp-none-waiting = لا توجد استدعاءات MCP تنتظر الموافقة.
mcp-approve = موافقة
mcp-deny-call = رفض

## Updates (release channels).

updates-going-back = جارٍ الرجوع إلى { $version }…
updates-rollback-failed = تعذّر الرجوع: { $error }
updates-save-hint = احفظ لتبديل القناة؛ سيستخدم فحص التحديث التالي القناة الجديدة.
updates-bucket = تصل الإصدارات المرحلية إلى هذا التثبيت عندما تُطرح لأكثر من { $bucket }%.
updates-previous = الإصدار السابق: { $version }
updates-kept = { " " }(محفوظ على هذا الحاسوب)
updates-download-again = { " " }(يُنزَّل مجددًا من إصداره)
updates-rollback = الرجوع إلى { $version }
updates-none = لا يوجد إصدار سابق للرجوع إليه بعد.

## About: licences.

licences-title = { $count ->
    [zero] التراخيص (لا حزم)
    [one] التراخيص (حزمة واحدة)
    [two] التراخيص (حزمتان)
    [few] التراخيص ({ $count } حزم)
   *[other] التراخيص ({ $count } حزمة)
}
licences-filter = التصفية حسب الاسم أو الترخيص
licences-package = الحزمة
licences-version = الإصدار
licences-licence = الترخيص
licences-show = عرض نصوص التراخيص
licences-hide = إخفاء نصوص التراخيص

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
wrapper-menu-open-in-browser = فتح في المتصفح…
wrapper-prefs-title = التفضيلات
wrapper-prefs-permissions = الأذونات
wrapper-prefs-site = الموقع
wrapper-prefs-certificate = الشهادة
wrapper-prefs-keys = المفاتيح
wrapper-prefs-media = الوسائط
wrapper-prefs-features = ميزات المتصفح
wrapper-prefs-headers = ترويسات الطلبات
wrapper-prefs-autofill = الملء التلقائي للنماذج
wrapper-prefs-window = النافذة وبدء التشغيل
wrapper-prefs-dock-icon = الإظهار في شريط Dock
wrapper-prefs-menu-bar-icon = الإظهار في شريط القوائم
wrapper-prefs-tray-icon = الإظهار في علبة النظام
wrapper-prefs-keep-one = أبقِ أيقونة Dock أو أيقونة شريط القوائم، وإلا تعذّر الوصول إلى التطبيق.
wrapper-prefs-run-at-startup = الفتح عند تسجيل الدخول
wrapper-prefs-remember-window = تذكّر حجم النافذة ومكانها
wrapper-prefs-applies-now = يُطبَّق فورًا.
wrapper-prefs-applies-next-launch = يُطبَّق عند التشغيل التالي.
