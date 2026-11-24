// Text for a rejected invoke(): commands reject with { code, message } (see
// src-tauri/src/error.rs) or, in older commands, a plain string.
export function errorText(e) {
  if (e && typeof e === 'object' && 'message' in e) return String(e.message);
  return String(e);
}

// The error's code ('invalid', 'not_found', ...), or 'failed' for a plain string.
export function errorCode(e) {
  return (e && typeof e === 'object' && e.code) || 'failed';
}
