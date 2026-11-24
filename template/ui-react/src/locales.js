// The translations (feature i18n): every locales/<code>/app.ftl, embedded at build time, as
// @fluent/bundle bundles. src/i18n.js is what the components use; this file only loads.
//
// A new locale needs no code here: `kitchen generate locale fr` adds locales/fr/app.ftl and
// the next build picks it up. 'en-XA' is the pseudo-locale (accented, 30% longer, from the
// English messages; `?pseudo=1` with the dev mock), for finding text that would not fit.
// (No double curly braces in this folder: the template engine would eat them.)
import { FluentBundle, FluentResource } from '@fluent/bundle';
import { pseudo } from './pseudo.js';

// { '../../locales/de/app.ftl': '...' }: the same files desktop-core embeds.
const files = import.meta.glob('../../locales/*/app.ftl', { query: '?raw', import: 'default', eager: true });

export const SOURCE = 'en';
export const PSEUDO = 'en-XA';

/** @type {Record<string, string>} locale code -> Fluent source */
const SOURCES = Object.fromEntries(Object.entries(files).map(([file, ftl]) => [file.split('/').at(-2), ftl]));

// Written right to left: <html dir="rtl"> and mirrored layout (the same list as desktop_core::i18n).
const RTL = ['ar', 'arc', 'ckb', 'dv', 'fa', 'he', 'ks', 'ps', 'sd', 'ug', 'ur', 'yi'];

/** 'rtl' or 'ltr' for a locale code. */
export function direction(code) {
  const lower = code.toLowerCase();
  return RTL.includes(lower.split(/[-_]/)[0]) || lower.includes('-arab') ? 'rtl' : 'ltr';
}

/** The locale codes this app has, the source locale first. */
export function codes() {
  return [
    SOURCE,
    ...Object.keys(SOURCES)
      .filter((c) => c !== SOURCE)
      .sort(),
  ];
}

/** The closest locale to `requested` (as desktop_core::i18n::negotiate does): the same, the same language, or English. */
export function negotiate(requested) {
  const want = String(requested || '')
    .split(/[.@]/)[0]
    .replace(/_/g, '-')
    .toLowerCase();
  if (want === PSEUDO.toLowerCase()) return PSEUDO;
  const all = codes();
  const lang = (c) => c.toLowerCase().split('-')[0];
  return all.find((c) => c.toLowerCase() === want) || all.find((c) => want && lang(c) === lang(want)) || SOURCE;
}

// Digits (the same rule as desktop_core::i18n::number_locale): Latin by default in every
// language, so every WebView writes 1,234.5 whatever its ICU data says about 'ar'; with
// ui.digits = 'native', Arabic-Indic for Arabic and Persian digits for Persian.
const NATIVE = { ar: 'arab', fa: 'arabext' };
const LATN_REGIONS = ['ar-ma', 'ar-dz', 'ar-tn', 'ar-ly'];

/**
 * The tag for Intl (and Fluent) that writes numbers as the backend does, e.g. 'ar-u-nu-latn'.
 * @param {string} code
 * @param {string} [digits] 'latin' (default) or 'native'
 */
export function numberLocale(code, digits) {
  const lower = code.toLowerCase();
  const latn = lower.includes('-latn') || LATN_REGIONS.some((p) => lower.startsWith(p));
  const nu = (digits === 'native' && !latn && NATIVE[lower.split(/[-_]/)[0]]) || 'latn';
  return `${code}-u-nu-${nu}`;
}

const cache = new Map();

function bundle(code, digits) {
  const key = `${code}|${digits}`;
  if (!cache.has(key)) {
    const pseudoLocale = code === PSEUDO;
    const b = new FluentBundle(numberLocale(code, digits), {
      // Isolation marks keep a file name or number in its own direction; only RTL needs them.
      useIsolating: direction(code) === 'rtl',
      transform: pseudoLocale ? pseudo : undefined,
    });
    const errors = b.addResource(new FluentResource(SOURCES[pseudoLocale ? SOURCE : code] || ''));
    errors.forEach((e) => console.warn(`locales/${code}/app.ftl:`, e));
    cache.set(key, b);
  }
  return cache.get(key);
}

/**
 * The bundles to format `code` with: its own, then English for what it does not have.
 * @param {string} code
 * @param {string} [digits] 'latin' (default) or 'native'
 */
export function bundles(code, digits) {
  return code === SOURCE ? [bundle(SOURCE, digits)] : [bundle(code, digits), bundle(SOURCE, digits)];
}

/** The locales to choose from: { code, name (in its own language), dir }. */
export function available() {
  return codes().map((code) => {
    const b = bundle(code, 'latin');
    const m = b.getMessage('language-name');
    return { code, name: m?.value ? b.formatPattern(m.value) : code, dir: direction(code) };
  });
}
