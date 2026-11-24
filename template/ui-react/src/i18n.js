// Translations for the UI: t(id, english, args) everywhere a text is shown.
//
//   import { t } from './i18n.js';
//   <h2>{t('search-title', 'Search')}</h2>
//   setMsg(t('add-file-done', 'Added { $count } chunks from { $path }.', { count, path }));
//
// The English after the id is what shows when the app is built without translations
// (feature i18n off), and it keeps the code readable. With the feature on, the message comes
// from locales/<code>/app.ftl (Fluent, the same files the Rust side uses; src/locales.js
// embeds them at build time), in the user's language, with plurals and number formats;
// a message missing there falls back to locales/en/app.ftl, then to the English here.
// `kitchen i18n check` lists ids used here that locales/en/app.ftl does not have.
//
// The roots (App.jsx, SettingsWindow.jsx) call useLocale(), so a language change re-renders
// everything at once and sets <html lang dir>.
// (No double curly braces in this folder: the template engine would eat them.)
import { {% if i18n %}useEffect, {% endif %}useSyncExternalStore } from 'react';
{% if i18n %}import { listen } from '@tauri-apps/api/event';
import { api } from './ipc.js';
import * as fluent from './locales.js';
{% endif %}
let current = { locale: 'en', dir: 'ltr', bundles: [] };
const listeners = new Set();

// The English fallback with `{ $name }` filled in.
function fill(text, args) {
  return text.replace(/\{\s*\$([\w-]+)\s*\}/g, (m, k) => (args && k in args ? String(args[k]) : m));
}

/**
 * Message `id` in the user's language, else `english` (with `{ $name }` filled from `args`).
 * `id.attr` reads an attribute, e.g. t('setting-ui-language.help', '...').
 * @param {string} id
 * @param {string} english
 * @param {Record<string, string | number | Date>} [args]
 * @returns {string}
 */
export function t(id, english, args) {
  const [msgId, attr] = id.split('.');
  for (const b of current.bundles) {
    const m = b.getMessage(msgId);
    const pattern = attr ? m?.attributes?.[attr] : m?.value;
    if (pattern) return b.formatPattern(pattern, args);
  }
  return fill(english, args);
}

/** The locale in use, e.g. 'de', and its direction ('ltr' or 'rtl'). */
export const locale = () => current.locale;
export const direction = () => current.dir;

/** The locales to choose from, each named in its own language. Empty without feature i18n. */
export function available() {
{% if i18n %}  return fluent.available();
{% else %}  return [];
{% endif %}}

/** Whether this build has translations (feature i18n). */
export const translated = {% if i18n %}true{% else %}false{% endif %};

/**
 * Switch the UI to `requested` ('system', a locale code, or 'en-XA' for the pseudo-locale)
 * and return the locale used. 'system' asks the Rust side for the OS language.
 * @param {string} requested
 */
export async function setLocale(requested) {
{% if i18n %}  let wanted = requested || 'system';
  if (wanted === 'system') {
    wanted = await api
      .i18n_bundle({ locale: 'system' })
      .then((b) => b.locale)
      .catch(() => navigator.language || 'en');
  }
  const code = fluent.negotiate(wanted);
  current = { locale: code, dir: fluent.direction(code), bundles: fluent.bundles(code) };
{% else %}  void requested;
{% endif %}  const root = document.documentElement;
  root.lang = current.locale;
  root.dir = current.dir;
  listeners.forEach((f) => f());
  return current.locale;
}

const subscribe = (f) => {
  listeners.add(f);
  return () => listeners.delete(f);
};

/**
 * For the root components: re-render on a language change (here or in another window,
 * through the `locale-changed` event) and return the locale.
 */
export function useLocale() {
{% if i18n %}  useEffect(() => {
    const off = listen('locale-changed', (e) => setLocale(e.payload));
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
{% endif %}  return useSyncExternalStore(subscribe, () => current.locale);
}
