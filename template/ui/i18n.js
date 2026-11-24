// Translations (feature `i18n`) for the plain UI, no build step. The backend sends one
// locale's messages, already parsed from locales/<code>/app.ftl (`i18n_bundle`), and this
// file fills them in: `i18n.t('add-file-done', 'Added { $count } chunks.', { count: 3 })`.
// Static text in the HTML is marked `data-i18n="<id>"` (and `data-i18n-placeholder`,
// `data-i18n-title`, `data-i18n-aria-label` for attributes); the English in the HTML is the
// fallback. Plurals and numbers use the browser's Intl with the same rules as the backend.
/* global window, document, Intl */
(function () {
  const invoke = window.__TAURI__.core.invoke;
  const { listen } = window.__TAURI__.event;
  /** @type {null | {locale: string, dir: string, available: {code: string, name: string, dir: string}[], messages: Record<string, unknown>}} */
  let bundle = null;
  const listeners = new Set();
  const locale = () => (bundle ? bundle.locale : 'en');
  // Fluent wraps variables in bidi isolation marks; only right-to-left text needs them.
  const isolate = (s) => (bundle && bundle.dir === 'rtl' ? `⁨${s}⁩` : s);

  function number(n, options = {}) {
    const o = {};
    if (options.minimumFractionDigits) o.minimumFractionDigits = Number(options.minimumFractionDigits);
    if (options.maximumFractionDigits) o.maximumFractionDigits = Number(options.maximumFractionDigits);
    if (options.useGrouping === 'false') o.useGrouping = false;
    if (options.style === 'percent') o.style = 'percent';
    try { return new Intl.NumberFormat(locale(), o).format(n); } catch { return String(n); }
  }

  function date(v, options = {}) {
    const d = new Date(typeof v === 'number' ? v : String(v));
    if (Number.isNaN(d.getTime())) return String(v);
    const o = {};
    if (options.dateStyle && options.dateStyle !== 'none') o.dateStyle = options.dateStyle;
    if (options.timeStyle && options.timeStyle !== 'none') o.timeStyle = options.timeStyle;
    if (!o.dateStyle && !o.timeStyle) o.dateStyle = 'medium';
    try { return new Intl.DateTimeFormat(locale(), o).format(d); } catch { return d.toISOString(); }
  }

  // The value of an expression: what a selector compares, or a variable's raw value.
  function value(e, args) {
    if (e && typeof e === 'object' && 'var' in e) return args[e.var];
    return render(e, args, 0);
  }

  function render(v, args, depth) {
    if (depth > 8 || v == null) return '';
    if (typeof v === 'string') return v;
    if (v.parts) return v.parts.map((p) => render(p, args, depth + 1)).join('');
    if ('var' in v) {
      const x = args[v.var];
      if (x == null) return `{$${v.var}}`;
      return isolate(typeof x === 'number' ? number(x) : String(x));
    }
    if (v.fn === 'NUMBER') return isolate(number(Number(value(v.arg, args)), v.options));
    if (v.fn === 'DATETIME') return isolate(date(value(v.arg, args), v.options));
    if (v.ref) return render(bundle && bundle.messages[v.ref], args, depth + 1);
    if (v.select) {
      const x = value(v.select, args);
      let key = String(x);
      if (!(key in v.variants) && typeof x === 'number') {
        try { key = new Intl.PluralRules(locale()).select(x); } catch { key = v.default; }
      }
      return render(v.variants[key] ?? v.variants[v.default], args, depth + 1);
    }
    return '';
  }

  // English with `{ $name }` filled in: the text when a message is missing.
  const fill = (english, args) => english.replace(/\{ *\$(\w+) *\}/g, (_, k) => (args[k] == null ? '' : String(args[k])));

  /** A translated message; `english` (with `{ $name }` variables) when there is none. */
  function t(id, english = id, args = {}) {
    const m = bundle && bundle.messages[id];
    return m == null ? fill(english, args) : render(m, args, 0);
  }

  const ATTRS = ['placeholder', 'title', 'aria-label'];
  /** Translates every element marked `data-i18n` under `root`. */
  function apply(root = document) {
    for (const el of root.querySelectorAll('[data-i18n]')) {
      if (el.dataset.i18nEn == null) el.dataset.i18nEn = el.textContent.trim();
      el.textContent = t(el.dataset.i18n, el.dataset.i18nEn);
    }
    for (const a of ATTRS) {
      const key = `i18n${a.replace(/(^|-)(\w)/g, (_, __, c) => c.toUpperCase())}`;
      for (const el of root.querySelectorAll(`[data-i18n-${a}]`)) {
        const en = `${key}En`;
        if (el.dataset[en] == null) el.dataset[en] = el.getAttribute(a) || '';
        el.setAttribute(a, t(el.dataset[key], el.dataset[en]));
      }
    }
  }

  /** Loads a locale (`null` for the saved setting, `system` for the OS language) and redraws. */
  async function load(requested) {
    bundle = await invoke('i18n_bundle', { locale: requested || null });
    document.documentElement.lang = bundle.locale;
    document.documentElement.dir = bundle.dir;
    apply();
    for (const f of listeners) f(bundle.locale);
  }

  window.i18n = {
    t,
    apply,
    load,
    locale,
    available: () => (bundle ? bundle.available : []),
    onChange: (f) => listeners.add(f),
    ready: load(null).catch(() => {}),
  };
  // Another window (Settings) switched the language.
  listen('locale-changed', (e) => load(e.payload).catch(() => {}));
})();
