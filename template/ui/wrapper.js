// Preferences for the web app wrapper (feature web-wrapper). The app's own page: it talks to
// the wrapper_* commands; the wrapped site never can.
'use strict';

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const form = $('form');
let current = null;

function errorText(e) {
  return (e && e.message) || String(e);
}

function get(obj, path) {
  return path.split('.').reduce((o, k) => (o == null ? o : o[k]), obj);
}

function set(obj, path, value) {
  const keys = path.split('.');
  const last = keys.pop();
  keys.reduce((o, k) => (o[k] = o[k] || {}), obj)[last] = value;
}

function row(container, values, placeholders) {
  const div = document.createElement('div');
  div.className = 'row';
  values.forEach((v, i) => {
    const input = document.createElement('input');
    input.value = v || '';
    input.placeholder = placeholders[i];
    input.setAttribute('aria-label', placeholders[i]);
    input.spellcheck = false;
    div.appendChild(input);
  });
  const remove = document.createElement('button');
  remove.type = 'button';
  remove.textContent = 'Remove';
  remove.onclick = () => div.remove();
  div.appendChild(remove);
  container.appendChild(div);
}

function rows(container) {
  return [...container.querySelectorAll('.row')]
    .map((r) => [...r.querySelectorAll('input')].map((i) => i.value.trim()))
    .filter((v) => v.some((x) => x));
}

function showPermissions(p) {
  const section = $('permissions');
  section.hidden = !(p.items && p.items.length);
  const list = $('perm-list');
  list.textContent = '';
  (p.items || []).forEach((item) => {
    const li = document.createElement('li');
    li.textContent = (item.granted ? '✓ ' : '✗ ') + item.label + ' ';
    if (!item.granted) {
      const b = document.createElement('button');
      b.type = 'button';
      b.textContent = 'Open System Settings';
      b.onclick = () => invoke('wrapper_open_privacy', { pane: item.id }).catch(() => {});
      li.appendChild(b);
    }
    list.appendChild(li);
  });
}

async function load() {
  const s = await invoke('wrapper_get');
  current = s.config;
  for (const el of form.elements) {
    if (!el.name) continue;
    const v = get(current, el.name);
    if (el.type === 'checkbox') el.checked = !!v;
    else if (v != null) el.value = v;
  }
  $('pin').textContent = s.pin || 'none';
  $('headers').textContent = '';
  current.headers.forEach((h) => row($('headers'), [h.pattern, h.name, h.value], ['pattern', 'name', 'value']));
  $('autofill').textContent = '';
  current.autofill.forEach((a) =>
    Object.entries(a.fields).forEach(([sel, v]) =>
      row($('autofill'), [a.pattern, sel, v], ['pattern', 'CSS selector', 'value']),
    ),
  );
  const problems = s.problems || [];
  $('problems').hidden = problems.length === 0;
  $('problems').textContent = problems.join('; ');
  showPermissions(s.permissions);
  return s;
}

function collect() {
  const c = JSON.parse(JSON.stringify(current));
  for (const el of form.elements) {
    if (!el.name) continue;
    const v = el.type === 'checkbox' ? el.checked : el.type === 'number' ? Number(el.value) : el.value;
    set(c, el.name, v);
  }
  c.headers = rows($('headers')).map(([pattern, name, value]) => ({ pattern, name, value }));
  const fills = {};
  rows($('autofill')).forEach(([pattern, sel, value]) => {
    (fills[pattern] = fills[pattern] || {})[sel] = value;
  });
  c.autofill = Object.entries(fills).map(([pattern, fields]) => ({ pattern, fields }));
  return c;
}

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  $('status').textContent = 'Saving…';
  try {
    const r = await invoke('wrapper_set', { config: collect() });
    await load();
    $('status').textContent = r.restart ? 'Saved. Some changes apply after a restart.' : 'Saved.';
    $('restart').hidden = !r.restart;
  } catch (err) {
    $('status').textContent = errorText(err);
  }
});

$('restart').onclick = () => invoke('wrapper_restart');
$('add-header').onclick = () => row($('headers'), ['', '', ''], ['pattern', 'name', 'value']);
$('add-fill').onclick = () => row($('autofill'), ['', '', ''], ['pattern', 'CSS selector', 'value']);
$('forget').onclick = async () => {
  await invoke('wrapper_forget_pin').catch((e) => ($('status').textContent = errorText(e)));
  await load();
};
$('save-secret').onclick = async () => {
  try {
    await invoke('wrapper_set_secret', { name: $('secret-name').value.trim(), value: $('secret-value').value });
    $('secret-value').value = '';
    $('status').textContent = 'Saved in the keychain.';
  } catch (err) {
    $('status').textContent = errorText(err);
  }
};

// Record a shortcut: the next key pressed with its modifiers.
$('record').onclick = () => {
  const input = form.elements.releaseShortcut;
  input.value = 'Press the keys…';
  const onKey = (e) => {
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
    e.preventDefault();
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Cmd');
    parts.push(e.code.replace(/^Key|^Digit/, ''));
    input.value = parts.join('+');
    window.removeEventListener('keydown', onKey, true);
  };
  window.addEventListener('keydown', onKey, true);
};

// The first-run permission screen notices when access is granted.
setInterval(async () => {
  if ($('permissions').hidden) return;
  try {
    const s = await invoke('wrapper_get');
    showPermissions(s.permissions);
  } catch {
    // the next round tries again
  }
}, 2000);

// Translations, when the app has them (feature i18n: the `i18n_bundle` command): the plain
// messages of the headings; the English in the page stays otherwise.
async function translate() {
  let bundle;
  try {
    bundle = await invoke('i18n_bundle', { locale: null });
  } catch {
    return;
  }
  const text = (m) =>
    typeof m === 'string' ? m : m && Array.isArray(m.parts) && m.parts.every((p) => typeof p === 'string') ? m.parts.join('') : null;
  document.documentElement.lang = bundle.locale;
  document.documentElement.dir = bundle.dir;
  for (const el of document.querySelectorAll('[data-i18n]')) {
    const t = text(bundle.messages[el.dataset.i18n]);
    if (t) el.textContent = t;
  }
}

translate();
load().catch((e) => ($('status').textContent = errorText(e)));
