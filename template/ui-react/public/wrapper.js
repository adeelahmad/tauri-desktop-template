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

// Only the fields the app's developer lets the user change are shown (`editable`); the app
// refuses any other field, so this list is for display, not for safety.
async function load() {
  const s = await invoke('wrapper_get');
  current = s;
  const editable = new Set(s.editable || []);
  for (const el of form.querySelectorAll('[data-field]')) el.hidden = !editable.has(el.dataset.field);
  for (const section of form.querySelectorAll('section[data-auto]')) {
    section.hidden = [...section.querySelectorAll('[data-field]')].every((f) => f.hidden);
  }
  for (const el of form.elements) {
    if (!el.name) continue;
    const v = (s.values || {})[el.name];
    if (el.type === 'checkbox') el.checked = !!v;
    else if (v != null) el.value = v;
  }
  showWindowPrefs(s, editable);
  $('actions').hidden = editable.size === 0;
  $('site').textContent = s.site || '';
  $('certificate').hidden = !s.insecureTls;
  $('pin').textContent = s.pin || 'none';
  $('forget').hidden = !s.canForgetPin;
  $('forget-hint').hidden = !s.canForgetPin;
  const names = s.secrets || [];
  $('secrets').hidden = names.length === 0;
  $('secret-name').textContent = '';
  names.forEach((n) => $('secret-name').appendChild(new Option(n, n)));
  const problems = s.problems || [];
  $('problems').hidden = problems.length === 0;
  $('problems').textContent = problems.join('; ');
  showPermissions(s.permissions);
  applyBundle();
  return s;
}

// When each window preference takes effect, the menu bar's name on this OS, and the rule
// that the Dock icon and the menu bar icon are never both off (the app checks it too).
const APPLIES = { now: 'Applies at once.', nextLaunch: 'Applies on the next launch.' };
function showWindowPrefs(s, editable) {
  const mac = s.permissions && s.permissions.platform === 'macos';
  for (const el of document.querySelectorAll('[data-os]')) el.hidden = (el.dataset.os === 'macos') !== mac;
  for (const el of document.querySelectorAll('[data-applies]')) {
    const when = (s.applies || {})[el.dataset.applies];
    el.textContent = APPLIES[when] || '';
    el.dataset.i18n = when === 'nextLaunch' ? 'wrapper-prefs-applies-next-launch' : 'wrapper-prefs-applies-now';
  }
  keepReachable(editable);
}

function keepReachable(editable) {
  const dock = form.elements.dockIcon;
  const bar = form.elements.menuBarIcon;
  const both = editable.has('dockIcon');
  // Without the menu bar icon the Dock icon must stay; without the Dock icon, the menu bar icon.
  dock.disabled = both && !(editable.has('menuBarIcon') ? bar.checked : current.values.menuBarIcon);
  bar.disabled = both && !dock.checked;
  if (dock.disabled) dock.checked = true;
  $('reachable').hidden = !both || !(dock.disabled || bar.disabled);
}

form.elements.dockIcon.addEventListener('change', () => keepReachable(new Set(current.editable)));
form.elements.menuBarIcon.addEventListener('change', () => keepReachable(new Set(current.editable)));

// The changed values of the shown fields only.
function collect() {
  const changes = {};
  for (const el of form.elements) {
    if (!el.name || !current.editable.includes(el.name)) continue;
    const v = el.type === 'checkbox' ? el.checked : el.type === 'number' ? Number(el.value) : el.value.trim();
    if (v !== current.values[el.name]) changes[el.name] = v;
  }
  return changes;
}

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  $('status').textContent = 'Saving…';
  try {
    const r = await invoke('wrapper_set', { changes: collect() });
    await load();
    $('status').textContent = r.warning ? 'Saved, but: ' + r.warning : r.restart ? 'Saved. Some changes apply after a restart.' : 'Saved.';
    $('restart').hidden = !r.restart;
  } catch (err) {
    $('status').textContent = errorText(err);
  }
});

$('restart').onclick = () => invoke('wrapper_restart');
$('forget').onclick = async () => {
  await invoke('wrapper_forget_pin').catch((e) => ($('status').textContent = errorText(e)));
  await load();
};
$('save-secret').onclick = async () => {
  try {
    await invoke('wrapper_set_secret', { name: $('secret-name').value, value: $('secret-value').value });
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
let bundle = null;
async function translate() {
  try {
    bundle = await invoke('i18n_bundle', { locale: null });
  } catch {
    return;
  }
  applyBundle();
}

// Again after each load: the "applies" hints are set then.
function applyBundle() {
  if (!bundle) return;
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
