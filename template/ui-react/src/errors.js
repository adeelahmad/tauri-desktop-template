// Text for a rejected invoke(). Commands reject with { code, message, details? } (a
// CommandError, see src-tauri/src/error.rs and the code table in docs/GUIDE.md); anything
// else (a plain string, a browser Error) is shown as it is.
//
//   try { await api.search({ query }); } catch (e) { setMsg(errorText(e)); }
//   if (errorCode(e) === 'not_ready') retryLater();

// A next step for codes where the message alone does not say what to do. Keyed by the
// stable codes, so a message can change without breaking these.
const HINTS = {
  denied: 'Check that the app may use this file or folder.',
  not_built: 'This build of the app does not include it.',
  mail: 'Check the email settings.',
  oauth: 'Try signing in again.',
  tailscale: 'Check that Tailscale is running.',
};

/** @param {unknown} e */
const isCommandError = (e) => !!e && typeof e === 'object' && 'message' in e;

/**
 * The message for people, with a next step for some codes.
 * @param {unknown} e what the promise rejected with
 * @returns {string}
 */
export function errorText(e) {
  const message = isCommandError(e) ? String(/** @type {any} */ (e).message) : String(e);
  const hint = HINTS[errorCode(e)];
  if (!hint || message.includes(hint)) return message;
  return `${message}${/[.!?]$/.test(message) ? '' : '.'} ${hint}`;
}

/**
 * The error's stable code ('invalid', 'not_found', 'not_ready', ...), or 'failed' when it
 * is not a CommandError.
 * @param {unknown} e
 * @returns {string}
 */
export function errorCode(e) {
  return (e && typeof e === 'object' && 'code' in e && typeof e.code === 'string' && e.code) || 'failed';
}
