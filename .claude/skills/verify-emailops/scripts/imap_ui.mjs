// Drive a real IMAP account through the desktop UI, against the local test
// Dovecot of scripts/imap_test.sh, on a fresh data dir:
//
//   first-run wizard (plain) → IMAP / SMTP form → Test connection → Add account
//   → the seeded mail shows in the inbox → star it and archive it from the UI
//   → the server shows \Flagged and the message in Archive.
//
//   node imap_ui.mjs <out_dir>
//   env IMAP_UI_USER   the fresh login (already seeded by scripts/imap_ui.sh)
//   env IMAP_UI_SUBJECTS  subjects seeded, separated by "|"
//   env EMAILOPS_IMAP_LIVE_*  where the server is (scripts/imap_test.sh env)
//
// Writes <out_dir>/imap-ui.json: [{step, status, detail, shot}] and exits 1 on a failure.
import { remote } from 'webdriverio';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const [outDir] = process.argv.slice(2);
if (!outDir) { console.error('usage: imap_ui.mjs <out_dir>'); process.exit(2); }
fs.mkdirSync(outDir, { recursive: true });
const USER = process.env.IMAP_UI_USER;
const SUBJECTS = (process.env.IMAP_UI_SUBJECTS || '').split('|').filter(Boolean);
const HOST = process.env.EMAILOPS_IMAP_LIVE_HOST;
const PORT = process.env.EMAILOPS_IMAP_LIVE_PORT;
const SMTP_PORT = process.env.EMAILOPS_IMAP_LIVE_SMTP_PORT;
const PASSWORD = process.env.EMAILOPS_IMAP_LIVE_PASSWORD;
const REPO = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../../../..');

