export const APP_NAME = '{{app_name}}';
export const DESCRIPTION = '{{description}}';

// The build-time app config: config/settings.json, then config/environments/<env>.json,
// then config/platforms/<platform>.json, merged by src-tauri/build.rs for this build and
// handed to the UI as `app_info().config`. It is fixed when the app is built; the user's
// own settings are the Settings screen (get_config / set_config).
export const CONFIG = {};

// Fill CONFIG from app_info (App.jsx does this once at start).
export function setConfig(value) {
  for (const k of Object.keys(CONFIG)) delete CONFIG[k];
  Object.assign(CONFIG, value || {});
  return CONFIG;
}

// The value at a dot path, e.g. configGet('search.defaultTopK', 10).
export function configGet(path, fallback, from = CONFIG) {
  let v = from;
  for (const key of path ? path.split('.') : []) {
    if (v == null || typeof v !== 'object' || !(key in v)) return fallback;
    v = v[key];
  }
  return v === undefined ? fallback : v;
}
