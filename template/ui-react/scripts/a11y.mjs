// `npm run a11y`: accessibility check of the built UI, in a real browser.
//
// Builds the UI with the browser dev mock (src/dev-mock.js stands in for Rust), serves it,
// and runs axe-core (https://github.com/dequelabs/axe-core) on the main screen, the first-run
// wizard and every Settings section, in light and dark mode. Serious or critical issues fail
// the run, as does a Tab stop that cannot be seen. It also renders each screen with the pseudo-locale (accented text, 30% longer;
// src/pseudo.js) and fails when the page then grows wider than the window.
//
// Needs Playwright with Chromium, which is not a dependency of this app (it is large):
// `npm i -g playwright && npx playwright install chromium` (with NODE_PATH pointing at the
// global modules if Node does not find it). Without it the check is skipped (exit 0), unless
// A11Y_REQUIRED=1.
//
// Options: A11Y_LEVEL=moderate also fails on moderate issues; A11Y_REQUIRED=1.
// (No double curly braces in this folder: the template engine would eat them.)
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';

const require = createRequire(import.meta.url);
const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
// Inside node_modules: ignored by git, prettier and eslint, and emptied by the next run.
const outDir = path.join(root, 'node_modules', '.a11y-dist');
const levels = ['minor', 'moderate', 'serious', 'critical'];
const failAt = levels.indexOf(process.env.A11Y_LEVEL || 'serious');

let playwright;
try {
  playwright = require('playwright');
} catch {
  console.log('a11y: Playwright is not installed (npm i -g playwright && npx playwright install chromium); skipped');
  process.exit(process.env.A11Y_REQUIRED === '1' ? 1 : 0);
}

const { build, preview } = await import('vite');
process.env.VITE_DEV_MOCK = '1';
await build({ root, logLevel: 'warn', build: { outDir, emptyOutDir: true } });
const server = await preview({ root, logLevel: 'warn', build: { outDir }, preview: { port: 4173, strictPort: false, open: false } });
const base = server.resolvedUrls.local[0].replace(/\/$/, '');
const pages = [['main', '/']];
if (fs.existsSync(path.join(outDir, 'settings.html'))) pages.push(['settings window', '/settings.html']);

const browser = await playwright.chromium.launch();
const axeSource = fs.readFileSync(require.resolve('axe-core/axe.min.js'), 'utf8');
const problems = [];
let checked = 0;

async function axe(page, where) {
  await page.evaluate(axeSource);
  const r = await page.evaluate(() =>
    window.axe.run(document, { resultTypes: ['violations'], runOnly: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'best-practice'] }),
  );
  checked++;
  for (const v of r.violations) {
    const bad = levels.indexOf(v.impact) >= failAt;
    const nodes = v.nodes.map((n) => `      ${n.target.join(' ')}: ${n.failureSummary.split('\n').slice(1).join('; ').trim()}`);
    const line = `${bad ? 'FAIL' : 'note'} [${v.impact}] ${where}: ${v.id} (${v.help})\n${nodes.slice(0, 5).join('\n')}`;
    console.log(line);
    if (bad) problems.push(line);
  }
}

// Nothing may stick out past the window once the text is 30% longer.
async function layout(page, where) {
  checked++;
  const wide = await page.evaluate(() => {
    const out = [];
    const width = document.documentElement.clientWidth;
    for (const el of document.querySelectorAll('body *')) {
      const r = el.getBoundingClientRect();
      if (r.width && r.right > width + 1)
        out.push(el.tagName.toLowerCase() + (el.id ? `#${el.id}` : '') + (el.className ? `.${el.className}` : ''));
    }
    return out.slice(0, 5);
  });
  if (wide.length) {
    const line = `FAIL [layout] ${where}: wider than the window with longer text: ${wide.join(', ')}`;
    console.log(line);
    problems.push(line);
  }
}

// Keyboard: Tab moves through the controls in page order, each one visible when focused.
async function keyboard(page, where) {
  checked++;
  await page.evaluate(() => document.activeElement?.blur());
  const seen = [];
  for (let i = 0; i < 80; i++) {
    await page.keyboard.press('Tab');
    const f = await page.evaluate(() => {
      const el = document.activeElement;
      if (!el || el === document.body) return null;
      const r = el.getBoundingClientRect();
      const name =
        el.tagName.toLowerCase() + (el.id ? `#${el.id}` : '') + (el.textContent ? ` "${el.textContent.trim().slice(0, 20)}"` : '');
      return { name, visible: r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden', top: r.top + window.scrollY };
    });
    if (!f || seen.includes(f.name)) break;
    seen.push(f.name);
    if (!f.visible) {
      const line = `FAIL [keyboard] ${where}: focus lands on ${f.name}, which cannot be seen`;
      console.log(line);
      problems.push(line);
    }
  }
  if (seen.length === 0) problems.push(`FAIL [keyboard] ${where}: Tab reaches no control`);
}

// The page as it is, then each Settings section in turn.
async function screens(page, label, run) {
  await run(label);
  const count = (await page.$$('.settings-nav button[data-section]')).length;
  for (let i = 0; i < count; i++) {
    const button = (await page.$$('.settings-nav button[data-section]'))[i];
    const id = await button.getAttribute('data-section');
    await button.click();
    await page.waitForTimeout(150);
    await run(`${label} > Settings > ${id}`);
  }
}

for (const scheme of ['light', 'dark']) {
  for (const pseudo of [false, true]) {
    const context = await browser.newContext({ colorScheme: scheme, viewport: { width: 900, height: 800 } });
    const page = await context.newPage();
    page.on('pageerror', (e) => {
      const line = `FAIL [script] ${e.message}`;
      console.log(line);
      problems.push(line);
    });
    for (const [name, url] of [...pages, ['wizard', '/?wizard=1']]) {
      const q = `${url}${url.includes('?') ? '&' : '?'}mode=${scheme}${pseudo ? '&pseudo=1' : ''}`;
      await page.goto(base + q);
      await page.waitForSelector('main, .settings, h2', { timeout: 15000 });
      await page.waitForTimeout(400);
      const label = `${name} (${scheme}${pseudo ? ', pseudo-locale' : ''})`;
      const run = (where) => (pseudo ? layout(page, where) : axe(page, where));
      if (!pseudo) await keyboard(page, label);
      if (name === 'wizard') await run(label);
      else await screens(page, label, run);
    }
    await context.close();
  }
}

await browser.close();
await new Promise((done) => server.httpServer.close(done));
console.log(`a11y: ${checked} screens checked (axe-core and the pseudo-locale layout), ${problems.length} problem(s)`);
process.exit(problems.length ? 1 : 0);
