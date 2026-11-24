// Pseudo-locale: a layout test for translations. Every text on the page is shown accented and
// 30% longer ("Search" becomes "[Śéáŕçĥ ~~]"), so text that would be cut off, overflow or wrap
// badly in a longer language shows up before anyone translates the app, and text that stays
// plain English was never meant to be translated (or is hard-coded somewhere unexpected).
//
// Turn it on in a browser build with the dev mock (`npm run dev`, or the a11y check): add
// `?pseudo=1` to the URL, or run `localStorage.pseudoLocale = '1'` in the console and reload.
// main.jsx loads this file only then, so the app itself never contains it.
// (No double curly braces in this folder: the template engine would eat them.)

const ACCENTS = {
  a: 'á',
  b: 'ƀ',
  c: 'ç',
  d: 'ð',
  e: 'é',
  f: 'ƒ',
  g: 'ĝ',
  h: 'ĥ',
  i: 'í',
  j: 'ĵ',
  k: 'ķ',
  l: 'ĺ',
  m: 'ɱ',
  n: 'ñ',
  o: 'ó',
  p: 'þ',
  q: 'ǫ',
  r: 'ŕ',
  s: 'š',
  t: 'ţ',
  u: 'ú',
  v: 'ṽ',
  w: 'ŵ',
  x: 'ẋ',
  y: 'ý',
  z: 'ž',
  A: 'Á',
  B: 'Ɓ',
  C: 'Ç',
  D: 'Ð',
  E: 'É',
  F: 'Ƒ',
  G: 'Ĝ',
  H: 'Ĥ',
  I: 'Í',
  J: 'Ĵ',
  K: 'Ķ',
  L: 'Ĺ',
  M: 'Ṁ',
  N: 'Ñ',
  O: 'Ó',
  P: 'Þ',
  Q: 'Ǫ',
  R: 'Ŕ',
  S: 'Š',
  T: 'Ţ',
  U: 'Ú',
  V: 'Ṽ',
  W: 'Ŵ',
  X: 'Ẋ',
  Y: 'Ý',
  Z: 'Ž',
};

/** Accented, about 30% longer, in brackets: `pseudo('Save') === '[Šáṽé ~]'`. */
export function pseudo(text) {
  if (!text.trim()) return text;
  const accented = text.replace(/[A-Za-z]/g, (c) => ACCENTS[c]);
  const pad = '~'.repeat(Math.max(1, Math.round(text.trim().length * 0.3) - 2));
  const lead = text.match(/^\s*/)[0];
  const trail = text.match(/\s*$/)[0];
  return `${lead}[${accented.trim()} ${pad}]${trail}`;
}

const SKIP = new Set(['SCRIPT', 'STYLE', 'CODE', 'PRE', 'TEXTAREA', 'KBD']);
const ATTRS = ['placeholder', 'title', 'aria-label'];
// What this file last wrote into each node, so its own changes are not changed again.
const written = new WeakMap();

function textNode(n) {
  if (!n.parentElement || SKIP.has(n.parentElement.tagName) || written.get(n) === n.data) return;
  n.data = pseudo(n.data);
  written.set(n, n.data);
}

// Per element: the attribute values this file wrote.
const attrs = new WeakMap();

function element(el) {
  if (SKIP.has(el.tagName)) return;
  const mine = attrs.get(el) || {};
  for (const a of ATTRS) {
    const v = el.getAttribute(a);
    if (v && mine[a] !== v) {
      mine[a] = pseudo(v);
      el.setAttribute(a, mine[a]);
    }
  }
  attrs.set(el, mine);
}

function walk(root) {
  if (root.nodeType === Node.TEXT_NODE) return textNode(root);
  if (root.nodeType !== Node.ELEMENT_NODE) return;
  element(root);
  const it = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  for (let n = it.nextNode(); n; n = it.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) textNode(n);
    else element(n);
  }
}

/** Show the page in the pseudo-locale from now on (React renders included). */
export function startPseudoLocale(root = document.body) {
  document.documentElement.lang = 'en-XA';
  walk(root);
  new MutationObserver((changes) => {
    for (const c of changes) {
      if (c.type === 'characterData') textNode(c.target);
      else if (c.type === 'attributes') element(c.target);
      else c.addedNodes.forEach(walk);
    }
  }).observe(root, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });
}

/** Whether the URL (`?pseudo=1`) or localStorage (`pseudoLocale = '1'`) asks for it. */
export function wanted() {
  try {
    return new URLSearchParams(location.search).get('pseudo') === '1' || localStorage.getItem('pseudoLocale') === '1';
  } catch {
    return false;
  }
}
