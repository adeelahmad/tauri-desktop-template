// The Settings window (feature app-shell): src-tauri/src/shell.rs opens it with Cmd/Ctrl+,,
// the titlebar's gear button or the app menu. Its capability allows events only.
/** @type {import('./bindings/commands').Invoke} (typed IPC, see app.js) */
const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);
const say = (text, error = false) => { $('status').textContent = text; $('status').classList.toggle('error', error); };
const FIELDS = { mode: 'mode', theme: 'theme', uiFont: 'ui-font', monoFont: 'mono-font', fontSize: 'font-size' };

function show(config) {
  const { appearance = {}, ...rest } = config;
  for (const [key, id] of Object.entries(FIELDS)) $(id).value = appearance[key] ?? '';
  $('config').value = JSON.stringify(rest, null, 2);
}

async function save(config) {
  try {
    show(await invoke('set_config', { config }));
    say('Saved.');
  } catch (e) { say(`Not saved: ${errorText(e)}`, true); }
}

$('save').onclick = () => {
  let rest;
  try { rest = JSON.parse($('config').value || '{}'); } catch (e) { return say(`Not valid JSON: ${e.message}`, true); }
  const appearance = {};
  for (const [key, id] of Object.entries(FIELDS)) appearance[key] = key === 'fontSize' ? Number($(id).value) : $(id).value;
  save({ ...rest, appearance });
};
$('reset').onclick = () => save({});
invoke('get_config').then(show).catch((e) => say(errorText(e), true));
