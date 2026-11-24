// Text for a rejected invoke() (loaded before app.js and settings.js). Commands reject with
// { code, message, details? } (a CommandError: src-tauri/src/error.rs, codes in
// docs/GUIDE.md, "Command errors"); anything else is shown as it is.
//
//   try { await invoke('search', { query }); } catch (e) { say('status', errorText(e), true); }

// A next step for codes where the message alone does not say what to do.
const ERROR_HINTS = {
  denied: 'Check that the app may use this file or folder.',
  not_built: 'This build of the app does not include it.',
  mail: 'Check the email settings.',
  oauth: 'Try signing in again.',
  tailscale: 'Check that Tailscale is running.',
};

/** The error's stable code ('invalid', 'not_found', 'not_ready', ...), or 'failed'. */
function errorCode(e) {
  return (e && typeof e === 'object' && typeof e.code === 'string' && e.code) || 'failed';
}

/** The message for people (e.message, else String(e)), with a next step for some codes. */
function errorText(e) {
  const message = e && typeof e === 'object' && 'message' in e ? String(e.message) : String(e);
  const hint = ERROR_HINTS[errorCode(e)];
  if (!hint || message.includes(hint)) return message;
  return `${message}${/[.!?]$/.test(message) ? '' : '.'} ${hint}`;
}