const b = await remote({
  hostname: '127.0.0.1', port: Number(process.env.TAURI_WEBDRIVER_PORT || 4445), path: '/', capabilities: {}, logLevel: 'error',
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const js = (fn, ...a) => b.execute(fn, ...a);
const screen = () => js(() => document.body.innerText.replace(/\s+/g, ' '));

const results = [];
let shots = 0;
async function shot(name) {
  const f = `imap-ui-${String(++shots).padStart(2, '0')}-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`;
  await b.saveScreenshot(path.join(outDir, f)).catch(() => {});
  return f;
}
async function step(name, fn) {
  let status = 'ok', detail = '';
  try { detail = (await fn()) || ''; } catch (e) { status = 'fail'; detail = String(e.message || e); }
  const f = await shot(name);
  results.push({ step: name, status, detail, shot: f });
  console.log(`  ${status === 'ok' ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  return status === 'ok';
}
async function until(what, probe, ms = 30000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v) return v;
    await sleep(500);
  }
  throw new Error(`timed out waiting for ${what}`);
}
// Click the first visible button whose text or aria-label starts with `text`
// (case-insensitive: some buttons are rendered in title case).
async function press(text, { within = null } = {}) {
  const ok = await until(`«${text}»`, () => js((t, w) => {
    const root = w ? document.querySelector(w) : document;
    const lower = t.toLowerCase();
    const el = [...(root || document).querySelectorAll('button,[role=button]')].find((e) => e.offsetParent !== null && !e.disabled
      && ((e.innerText || '').replace(/\s+/g, ' ').trim().toLowerCase().startsWith(lower)
        || (e.getAttribute('aria-label') || '').toLowerCase() === lower));
    if (!el) return false;
    el.click();
    return true;
  }, text, within), 10000);
  await sleep(700);
  return ok;
}
// Click an icon button of the reading pane (no text of its own) whose title or
// aria-label starts with `label`: its actions read "Star (s)", "Archive (e)".
// Scoped to the pane — found by walking up from its Reply All button — since
// the sidebar has a "Star on GitHub" icon and "Starred" / "Archive" views.
async function pressIcon(label) {
  await until(`the «${label}» icon in the reading pane`, () => js((l) => {
    const lower = l.toLowerCase();
    const isTarget = (e) => e.offsetParent !== null && !e.disabled && !(e.innerText || '').trim()
      && (e.title || e.getAttribute('aria-label') || '').toLowerCase().startsWith(lower);
    const reply = [...document.querySelectorAll('button')]
      .find((e) => e.offsetParent !== null && (e.innerText || '').trim() === 'Reply All');
    for (let pane = reply?.parentElement; pane; pane = pane.parentElement) {
      const el = [...pane.querySelectorAll('button')].find(isTarget);
      if (el) { el.click(); return true; }
    }
    return false;
  }, label), 10000);
  await sleep(700);
}

// Set a React-controlled input the way typing does: the native value setter,
// then an input event. clearValue + setValue leaves a stray 0 in number inputs.
async function fill(selector, value) {
  const el = await b.$(selector);
  await el.waitForDisplayed({ timeout: 10000 });
  await b.execute((node, v) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(node, v);
    node.dispatchEvent(new Event('input', { bubbles: true }));
  }, el, String(value));
  const now = await el.getValue();
  if (now !== String(value)) throw new Error(`${selector} reads «${now}», expected «${value}»`);
}
const probe = (subject) => JSON.parse(execFileSync('uv', ['run', '--no-project', 'python', '-I',
  path.join(REPO, 'scripts/imap_test/imap_probe.py'), 'where', USER, subject], { encoding: 'utf8' }));

let ready = await step('asistente hasta el paso de cuentas', async () => {
  await until('the first-run wizard', async () => /STEP \d OF \d/.test(await screen()), 60000);
  await press('Plain email client'); await press('Continue');
  await press('Split view'); await press('Continue');
  await press('IMAP / SMTP');
  return 'Plain email client → Split view → IMAP / SMTP';
});

if (ready) ready = await step('formulario IMAP rellenado y probado', async () => {
  await fill('#add-imap-email', USER);
  await fill('#add-imap-password', PASSWORD);
  await fill('#add-imap-host', HOST);
  await fill("//label[normalize-space()='IMAP port']/following-sibling::input", PORT);
  await fill('#add-imap-smtp-host', HOST);
  await fill("//label[normalize-space()='SMTP port']/following-sibling::input", SMTP_PORT);
  await fill("//label[normalize-space()='Display Name (optional)']/following-sibling::input", 'UI Test');
  await press('Test connection');
  const outcome = await until('the connection test', async () => {
    const s = await screen();
    if (s.includes('Connection successful')) return 'ok';
    const m = s.match(/Connection failed:? [^.]*/);
    return m ? m[0] : null;
  }, 40000);
  if (outcome !== 'ok') throw new Error(outcome);
  return 'Connection successful';
});

if (ready) ready = await step('cuenta añadida y correo sincronizado', async () => {
  await press('Add account');
  const missing = await until('the seeded mail in the inbox', async () => {
    const s = await screen();
    if (/Skip for now/.test(s) && /STEP/.test(s)) await press('Skip for now').catch(() => {});
    const left = SUBJECTS.filter((x) => !s.includes(x));
    return left.length ? null : 'none';
  }, 90000);
  return `${SUBJECTS.length} mensajes en la bandeja (${missing === 'none' ? 'todos' : missing})`;
});

const target = SUBJECTS[0];
if (ready) ready = await step('abrir el mensaje', async () => {
  const opened = await js((t) => {
    const row = [...document.querySelectorAll('[role=row],[role=option],li,button,div')]
      .filter((e) => e.offsetParent !== null && (e.innerText || '').includes(t))
      .sort((a, b2) => a.innerText.length - b2.innerText.length)[0];
    if (!row) return false;
    row.click();
    return true;
  }, target);
  if (!opened) throw new Error(`no row for «${target}»`);
  await until('the reading pane', async () => (await screen()).includes('Synthetic test message'), 15000);
  return target;
});

if (ready) await step('destacar desde la UI llega al servidor', async () => {
  await pressIcon('Star');
  const state = await until('\\Flagged on the server', async () => {
    const s = probe(target);
    return s.flags.includes('\\Flagged') ? s : null;
  }, 20000);
  return `servidor: ${state.folder} ${state.flags.join(' ')}`;
});

if (ready) await step('archivar desde la UI lo mueve a Archive en el servidor', async () => {
  await pressIcon('Archive');
  const state = await until('the message in Archive', async () => {
    const s = probe(target);
    return s.folder === 'Archive' ? s : null;
  }, 20000);
  const stillListed = (await screen()).includes(target);
  if (stillListed) throw new Error('el mensaje sigue en la bandeja de la UI');
  return `servidor: ${state.folder}`;
});

fs.writeFileSync(path.join(outDir, 'imap-ui.json'), JSON.stringify(results, null, 2));
await b.deleteSession().catch(() => {});
const failed = results.filter((r) => r.status !== 'ok').length;
console.log(`\n${results.length - failed}/${results.length} steps passed — evidence in ${outDir}`);
process.exit(failed ? 1 : 0);
