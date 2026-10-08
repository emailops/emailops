// One-session UI sweep of EmailOps through the embedded WebDriver.
// Usage: node sweep.mjs <run_dir>   (env TAURI_WEBDRIVER_PORT, default 4445)
// Writes <run_dir>/sweep/*.png and <run_dir>/sweep/results.json
import { remote } from 'webdriverio';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const runDir = process.argv[2];
// The demo DB of the instance under test, for the side effects a step must prove (a saved draft).
const db = path.join(fs.readFileSync(path.join(runDir, 'data_dir'), 'utf8').trim(), 'emailops.db');
const sql = (query) => { const txt = execFileSync('sqlite3', ['-json', db, query], { encoding: 'utf8' }); return txt.trim() ? JSON.parse(txt) : []; };
const out = path.join(runDir, 'sweep'); fs.mkdirSync(out, { recursive: true });
const port = Number(process.env.TAURI_WEBDRIVER_PORT || 4445);
const b = await remote({ hostname: '127.0.0.1', port, path: '/', capabilities: {}, logLevel: 'error' });
const results = [];
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
let n = 0;
async function shot(name) { const f = `${String(++n).padStart(2,'0')}-${name}.png`; await b.saveScreenshot(path.join(out, f)); return f; }
const js = (fn, ...a) => b.execute(fn, ...a);
const bodyText = () => js(() => document.body.innerText);
const rows = () => js(() => document.querySelectorAll('main div[role="button"], div[role="button"]').length);
const exists = async (sel) => (await b.$(sel)).isExisting();
async function click(sel, t = 5000) { const el = await b.$(sel); await el.waitForClickable({ timeout: t }); await el.click(); }
async function type(sel, text) { const el = await b.$(sel); await el.click(); await el.setValue(text); }
async function enter() { await b.keys('Enter'); await js(() => { const el = document.activeElement; const f = el && el.closest ? el.closest('form') : null; if (f && el.tagName !== 'TEXTAREA') f.requestSubmit(); }); }
async function errorsOnScreen() {
  return js(() => {
    const t = document.body.innerText;
    const hits = [];
    for (const re of [/Something went wrong/i, /Uncaught/i, /TypeError/i, /ReferenceError/i, /undefined is not/i, /\[object Object\]/, /NaN/]) { const m = t.match(re); if (m) hits.push(m[0]); }
    return hits;
  });
}
async function step(feature, name, expect, fn) {
  const rec = { feature, step: name, expect, status: 'ok', detail: '', shot: null, t: new Date().toISOString().slice(11,19) };
  try {
    const detail = await fn();
    rec.detail = typeof detail === 'string' ? detail : JSON.stringify(detail);
    if (rec.detail.startsWith('FAIL:')) rec.status = 'fail';
    if (rec.detail.startsWith('SKIP:')) rec.status = 'skip';
  } catch (e) { rec.status = 'fail'; rec.detail = `FAIL: ${e.message.split('\n')[0]}`; }
  const errs = await errorsOnScreen().catch(() => []);
  if (errs.length) { rec.status = 'fail'; rec.detail += ` | error text on screen: ${errs.join(', ')}`; }
  rec.shot = await shot(name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()).catch(() => null);
  results.push(rec); console.log(`${rec.status.toUpperCase().padEnd(4)} ${feature} / ${name}: ${rec.detail.slice(0, 140)}`);
}
const ok = (cond, good, bad) => cond ? good : `FAIL: ${bad}`;

// ---------- Inbox ----------
await click('button=Inbox'); await sleep(1500);
await step('Inbox', 'lista inicial', 'la bandeja de la cuenta demo muestra filas', async () => { const r = await rows(); return ok(r > 5, `${r} filas`, `solo ${r} filas`); });
await step('Inbox', 'pestañas de categoría', 'las pestañas Primary/Social/… solo existen en cuentas Gmail', async () => {
  const tabs = []; for (const cat of ['Primary', 'Social', 'Updates', 'Forums', 'Promotions']) if (await exists(`button=${cat}`)) tabs.push(cat);
  if (!tabs.length) return 'SKIP: cuentas IMAP de la demo, sin categorías Gmail';
  for (const cat of tabs) { await click(`button=${cat}`); await sleep(1000); }
  return `pestañas: ${tabs.join(', ')}`;
});
await step('Inbox', 'abrir hilo', 'clic en una fila abre el hilo con su asunto como H1 y botón Back', async () => {
  await click('//div[@role="button"][contains(., "Nadia Brunner")]'); await sleep(1500);
  const h1 = await js(() => [...document.querySelectorAll('h1')].map(h => h.textContent.trim()).join(' | '));
  return ok(/How do I add a new email account/.test(h1) && await exists('button=Back'), `H1: ${h1}`, `H1 inesperado: ${h1}`);
});
await step('Inbox', 'menú del hilo', 'los botones Reply, Reply All y AI Draft están presentes', async () => {
  const have = []; for (const t of ['Reply', 'Reply All', 'AI Draft']) if (await exists(`button=${t}`)) have.push(t);
  return ok(have.length === 3, have.join(', '), `faltan: ${['Reply','Reply All','AI Draft'].filter(x => !have.includes(x))}`);
});
await step('Inbox', 'volver con Back', 'Back devuelve a la lista', async () => { await click('button=Back'); await sleep(1200); return ok(await exists('h2*=Inbox') && !(await exists('h1*=How do I add')), 'lista visible', 'el hilo sigue abierto'); });
await step('Inbox', 'menú ⋮ de una fila', 'el menú de acciones de la fila abre con opciones', async () => {
  const btn = await b.$('aria/More actions'); if (!(await btn.isExisting())) return 'FAIL: no hay botón "More actions"';
  await js(el => el.click(), btn); await sleep(800);
  const t = await bodyText(); const items = ['Open in new tab', 'Chat about this thread', 'Block sender'].filter(i => t.includes(i));
  await b.keys('Escape'); await sleep(400);
  return ok(items.length >= 2, `opciones: ${items.join(', ')}`, `opciones visibles: ${items.join(', ') || 'ninguna'}`);
});

// ---------- Helpers de filas y avisos (paridad Gmail/Outlook) ----------
const toastText = () => js(() => document.querySelector('[data-testid="toast-stack"]')?.innerText.replace(/\n/g, ' | ') || '');
const closeToasts = async () => { await js(() => document.querySelectorAll('[data-testid="toast-stack"] button[aria-label="Close"]').forEach((x) => x.click())); await sleep(300); };
const clickToastAction = (label) => js((l) => { const x = [...document.querySelectorAll('[data-testid="toast-stack"] button')].find((y) => y.textContent.trim() === l); if (!x) return false; x.click(); return true; }, label);
async function waitToast(re, ms) { const t0 = Date.now(); while (Date.now() - t0 < ms) { const t = await toastText(); if (re.test(t)) return t; await sleep(400); } return null; }
const rowTexts = () => js(() => [...document.querySelectorAll('div[role="button"]')].map((r) => r.innerText.replace(/\n/g, ' ')));
const hasRow = async (text) => (await rowTexts()).some((t) => t.includes(text));
// Something inside the first row whose text includes `text`: returns null without the row/element, else its aria-pressed (or true) after an optional click.
const inRow = (text, sel, act = false) => js((s, q, a) => { const r = [...document.querySelectorAll('div[role="button"]')].find((x) => x.innerText.includes(s)); const el = r?.querySelector(q); if (!el) return null; if (a) el.click(); return el.getAttribute('aria-pressed') ?? true; }, text, sel, act);
const rowMenu = (text) => inRow(text, '[aria-label="More actions"]', true);
const menuItem = (label) => js((l) => { const x = [...document.querySelectorAll('button')].find((y) => y.textContent.trim() === l && y.offsetParent && !y.closest('[data-testid="bulk-toolbar"], nav, aside, [data-testid="toast-stack"]')); if (!x) return false; x.click(); return true; }, label);
const ids = (rows) => rows.map((r) => `'${String(r).replace(/'/g, "''")}'`).join(',');
const blur = () => js(() => document.activeElement?.blur());
// `aria/Close settings` resolves the accessible name over the whole DOM and can take 45 s with the signature editor open: use the title.
const closeSettings = async () => { await js(() => document.querySelector('button[title="Close settings"]')?.click()); await sleep(800); };

// ---------- Atajos de teclado ----------
await click('button=Inbox'); await sleep(1000);
await step('Atajos', '? abre la ayuda y Escape la cierra', 'con el foco fuera de un campo, «?» abre la lista de atajos por grupos y Escape la cierra', async () => {
  await blur(); await b.keys('?'); await sleep(800);
  const help = await js(() => { const h = document.querySelector('[data-testid="shortcut-help"]'); return h ? { groups: h.querySelectorAll('[data-testid="shortcut-group"]').length, text: h.innerText } : null; });
  await b.keys('Escape'); await sleep(600);
  const closed = !(await exists('[data-testid="shortcut-help"]'));
  return ok(help && help.groups >= 5 && /Next conversation/.test(help.text) && closed, `${help?.groups} grupos; cerrado con Escape`, `ayuda=${JSON.stringify(help && { groups: help.groups })}, cerrada=${closed}`);
});
const cursorRow = () => js(() => document.querySelector('[data-cursor="true"]')?.innerText.replace(/\n/g, ' ').slice(0, 80) ?? null);
await step('Atajos', 'j y k mueven el cursor', 'j baja el cursor de teclado a la fila siguiente y k lo devuelve', async () => {
  await blur();
  const c0 = await cursorRow(); await b.keys('j'); await sleep(400); const c1 = await cursorRow(); await b.keys('k'); await sleep(400); const c2 = await cursorRow();
  return ok(c0 && c1 && c1 !== c0 && c2 === c0, `${c0?.slice(0, 40)} → ${c1?.slice(0, 40)} → de vuelta`, `cursor: ${c0} → ${c1} → ${c2}`);
});
await step('Atajos', 's destaca la conversación del cursor', 's pone la estrella en la fila del cursor (y en la BD) y otra s la quita', async () => {
  await blur();
  const star = () => js(() => document.querySelector('[data-cursor="true"] [data-testid="star-toggle"]')?.getAttribute('aria-pressed') ?? null);
  const starredInDb = () => sql("SELECT COUNT(DISTINCT thread_id) AS n FROM emails WHERE account_id = 'demo-acct-work' AND is_starred = 1")[0]?.n;
  const row = await cursorRow(); if (!row) return 'FAIL: no hay fila con el cursor de teclado';
  if ((await star()) === 'true') return `FAIL: la fila del cursor ya estaba destacada: ${row}`;
  const base = starredInDb();
  await b.keys('s'); await sleep(1500); const on = await star(); const dbOn = starredInDb();
  await b.keys('s'); await sleep(1500); const off = await star(); const dbOff = starredInDb();
  return ok(on === 'true' && off === 'false' && dbOn === base + 1 && dbOff === base, `«${row.slice(0, 50)}»: estrella puesta y quitada; hilos destacados en BD ${base} → ${dbOn} → ${dbOff}`, `UI ${on} → ${off}, BD ${base} → ${dbOn} → ${dbOff}`);
});

// The open conversation's subject (the reading pane header; the app has other h1s).
const h1Text = () => js(() => document.querySelector('header h1')?.textContent.trim() || '');
const listRowTexts = () => js(() => [...document.querySelectorAll('div[role="button"]')].filter((r) => r.querySelector('[data-testid="row-select"]')).map((r) => r.innerText.replace(/\n/g, ' ')));
await step('Atajos', 'e archiva y abre la siguiente', 'con una conversación abierta, «e» la archiva y abre la siguiente de la lista (auto-avance por defecto); Undo la devuelve a la lista sin quitar la que se lee', async () => {
  await closeToasts(); await blur();
  const list = await listRowTexts(); if (list.length < 3) return `FAIL: la lista solo tiene ${list.length} filas`;
  await js(() => [...document.querySelectorAll('div[role="button"]')].filter((r) => r.querySelector('[data-testid="row-select"]'))[1].click()); await sleep(1500);
  const first = await h1Text(); if (!first || !list[1].includes(first)) return `FAIL: no se abrió la segunda fila (H1 «${first}»)`;
  await blur(); await b.keys('e'); await sleep(1500);
  const next = await h1Text(); const t = await toastText();
  const undone = await clickToastAction('Undo'); await sleep(1200);
  const stillOpen = await h1Text(); await closeToasts();
  if (await exists('button=Back')) { await click('button=Back'); await sleep(1000); }
  const back = (await listRowTexts()).some((r) => r.includes(first));
  return ok(next !== first && list[2].includes(next) && /Archived 1 conversation/.test(t) && undone && stillOpen === next && back,
    `«${first.slice(0, 40)}» archivada → abierta «${next.slice(0, 40)}»; Undo la devuelve a la lista`,
    `antes «${first}», después «${next}» (esperada la fila 3: ${list[2].slice(0, 60)}), aviso «${t}», undo=${undone}, abierta tras Undo «${stillOpen}», de vuelta=${back}`);
});
await step('Atajos', 'nada actúa con el menú ⋮ abierto', 'con el menú ⋮ de una fila abierto, «#», «e» y «j» no hacen nada (ni borran ni archivan ni mueven el cursor); Escape cierra el menú', async () => {
  await closeToasts(); await blur(); await sleep(1000);
  const before = await listRowTexts(); const cursor0 = await cursorRow();
  await js(() => [...document.querySelectorAll('div[role="button"]')].filter((r) => r.querySelector('[data-testid="row-select"]'))[2]?.querySelector('[aria-label="More actions"]')?.click()); await sleep(700);
  const menuOpen = /Block sender/.test(await bodyText()); if (!menuOpen) return 'FAIL: el menú ⋮ no se abrió';
  await b.keys('#'); await sleep(300); await b.keys('e'); await sleep(300); await b.keys('j'); await sleep(800);
  const after = await listRowTexts(); const t = await toastText(); const cursor1 = await cursorRow();
  await b.keys('Escape'); await sleep(500); const closed = !/Block sender/.test(await bodyText());
  // The list may grow (paging, a late refresh); no row may leave it.
  const left = before.filter((r) => !after.includes(r));
  return ok(!left.length && !/Deleted|Archived/.test(t) && cursor1 === cursor0 && closed,
    `ninguna fila salió de la lista, sin aviso, cursor quieto; Escape cerró el menú`,
    `filas que salieron: ${left.join(' // ').slice(0, 120) || 'ninguna'}, aviso «${t}», cursor ${cursor0} → ${cursor1}, menú cerrado=${closed}`);
});

// ---------- Organizar: estrella, archivo, selección múltiple, deshacer, posponer ----------
await step('Organizar', 'destacar desde la fila', 'la estrella de la fila de Nadia Brunner queda pulsada y la BD marca el hilo como destacado', async () => {
  const before = await inRow('Nadia Brunner', '[data-testid="star-toggle"]');
  if (before === null) return 'FAIL: la fila de Nadia Brunner no tiene estrella';
  if (before === 'true') { await inRow('Nadia Brunner', '[data-testid="star-toggle"]', true); await sleep(1200); }
  await inRow('Nadia Brunner', '[data-testid="star-toggle"]', true); await sleep(1500);
  const pressed = await inRow('Nadia Brunner', '[data-testid="star-toggle"]');
  const dbs = sql("SELECT MAX(is_starred) AS s FROM emails WHERE sender = 'Nadia Brunner'")[0]?.s;
  return ok(pressed === 'true' && dbs === 1, 'estrella pulsada; emails.is_starred = 1', `aria-pressed=${pressed}, BD is_starred=${dbs}`);
});
await step('Organizar', 'vista Starred', 'Starred lista el hilo recién destacado y el que la BD demo trae destacado (Corrected Larkspur Freight renewal quote)', async () => {
  await click('button*=Starred'); await sleep(1500);
  const h2 = await js(() => document.querySelector('h2')?.textContent.trim() || ''); const rows = await rowTexts();
  const want = ['Nadia Brunner', 'Corrected Larkspur Freight renewal quote'].filter((x) => !rows.some((r) => r.includes(x)));
  return ok(/^Starred/.test(h2) && !want.length, `${h2}: ${rows.length} filas`, `cabecera «${h2}», faltan: ${want.join(', ')} (¿BD demo sin la fixture destacada?)`);
});
await step('Organizar', 'quitar la estrella en Starred', 'quitar la estrella saca el hilo de Starred y de la BD; la fixture destacada se queda', async () => {
  await inRow('Nadia Brunner', '[data-testid="star-toggle"]', true); await sleep(1500);
  const rows = await rowTexts(); const dbs = sql("SELECT MAX(is_starred) AS s FROM emails WHERE sender = 'Nadia Brunner'")[0]?.s;
  const gone = !rows.some((r) => r.includes('Nadia Brunner')), kept = rows.some((r) => r.includes('Corrected Larkspur'));
  return ok(gone && kept && dbs === 0, 'fuera de Starred; BD is_starred = 0', `fuera=${gone}, fixture=${kept}, BD=${dbs}`);
});
await step('Organizar', 'vista Archive', 'una cuenta IMAP no muestra la vista Archive (archiva en su carpeta); con All accounts, Archive lista el correo archivado de la BD demo', async () => {
  await click('button*=ulises@emailopslabs.dev'); await sleep(1000);
  const imapHidden = !(await exists('button=Archive'));
  await click('button=All accounts'); await sleep(1500);
  if (!(await exists('button=Archive'))) { await click('button*=ulises@emailopslabs.dev'); await sleep(800); return 'FAIL: sin entrada Archive con All accounts'; }
  await click('button=Archive'); await sleep(1500);
  const h2 = await js(() => document.querySelector('h2')?.textContent.trim() || ''); const has = await hasRow('Studio key handover confirmed');
  await click('button*=ulises@emailopslabs.dev'); await sleep(1000); await click('button=Inbox'); await sleep(1200);
  return ok(imapHidden && /^Archive/.test(h2) && has, `IMAP sin Archive; ${h2} con «Studio key handover confirmed»`, `IMAP oculta=${imapHidden}, cabecera «${h2}», fila archivada=${has}`);
});
await step('Organizar', 'selección múltiple', 'marcar dos casillas muestra la barra de acciones con «2 selected» y Archive, Snooze, Delete, Mark as unread, Star', async () => {
  await inRow('Kwame Boateng', '[data-testid="row-select"]', true); await inRow('GlitchTip', '[data-testid="row-select"]', true); await sleep(600);
  const bar = await js(() => { const t = document.querySelector('[data-testid="bulk-toolbar"]'); return t ? { text: t.innerText.replace(/\n/g, ' '), buttons: [...t.querySelectorAll('button')].map((x) => x.getAttribute('aria-label') || x.title || x.textContent.trim()) } : null; });
  if (!bar) return 'FAIL: no aparece la barra de acciones en bloque';
  const want = ['Archive', 'Snooze', 'Delete', 'Mark as unread', 'Star'].filter((x) => !bar.buttons.includes(x));
  return ok(/2 selected/.test(bar.text) && !want.length, `${bar.text.trim()}; ${bar.buttons.join(', ')}`, `texto «${bar.text}», faltan: ${want.join(', ')}`);
});
await step('Organizar', 'archivar en bloque y deshacer', 'Archive en la barra quita las dos filas con un aviso «Archived 2 conversations · Undo»; Undo las devuelve y la BD nunca las movió', async () => {
  const hit = await js(() => { const x = [...document.querySelectorAll('[data-testid="bulk-toolbar"] button')].find((y) => (y.getAttribute('aria-label') || y.title || y.textContent.trim()) === 'Archive'); if (!x) return false; x.click(); return true; });
  if (!hit) return 'FAIL: no hay Archive en la barra';
  await sleep(800);
  const gone = !(await hasRow('Kwame Boateng')) && !(await hasRow('GlitchTip')); const t = await toastText();
  const undone = await clickToastAction('Undo'); await sleep(1500);
  const back = (await hasRow('Kwame Boateng')) && (await hasRow('GlitchTip'));
  await sleep(7000); // past the 6 s window: nothing may reach the provider after Undo
  const mb = sql("SELECT DISTINCT mailbox FROM emails WHERE sender IN ('Kwame Boateng', 'GlitchTip')").map((r) => r.mailbox);
  const late = await toastText();
  return ok(gone && /Archived 2 conversations/.test(t) && undone && back && mb.join() === 'inbox' && !/Could not archive/.test(late), `aviso «${t}»; Undo devuelve las filas; BD: ${mb.join()}`, `quitadas=${gone}, aviso «${t}», undo=${undone}, de vuelta=${back}, BD=${mb.join()}, aviso tardío «${late}»`);
});
await step('Organizar', 'archivar sin credenciales', 'archivar en la cuenta demo sin credenciales: tras la ventana de deshacer falla con un aviso visible y la fila vuelve a la bandeja (la BD no cambia)', async () => {
  await closeToasts();
  if (!(await rowMenu('GlitchTip'))) return 'FAIL: la fila de GlitchTip no tiene menú ⋮';
  await sleep(700);
  if (!(await menuItem('Archive'))) { await b.keys('Escape'); return 'FAIL: el menú ⋮ no ofrece Archive'; }
  await sleep(600); const gone = !(await hasRow('GlitchTip'));
  const err = await waitToast(/Could not archive 1 conversation/, 20000);
  await sleep(800); const back = await hasRow('GlitchTip');
  const mb = sql("SELECT DISTINCT mailbox FROM emails WHERE sender = 'GlitchTip'").map((r) => r.mailbox).join();
  await closeToasts();
  return ok(gone && !!err && back && mb === 'inbox', `aviso: ${err?.slice(0, 120)}; fila restaurada; BD inbox`, `quitada=${gone}, aviso=${err}, restaurada=${back}, BD=${mb}`);
});
const kwameThread = () => sql("SELECT DISTINCT thread_id FROM emails WHERE sender = 'Kwame Boateng' AND account_id = 'demo-acct-work'").map((r) => r.thread_id);
await step('Organizar', 'posponer desde el menú', 'Snooze en el menú ⋮ ofrece momentos predefinidos; elegir el primero saca el hilo de la bandeja y lo guarda en thread_snoozes', async () => {
  await closeToasts();
  if (!(await rowMenu('Kwame Boateng'))) return 'FAIL: la fila de Kwame Boateng no tiene menú ⋮';
  await sleep(700);
  if (!(await exists('[data-testid="row-snooze"]'))) { await b.keys('Escape'); return 'FAIL: el menú ⋮ no ofrece Snooze'; }
  await click('[data-testid="row-snooze"]'); await sleep(600);
  const presets = await js(() => [...document.querySelectorAll('[data-testid^="snooze-preset-"]')].map((x) => x.innerText.replace(/\n/g, ' ')));
  if (!presets.length) { await b.keys('Escape'); return 'FAIL: el selector no ofrece ningún momento'; }
  await js(() => document.querySelector('[data-testid^="snooze-preset-"]').click()); await sleep(1500);
  const t = await toastText(); const gone = !(await hasRow('Kwame Boateng'));
  const thr = kwameThread(); const rows = sql(`SELECT snoozed_until, woke_at FROM thread_snoozes WHERE thread_id IN (${ids(thr)})`);
  const future = rows.length === 1 && rows[0].snoozed_until > Date.now() / 1000 && rows[0].woke_at === null;
  await closeToasts();
  return ok(gone && future && /Snoozed until/.test(t), `«${presets[0]}»; aviso «${t.slice(0, 60)}»; thread_snoozes con hora futura`, `fuera=${gone}, BD=${JSON.stringify(rows)}, aviso «${t}»`);
});
await step('Organizar', 'vista Snoozed', 'Snoozed lista el hilo pospuesto con su marca «Snoozed until …»', async () => {
  await click('button*=Snoozed'); await sleep(1500);
  const has = await hasRow('Kwame Boateng'); const badge = await inRow('Kwame Boateng', '[data-testid="snooze-badge"]');
  return ok(has && badge !== null, 'hilo listado con su marca', `fila=${has}, marca=${badge}`);
});
await step('Organizar', 'quitar el aplazamiento', 'Unsnooze en el menú ⋮ saca el hilo de Snoozed, borra su fila de thread_snoozes y lo devuelve a la bandeja', async () => {
  if (!(await rowMenu('Kwame Boateng'))) return 'FAIL: la fila pospuesta no tiene menú ⋮';
  await sleep(700);
  if (!(await exists('[data-testid="row-unsnooze"]'))) { await b.keys('Escape'); return 'FAIL: el menú ⋮ no ofrece Unsnooze'; }
  await click('[data-testid="row-unsnooze"]'); await sleep(1500);
  const gone = !(await hasRow('Kwame Boateng'));
  const left = sql(`SELECT COUNT(*) AS n FROM thread_snoozes WHERE thread_id IN (${ids(kwameThread())})`)[0]?.n;
  await click('button=Inbox'); await sleep(1500); const back = await hasRow('Kwame Boateng');
  await closeToasts();
  return ok(gone && left === 0 && back, 'fuera de Snoozed; BD sin aplazamiento; de vuelta en Inbox', `fuera=${gone}, filas en BD=${left}, en Inbox=${back}`);
});

// ---------- Search ----------
await step('Búsqueda', 'consulta Ollama', 'filtra a la fila de Kwame Boateng', async () => {
  await type('input[placeholder^="Search…"]', 'Ollama'); await enter(); await sleep(1500);
  return ok(await rows() === 1 && await exists('//div[@role="button"][contains(., "Ollama")]'), '1 fila, Ollama', `${await rows()} filas`);
});
await step('Búsqueda', 'sin resultados', 'muestra "No emails match your search"', async () => {
  await type('input[placeholder^="Search…"]', 'zzzz-no-such-term'); await enter(); await sleep(1500);
  return ok((await bodyText()).includes('No emails match your search') && await rows() === 0, 'estado vacío correcto', 'no aparece el estado vacío');
});
await step('Búsqueda', 'limpiar', 'la ✕ vacía el cuadro y restaura la lista', async () => {
  await click('form:has(input[placeholder^="Search…"]) button'); await sleep(1500);
  const v = await js(() => document.querySelector('input[placeholder^="Search…"]').value);
  return ok(v === '' && await rows() > 5, `${await rows()} filas`, `valor "${v}", ${await rows()} filas`);
});

// ---------- Smart filters → cuadro de búsqueda (DECISIONS 2026-10-08) ----------
const searchValue = () => js(() => document.querySelector('input[placeholder^="Search…"]')?.value ?? null);
// First filter of a sidebar tag group (heading = the tag type), as { value, active }.
const firstSmartFilter = (group) => js((g) => {
  const h = [...document.querySelectorAll('nav h3')].find((x) => x.textContent.trim().toLowerCase() === g);
  const btn = h?.nextElementSibling?.querySelector('li > button');
  return btn ? { value: btn.querySelector('span.truncate')?.textContent.trim() || '', active: btn.className.includes('bg-primary-600') } : null;
}, group);
const smartFilterButton = (group, rightClick) => js((g, r) => {
  const h = [...document.querySelectorAll('nav h3')].find((x) => x.textContent.trim().toLowerCase() === g);
  const btn = h?.nextElementSibling?.querySelector('li > button');
  if (!btn) return false;
  if (r) btn.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 40 }));
  else btn.click();
  return true;
}, group, rightClick);
const ensureTopicFilters = async () => {
  if (await firstSmartFilter('topic')) return true;
  await js(() => document.querySelector('button[title="Recalculate filters"]')?.click()); await sleep(4000);
  return Boolean(await firstSmartFilter('topic'));
};
await step('Búsqueda', 'smart filter escribe su consulta', 'un clic en un smart filter de topic pone tag:topic=<valor> en el cuadro, lo resalta y lista filas; un segundo clic vacía el cuadro', async () => {
  if (!(await ensureTopicFilters())) return 'FAIL: la barra lateral no muestra smart filters de topic (¿BD demo sin etiquetas?)';
  const { value } = await firstSmartFilter('topic');
  await smartFilterButton('topic', false); await sleep(1500);
  const q = await searchValue(); const active = (await firstSmartFilter('topic'))?.active; const r = await rows();
  await smartFilterButton('topic', false); await sleep(1500);
  const cleared = await searchValue(); const back = await rows();
  return ok(q === `tag:topic=${value}` && active && r > 0 && cleared === '' && back > 5,
    `«${q}», resaltado, ${r} filas; segundo clic: cuadro vacío, ${back} filas`,
    `cuadro «${q}» (esperado tag:topic=${value}), resaltado=${active}, ${r} filas; tras el segundo clic «${cleared}», ${back} filas`);
});
await step('Búsqueda', 'botón derecho añade el filtro a la búsqueda', 'con «invoice» en el cuadro, botón derecho → Add to search deja «invoice tag:topic=<valor>» y no resalta el filtro (la consulta ya no es solo él)', async () => {
  if (!(await ensureTopicFilters())) return 'FAIL: la barra lateral no muestra smart filters de topic';
  const { value } = await firstSmartFilter('topic');
  await type('input[placeholder^="Search…"]', 'invoice'); await enter(); await sleep(1500);
  await smartFilterButton('topic', true); await sleep(500);
  const added = await js(() => { const x = [...document.querySelectorAll('[role="menuitem"]')].find((y) => y.textContent.trim() === 'Add to search'); if (!x) return false; x.click(); return true; });
  await sleep(1500);
  const q = await searchValue(); const active = (await firstSmartFilter('topic'))?.active;
  await click('form:has(input[placeholder^="Search…"]) button'); await sleep(1200);
  return ok(added && q === `invoice tag:topic=${value}` && !active,
    `«${q}», filtro sin resaltar`,
    `menú=${added}, cuadro «${q}» (esperado «invoice tag:topic=${value}»), resaltado=${active}`);
});

// ---------- Correo de verificación (scripts/generate_demo_db.py, insert_verification_fixtures) ----------
const SEARCH = 'input[placeholder^="Search…"]';
const clearSearch = async () => { await click('form:has(input[placeholder^="Search…"]) button'); await sleep(1200); };
await step('Búsqueda', 'correo en la papelera fuera de los resultados', 'buscar «Larkspur» lista solo la corrección: el presupuesto que se envió a la papelera no aparece', async () => {
  await type(SEARCH, 'Larkspur'); await enter(); await sleep(1500);
  const found = await js(() => [...document.querySelectorAll('div[role="button"]')].map((r) => r.innerText.replace(/\n/g, ' | ').slice(0, 90)));
  return ok(found.length === 1 && /Corrected Larkspur Freight renewal quote/.test(found[0]), `1 fila: ${found[0]}`, `${found.length} filas: ${found.join(' // ') || 'ninguna (¿BD demo sin las fixtures de verificación?)'}`);
});
await step('Adjuntos', 'tipo peligroso pide confirmación', 'abrir un acceso directo (.webloc) guardado pide confirmación con su nombre y su tipo, y Cancel cierra sin abrir nada', async () => {
  await click('//div[@role="button"][contains(., "Corrected Larkspur Freight renewal quote")]'); await sleep(1500);
  const chip = 'button[title^="larkspur-client-portal.webloc"]';
  if (!(await exists(chip))) return 'FAIL: el correo no muestra el adjunto larkspur-client-portal.webloc';
  await click(chip); await sleep(1200);
  const text = await js(() => { const c = document.querySelector('[data-testid="cancel-open-attachment"]'); return c ? (c.closest('.fixed') || c.parentElement.parentElement).innerText : null; });
  if (text === null) return 'FAIL: no aparece el diálogo de confirmación';
  const named = text.includes('larkspur-client-portal.webloc') && /shortcut/i.test(text) && await exists('[data-testid="confirm-open-attachment"]');
  // Never "Open anyway": that hands the file to the OS.
  await click('[data-testid="cancel-open-attachment"]'); await sleep(800);
  const closed = !(await exists('[data-testid="cancel-open-attachment"]'));
  const opened = /Opening attachment 'larkspur-client-portal/.test(await bodyText());
  return ok(named && closed && !opened, 'diálogo con nombre y tipo; Cancel lo cierra', `nombre y tipo=${named}, cerrado=${closed}, abierto=${opened}; texto: ${text.replace(/\n/g, ' ').slice(0, 160)}`);
});
await step('Adjuntos', 'página web en vista previa aislada', 'un adjunto HTML se abre dentro de la app, en un iframe con sandbox vacío, sin diálogo ni aplicación externa', async () => {
  const chip = 'button[title^="larkspur-renewal-terms.html"]';
  if (!(await exists(chip))) return 'FAIL: el correo no muestra el adjunto larkspur-renewal-terms.html';
  await click(chip); await sleep(1500);
  const frame = await js(() => { const i = document.querySelector('iframe[title="larkspur-renewal-terms.html"]'); return i ? { sandbox: i.getAttribute('sandbox'), src: (i.getAttribute('src') || '').slice(0, 22) } : null; });
  const dialog = await exists('[data-testid="cancel-open-attachment"]');
  await js(() => document.querySelector('button[title="Close"]')?.click()); await sleep(800);
  return ok(!!frame && frame.sandbox === '' && frame.src === 'data:text/html;base64,' && !dialog, JSON.stringify(frame), `iframe=${JSON.stringify(frame)}, diálogo=${dialog}`);
});
const LARKSPUR = 'Corrected Larkspur Freight renewal quote';
const larkspurState = () => sql(`SELECT mailbox, MAX(is_starred) AS starred FROM emails WHERE subject = '${LARKSPUR}' GROUP BY mailbox`);
await step('Atajos', 'la ayuda emergente nombra la tecla', 'los botones de la conversación abierta muestran su atajo: «Archive (E)», «Delete thread (#)», «Reply (R)»', async () => {
  if (!(await exists(`//div[@role="button"][contains(., "${LARKSPUR}")]`))) return 'FAIL: la búsqueda «Larkspur» ya no lista la corrección';
  await click(`//div[@role="button"][contains(., "${LARKSPUR}")]`); await sleep(1500);
  const titles = await js(() => [...document.querySelectorAll('header button')].map((x) => x.title).filter(Boolean));
  const want = ['Archive (E)', 'Delete thread (#)', 'Reply (R)'].filter((x) => !titles.includes(x));
  return ok(!want.length, titles.filter((x) => /\(.+\)$/.test(x)).join(', '), `faltan: ${want.join(', ')}; títulos: ${titles.join(', ')}`);
});
await step('Atajos', 'nada actúa tras el visor de imágenes', 'con la imagen adjunta abierta en el visor, «#», «e», «s» y «j» no tocan la conversación de detrás; Escape cierra el visor', async () => {
  const chip = 'button[title^="larkspur-dock-photo.png"]';
  if (!(await exists(chip))) return 'FAIL: el correo no muestra la imagen larkspur-dock-photo.png (¿BD demo anterior al fixture? make demo-db)';
  const before = JSON.stringify(larkspurState()); const h0 = await h1Text();
  await click(chip); await sleep(1200);
  const open = await exists('img[alt="larkspur-dock-photo.png"]'); if (!open) return 'FAIL: no se abrió el visor de imágenes';
  await blur(); for (const k of ['#', 'e', 's', 'j']) { await b.keys(k); await sleep(300); } await sleep(1000);
  const after = JSON.stringify(larkspurState()); const t = await toastText(); const stillOpen = await exists('img[alt="larkspur-dock-photo.png"]');
  await b.keys('Escape'); await sleep(600); const closed = !(await exists('img[alt="larkspur-dock-photo.png"]'));
  return ok(stillOpen && after === before && !/Deleted|Archived|Snoozed/.test(t) && (await h1Text()) === h0 && closed,
    `visor abierto; BD ${after}; sin aviso; Escape lo cierra`, `visor=${stillOpen}, BD ${before} → ${after}, aviso «${t}», cerrado=${closed}`);
});
await step('Atajos', 'nada actúa tras Ajustes', 'con Ajustes abierto sobre la conversación, «#», «e» y «s» no la tocan', async () => {
  const before = JSON.stringify(larkspurState()); const h0 = await h1Text();
  await click('aria/Application settings'); await sleep(1200);
  await blur(); for (const k of ['#', 'e', 's']) { await b.keys(k); await sleep(300); } await sleep(1000);
  const after = JSON.stringify(larkspurState()); const t = await toastText();
  await closeSettings();
  const h1 = await h1Text();
  if (await exists('button=Back')) { await click('button=Back'); await sleep(800); }
  return ok(after === before && !/Deleted|Archived|Snoozed/.test(t) && h1 === h0, `BD ${after}; sin aviso; la conversación sigue abierta`, `BD ${before} → ${after}, aviso «${t}», H1 «${h0}» → «${h1}»`);
});
await step('Inbox', 'imágenes remotas bloqueadas', 'un correo con una imagen remota muestra el aviso y «Show images», y el cuerpo se pinta sin la URL de la imagen', async () => {
  await type(SEARCH, 'Harborlight'); await enter(); await sleep(1500);
  if (!(await exists('//div[@role="button"][contains(., "Harborlight Weekly")]'))) { await clearSearch(); return 'FAIL: no hay correo de Harborlight Weekly (¿BD demo sin las fixtures de verificación?)'; }
  await click('//div[@role="button"][contains(., "Harborlight Weekly")]'); await sleep(1500);
  const r = await js(() => {
    const doc = document.querySelector('iframe[title="Email content"]')?.getAttribute('srcdoc') || '';
    return { banner: /Remote images were blocked/.test(document.body.innerText), show: [...document.querySelectorAll('button')].some((x) => x.textContent.trim() === 'Show images'),
      img: (doc.match(/<img[^>]*>/) || [''])[0], imgSrc: (doc.match(/img-src[^";]*/) || [''])[0] };
  });
  // Never "Show images": that would ask the sender's server for the picture.
  await click('button=Back'); await sleep(800); await clearSearch();
  return ok(r.banner && r.show && r.img !== '' && !/\ssrc=/.test(r.img) && !/https?:/.test(r.imgSrc), JSON.stringify(r), JSON.stringify(r));
});
await step('Junk', 'chip en la bandeja', 'el aviso que el usuario marcó como no deseado lleva su chip «phishing» en la fila; el aviso legítimo del mismo proveedor no', async () => {
  await type(SEARCH, 'Tessellate'); await enter(); await sleep(1500);
  const found = await js(() => [...document.querySelectorAll('div[role="button"]')].map((r) => ({ subject: r.innerText.replace(/\n/g, ' | ').slice(0, 200), chip: r.querySelector('span[title^="junk:"]')?.title || null })));
  await clearSearch();
  const marked = found.find((r) => /payment failed, plan suspended/.test(r.subject)), genuine = found.find((r) => /plan renews in March/.test(r.subject));
  return ok(!!marked && !!genuine && marked.chip === 'junk: phishing' && genuine.chip === null, `marcado: ${marked?.chip}; legítimo: sin chip`, JSON.stringify(found));
});

// ---------- Remitentes: darse de baja y bloquear (boletín de la BD demo con List-Unsubscribe) ----------
const HARBOR = 'news@harborlight-weekly.example';
const dialogWith = (re) => js((r) => { const d = [...document.querySelectorAll('[role="dialog"], .fixed')].find((x) => new RegExp(r).test(x.innerText.trim())); return d ? d.innerText : null; }, re);
const dialogButton = (re, label) => js((r, l) => { const d = [...document.querySelectorAll('[role="dialog"], .fixed')].find((x) => new RegExp(r).test(x.innerText.trim())); const x = d && [...d.querySelectorAll('button')].filter((y) => y.textContent.trim() === l).pop(); if (!x) return false; x.click(); return true; }, re, label);
await step('Remitentes', 'Unsubscribe: diálogo y Cancel', 'el boletín con List-Unsubscribe-Post ofrece Unsubscribe; el diálogo explica la petición de un clic a harborlight-weekly.example y Cancel lo cierra sin pedir nada', async () => {
  await type(SEARCH, 'Harborlight'); await enter(); await sleep(1500);
  if (!(await exists('//div[@role="button"][contains(., "Harborlight Weekly")]'))) { await clearSearch(); return 'FAIL: no hay correo de Harborlight Weekly (¿BD demo sin las fixtures de verificación?)'; }
  await click('//div[@role="button"][contains(., "Harborlight Weekly")]'); await sleep(1500);
  if (!(await exists('[data-testid="unsubscribe-button"]'))) { await click('button=Back'); await clearSearch(); return 'FAIL: el correo no ofrece Unsubscribe (¿sin cabeceras List-Unsubscribe en email_headers?)'; }
  await click('[data-testid="unsubscribe-button"]'); await sleep(1000);
  const text = await dialogWith('^Unsubscribe from');
  // Never "Unsubscribe": that would POST to the sender's host.
  await dialogButton('^Unsubscribe from', 'Cancel'); await sleep(800);
  const closed = !(await exists('[data-testid="unsubscribe-confirm"]'));
  const stored = sql('SELECT COUNT(*) AS n FROM sender_unsubscribes')[0]?.n;
  return ok(!!text && /harborlight-weekly\.example/.test(text) && /request/.test(text) && closed && stored === 0, 'diálogo de un clic con el host del remitente; Cancel lo cierra; sender_unsubscribes vacío', `diálogo=${text?.replace(/\n/g, ' ').slice(0, 160)}, cerrado=${closed}, filas=${stored}`);
});
await step('Remitentes', 'bloquear remitente', 'Block sender en el menú ⋮ pide confirmación; al confirmar se guarda el bloqueo y, sin credenciales, el aviso dice que el correo existente no se pudo mover (se queda en la bandeja marcado como no deseado aquí)', async () => {
  await closeToasts(); await click('button=Back'); await sleep(1000);
  if (!(await rowMenu('Harborlight Weekly'))) return 'FAIL: la fila del boletín no tiene menú ⋮';
  await sleep(700);
  if (!(await exists('[data-testid="menu-block-sender"]'))) { await b.keys('Escape'); return 'FAIL: el menú ⋮ no ofrece Block sender'; }
  await click('[data-testid="menu-block-sender"]'); await sleep(1000);
  const text = await dialogWith(`^Block ${HARBOR}`);
  if (!text) return 'FAIL: no aparece el diálogo «Block …?»';
  await click('[data-testid="sender-block-confirm"]');
  const t = await waitToast(/Blocked news@harborlight-weekly\.example/, 10000);
  const blocked = sql(`SELECT COUNT(*) AS n FROM blocked_senders WHERE account_id = 'demo-acct-work' AND address = '${HARBOR}'`)[0]?.n;
  const mail = sql(`SELECT e.mailbox, j.user_override FROM emails e LEFT JOIN email_junk j ON j.email_id = e.id WHERE e.sender_email = '${HARBOR}'`);
  await closeToasts();
  return ok(blocked === 1 && /could not be moved: 1/.test(t || '') && mail.length === 1 && mail[0].mailbox === 'inbox' && mail[0].user_override === 'junk',
    `aviso «${t}»; blocked_senders 1; el correo sigue en inbox marcado junk`, `aviso «${t}», bloqueos=${blocked}, correo=${JSON.stringify(mail)}`);
});
await step('Remitentes', 'aviso de remitente bloqueado en el hilo', 'abrir un correo del remitente bloqueado muestra el aviso con Unblock', async () => {
  await click('//div[@role="button"][contains(., "Harborlight Weekly")]'); await sleep(1500);
  const banner = await js(() => document.querySelector('[data-testid="blocked-sender-banner"]')?.innerText.replace(/\n/g, ' ') || null);
  await click('button=Back'); await sleep(800); await clearSearch();
  return ok(!!banner && /Unblock/.test(banner), banner, 'no aparece el aviso de remitente bloqueado');
});
await step('Remitentes', 'Ajustes → Junk: lista y desbloqueo', 'Blocked senders lista el remitente; Unblock (con «devolver su correo») lo quita de la lista y de la BD y olvida la marca de no deseado que dejó el bloqueo', async () => {
  await click('aria/Application settings'); await sleep(1200);
  await js(() => [...document.querySelectorAll('button')].filter((x) => x.textContent.includes('Junk')).pop()?.click()); await sleep(1200);
  const list = await js(() => document.querySelector('[data-testid="blocked-senders"]')?.innerText || '');
  if (!list.includes(HARBOR)) { await closeSettings(); return `FAIL: Blocked senders no lista ${HARBOR}: ${list.replace(/\n/g, ' ').slice(0, 120)}`; }
  await js(() => [...document.querySelectorAll('[data-testid="blocked-senders"] button')].find((x) => x.textContent.trim() === 'Unblock')?.click()); await sleep(1000);
  if (!(await dialogButton(`^Unblock ${HARBOR}`, 'Unblock'))) { await closeSettings(); return 'FAIL: no aparece el diálogo «Unblock …?»'; }
  const t = await waitToast(/Unblocked/, 10000); await sleep(600);
  const after = await js(() => document.querySelector('[data-testid="blocked-senders"]')?.innerText || '');
  const blocked = sql(`SELECT COUNT(*) AS n FROM blocked_senders WHERE address = '${HARBOR}'`)[0]?.n;
  const mark = sql(`SELECT j.user_override FROM emails e LEFT JOIN email_junk j ON j.email_id = e.id WHERE e.sender_email = '${HARBOR}'`)[0]?.user_override ?? null;
  await closeToasts(); await closeSettings();
  return ok(!!t && !after.includes(HARBOR) && blocked === 0 && mark === null, `aviso «${t}»; lista vacía; BD sin bloqueo ni marca junk`, `aviso «${t}», en lista=${after.includes(HARBOR)}, bloqueos=${blocked}, marca=${mark}`);
});

// ---------- Cuentas ----------
await step('Cuentas', 'cambiar a fastmail', 'la lista cambia a la cuenta personal', async () => {
  await click('button*=ulises@fastmail.com'); await sleep(1500);
  const h2 = await js(() => document.querySelector('h2')?.textContent.trim());
  return ok(/Inbox/.test(h2 || ''), `${h2}, ${await rows()} filas`, `cabecera: ${h2}`);
});
await step('Cuentas', 'All accounts', 'la vista unificada muestra más filas que una cuenta sola', async () => {
  const single = await rows(); await click('button=All accounts'); await sleep(1500); const all = await rows();
  return ok(all >= single, `${all} filas (cuenta sola: ${single})`, `${all} < ${single}`);
});
await step('Cuentas', 'volver a la cuenta de trabajo', 'la cuenta demo-acct-work vuelve a estar seleccionada', async () => { await click('button*=ulises@emailopslabs.dev'); await sleep(1200); return `${await rows()} filas`; });

// ---------- Otras vistas ----------
if (!(await exists('button=Spam'))) { await click('button=Other Views'); await sleep(800); }
const optional = { Calendar: 'el calendario solo se activa en cuentas Gmail/Outlook' };
for (const view of ['Tag Board', 'Attachments', 'Drafts', 'Scheduled', 'Sent', 'Starred', 'Snoozed', 'Calendar', 'Spam', 'Deleted', 'Contacts', 'Dashboard', 'Tasks', 'Lenses', 'Memory']) {
  await step('Vistas', view, `la vista ${view} abre sin errores y con contenido`, async () => {
    const sel = `button*=${view}`;
    if (!(await exists(sel))) return optional[view] ? `SKIP: ${optional[view]}` : `FAIL: no hay entrada "${view}" en la barra lateral`;
    await click(sel); await sleep(1800);
    const t = (await js(() => (document.querySelector('main') || document.body).innerText)).replace(/\s+/g, ' ').trim();
    const h2 = await js(() => document.querySelector('h2')?.textContent.trim() || '');
    if (['Sent', 'Spam', 'Deleted', 'Drafts', 'Attachments'].includes(view) && h2 && !new RegExp(view, 'i').test(h2) && /Inbox/.test(h2)) return `FAIL: la cabecera dice "${h2}" en la vista ${view}`;
    return ok(t.length > 20, t.slice(0, 120), `vista casi vacía: "${t.slice(0, 60)}"`);
  });
}

// ---------- Tag Board ----------
await click('button=Tag Board'); await sleep(1800);
await step('Tag Board', 'bloques', 'hay bloques con hilos', async () => { const r = await rows(); return ok(r > 0, `${r} filas en bloques`, 'sin filas'); });
await step('Tag Board', 'rango Today', 'con Today quedan menos filas (el correo demo no es de hoy) y All time las restaura', async () => {
  const before = await rows(); if (!(await exists('button=Today'))) return 'FAIL: no hay botón Today';
  await click('button=Today'); await sleep(1500); const today = await rows();
  await click('button=All time'); await sleep(1500); const back = await rows();
  return ok(today < before && back === before, `${before} → ${today} → ${back}`, `${before} → ${today} → ${back}`);
});
await step('Tag Board', 'buscar tag', 'Search tags… filtra los bloques', async () => {
  if (!(await exists('input[placeholder^="Search tags"]'))) return 'FAIL: no hay cuadro Search tags';
  const before = await rows(); await type('input[placeholder^="Search tags"]', 'codeberg'); await sleep(1500); const after = await rows();
  await type('input[placeholder^="Search tags"]', ''); await sleep(800);
  return ok(after <= before && after > 0, `${before} → ${after} filas`, `${before} → ${after} filas`);
});
await step('Tag Board', 'abrir hilo desde un bloque', 'clic en una fila abre el hilo en el panel central', async () => {
  const first = await js(() => document.querySelector('main div[role="button"]')?.textContent.trim().slice(0, 60));
  await click('(//main//div[@role="button"])[1]'); await sleep(1500);
  const h1s = await js(() => [...document.querySelectorAll('h1')].map(h => h.textContent.trim()).filter(t => !/^(EmailOps|Tag Board)$/.test(t)));
  const closeBtn = await b.$('main [aria-label="Close"], main button[title="Close"]');
  if (await closeBtn.isExisting()) await closeBtn.click(); else await b.keys('Escape');
  await sleep(800);
  return ok(h1s.length > 0, `hilo abierto: ${h1s[0]}`, `sin panel de lectura tras pulsar "${first}"`);
});
await step('Tag Board', 'barra de herramientas visible', 'con el panel de chat abierto, "Group by", el buscador de tags y el rango caben en el ancho disponible', async () => {
  const r = await js(() => {
    const pane = document.querySelector('h1') && [...document.querySelectorAll('h1')].find(h => h.textContent.trim() === 'Tag Board');
    const company = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Company');
    const search = document.querySelector('input[placeholder^="Search tags"]');
    if (!pane || !company || !search) return { missing: true };
    const p = pane.getBoundingClientRect(), c = company.getBoundingClientRect(), s = search.getBoundingClientRect();
    return { paneLeft: Math.round(p.left), companyLeft: Math.round(c.left), searchLeft: Math.round(s.left), width: window.innerWidth };
  });
  if (r.missing) return 'FAIL: no encuentro la barra de herramientas del Tag Board';
  const clipped = r.companyLeft < r.paneLeft - 1 || r.searchLeft < r.paneLeft - 1;
  return ok(!clipped, JSON.stringify(r), `controles recortados por la izquierda: ${JSON.stringify(r)}`);
});

// ---------- Compose ----------
// The docked chat panel also has a "Send" button; scope to the composer.
// The modal root is the fixed overlay around the subject field; `body` would also match a `:has()` query.
const composeSend = () => js(() => { const m = document.querySelector('input[placeholder^="Email subject"]')?.closest('.fixed, [role="dialog"]'); const b = m && [...m.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Send'); return b ? { disabled: b.disabled } : null; });
const clickComposeSend = () => js(() => { const m = document.querySelector('input[placeholder^="Email subject"]')?.closest('.fixed, [role="dialog"]'); const b = m && [...m.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Send'); if (!b) return false; b.click(); return true; });
// ---------- Firmas (Ajustes → Signatures, y su inserción en el compositor) ----------
const SIGNATURE = 'Ulises Demo · verification signature';
const signatureRow = () => sql("SELECT html, use_for_new FROM account_signatures WHERE account_id = 'demo-acct-work'")[0] || null;
async function openSignaturesTab() {
  await click('aria/Application settings'); await sleep(1200);
  await js(() => [...document.querySelectorAll('button')].filter((x) => x.textContent.includes('Signatures')).pop()?.click()); await sleep(1200);
}
const saveSignature = () => js(() => { const x = [...document.querySelectorAll('[data-testid="signatures-settings"] button')].find((y) => y.textContent.trim() === 'Save signature'); if (!x) return false; x.click(); return true; });
await click('button=Inbox'); await sleep(800);
await step('Firmas', 'guardar firma', 'en Ajustes → Signatures se escribe una firma para la cuenta de trabajo y «Save signature» la guarda en account_signatures', async () => {
  await openSignaturesTab();
  const editor = await js(() => { const ed = document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]'); if (!ed) return false; ed.focus(); document.execCommand('selectAll'); document.execCommand('delete'); return true; });
  if (!editor) { await closeSettings(); return 'FAIL: la pestaña Signatures no tiene editor'; }
  await js((t) => document.execCommand('insertText', false, t), SIGNATURE); await sleep(400);
  if (!(await saveSignature())) { await closeSettings(); return 'FAIL: no hay botón «Save signature»'; }
  await sleep(1500);
  const saved = await js(() => /Signature saved/.test(document.querySelector('[data-testid="signatures-settings"]')?.innerText || ''));
  const row = signatureRow();
  await closeSettings();
  return ok(saved && row?.html.includes(SIGNATURE) && row.use_for_new === 1, `«Signature saved»; BD: ${row?.html}`, `guardada=${saved}, BD=${JSON.stringify(row)}`);
});
await step('Firmas', 'firma en un mensaje nuevo', 'Compose abre con la firma de la cuenta ya insertada en el cuerpo; Cancel lo cierra sin dejar borrador', async () => {
  await click('button=Compose'); await sleep(1800);
  const sig = await js(() => document.querySelector('[contenteditable="true"] [data-emailops-signature]')?.innerText.trim() || null);
  if (await exists('button=Cancel')) { await click('button=Cancel'); await sleep(800); }
  if (await exists('input[placeholder^="Email subject"]')) { await b.keys('Escape'); await sleep(600); }
  return ok(sig === SIGNATURE, `firma insertada: «${sig}»`, `firma en el cuerpo: ${JSON.stringify(sig)}`);
});
// Feeds the hidden «Add image» input a file built in the page (WebDriver cannot pick files from a dialog).
const pickSignatureImage = (kind) => js((k) => {
  const input = document.querySelector('[data-testid="signature-image-input"]'); if (!input) return false;
  const deliver = (file) => { const dt = new DataTransfer(); dt.items.add(file); input.files = dt.files; input.dispatchEvent(new Event('change', { bubbles: true })); };
  if (k === 'svg') { deliver(new File(['<svg xmlns="http://www.w3.org/2000/svg"><script>x()</script></svg>'], 'verify-logo.svg', { type: 'image/svg+xml' })); return true; }
  const c = document.createElement('canvas'); c.width = 1600; c.height = 200; const g = c.getContext('2d'); g.fillStyle = '#2b6cb0'; g.fillRect(0, 0, 1600, 200);
  c.toBlob((blob) => deliver(new File([blob], 'verify-wide-logo.png', { type: 'image/png' })), 'image/png'); return true;
}, kind);
await step('Firmas', 'imagen no válida', '«Add image» con un SVG muestra el motivo (solo PNG, JPEG, GIF o WebP) y no toca la firma', async () => {
  await openSignaturesTab();
  if (!(await exists('[data-testid="signature-add-image"]'))) { await closeSettings(); return 'FAIL: no hay botón «Add image»'; }
  const before = await js(() => document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]')?.innerHTML || '');
  if (!(await pickSignatureImage('svg'))) { await closeSettings(); return 'FAIL: no hay selector de archivo'; }
  await sleep(1200);
  const text = await js(() => document.querySelector('[data-testid="signatures-settings"]')?.innerText || '');
  const after = await js(() => document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]')?.innerHTML || '');
  return ok(/PNG, JPEG, GIF or WebP/.test(text) && after === before, 'aviso de tipo no admitido; firma sin cambios', `aviso=${/PNG, JPEG, GIF or WebP/.test(text)}, firma cambiada=${after !== before}`);
});
await step('Firmas', 'imagen aceptada y reducida', 'un PNG de 1600 px se reduce a 600 px, entra en la firma y «Save signature» la guarda como data:image/png', async () => {
  await pickSignatureImage('png'); await sleep(2500);
  const width = await js(() => document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"] img')?.naturalWidth ?? null);
  if (!(await saveSignature())) { await closeSettings(); return 'FAIL: no hay botón «Save signature»'; }
  await sleep(1500);
  const row = signatureRow(); await closeSettings();
  return ok(width === 600 && /<img[^>]+src="data:image\/png;base64,/.test(row?.html || ''), `imagen de ${width} px guardada en account_signatures`, `ancho=${width}, BD=${(row?.html || '').slice(0, 120)}`);
});
await step('Firmas', 'borrar la firma', 'vaciar el editor y guardar deja la firma vacía (el estado de partida de la BD demo)', async () => {
  await openSignaturesTab();
  await js(() => { const ed = document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]'); ed.focus(); document.execCommand('selectAll'); document.execCommand('delete'); }); await sleep(400);
  await saveSignature(); await sleep(1500);
  const row = signatureRow();
  await closeSettings();
  return ok(row !== null && row.html === '', 'firma vacía en la BD', `BD=${JSON.stringify(row)}`);
});
await click('button=Inbox'); await sleep(1200);
await step('Compose', 'abrir', 'el modal de redacción aparece con To, Subject y cuerpo', async () => {
  await click('button=Compose'); await sleep(1500);
  const f = await js(() => [...document.querySelectorAll('input,textarea,[contenteditable=true]')].map(i => i.placeholder || i.getAttribute('aria-label') || i.tagName));
  return ok(f.some(x => /subject/i.test(x)), `campos: ${f.join(', ')}`, `campos: ${f.join(', ')}`);
});
await step('Compose', 'sin avisos de tiptap', 'abrir el editor no registra extensiones duplicadas', async () => {
  const log = fs.readFileSync(path.join(runDir, 'app.log'), 'utf8');
  const n = (log.match(/Duplicate extension names/g) || []).length;
  return ok(n === 0, 'sin avisos', `${n} aviso(s) "Duplicate extension names" en app.log`);
});
await step('Compose', 'Send deshabilitado sin destinatario', 'Send está deshabilitado hasta que hay un destinatario', async () => {
  const d = (await composeSend())?.disabled;
  return ok(d === true, 'Send deshabilitado', `Send habilitado sin destinatario (disabled=${d})`);
});
await step('Compose', 'rellenar', 'To, Subject y cuerpo aceptan texto y Send se habilita', async () => {
  await type('input[placeholder^="Add recipients"]', 'someone@example.com'); await sleep(400);
  // Commit the chip with a keydown on the field itself: within one WebDriver session the
  // server's `keys` does not always reach the element that was just typed into.
  await js(() => { const i = document.querySelector('input[placeholder^="Add recipients"]'); i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true })); }); await sleep(500);
  await type('input[placeholder^="Email subject"]', 'Verification run');
  // A contenteditable (tiptap/ProseMirror) ignores a value written straight into the DOM; execCommand produces the
  // DOM mutations + input events the editor listens to, like real typing does.
  const body = await b.$('[contenteditable="true"]'); await body.click(); await sleep(200);
  await js(() => document.execCommand('insertText', false, 'hello from the verifier')); await sleep(800);
  const v = await js(() => { const box = document.querySelector('input[placeholder^="Email subject"]')?.closest('.fixed, [role="dialog"]'); return { subject: document.querySelector('input[placeholder^="Email subject"]')?.value, body: document.querySelector('[contenteditable="true"]')?.innerText, chip: !!box && box.innerText.includes('someone@example.com') }; });
  v.sendDisabled = (await composeSend())?.disabled;
  return ok(v.subject === 'Verification run' && /hello from the verifier/.test(v.body || '') && v.sendDisabled === false, JSON.stringify(v), JSON.stringify(v));
});
// ---------- Envío: deshacer el envío y envío programado (outbox local) ----------
// Send no longer sends straight away: the message waits in the outbox for the undo window (10 s by default).
const outboxRows = (subject) => sql(`SELECT id, origin, status, send_at, created_at, last_error FROM outbox WHERE subject = ${ids([subject])} ORDER BY created_at DESC, rowid DESC`);
const subjectValue = () => js(() => document.querySelector('input[placeholder^="Email subject"]')?.value ?? null);
async function fillCompose(subject, body) {
  await click('button=Compose'); await sleep(1500);
  await type('input[placeholder^="Add recipients"]', 'someone@example.com'); await sleep(400);
  await js(() => { const i = document.querySelector('input[placeholder^="Add recipients"]'); i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true })); }); await sleep(500);
  await type('input[placeholder^="Email subject"]', subject);
  const ed = await b.$('[contenteditable="true"]'); await ed.click(); await sleep(200);
  await js((t) => document.execCommand('insertText', false, t), body); await sleep(800);
}
const sendButton = (suffix = '') => js((s) => { const x = [...document.querySelectorAll('[data-testid]')].find((y) => /^(compose|compose-tab|reply)-send$/.test(y.getAttribute('data-testid'))); if (!x) return null; const el = s ? document.querySelector(`[data-testid="${x.getAttribute('data-testid')}${s}"]`) : x; if (!el) return null; el.click(); return x.getAttribute('data-testid'); }, suffix);
await step('Envío', 'enviar y deshacer', 'Send cierra el compositor y deja el mensaje en el outbox con un aviso «Sending… · Undo»; Undo lo retira (cancelled) y reabre el compositor con el asunto y el texto', async () => {
  const subject = 'Verification run';
  if (!(await sendButton())) return 'FAIL: no hay botón Send en el compositor';
  const t = await waitToast(/Sending/, 5000);
  const closed = (await subjectValue()) === null;
  const queued = outboxRows(subject)[0];
  const window = queued ? queued.send_at - queued.created_at : null;
  if (!(await clickToastAction('Undo'))) return `FAIL: sin Undo en el aviso (aviso «${t}», en cola=${JSON.stringify(queued)})`;
  await sleep(2000);
  const after = outboxRows(subject)[0];
  const reopened = await subjectValue(); const text = await js(() => document.querySelector('[contenteditable="true"]')?.innerText || '');
  return ok(!!t && closed && queued?.origin === 'undo' && window > 0 && after?.id === queued.id && after.status === 'cancelled' && reopened === subject && /hello from the verifier/.test(text),
    `aviso «${t}»; en cola ${window} s; Undo → cancelled y compositor reabierto`, `aviso=${t}, cerrado=${closed}, cola=${JSON.stringify(queued)}, después=${JSON.stringify(after)}, asunto=${reopened}`);
});
await step('Envío', 'programar envío', 'el desplegable de Send ofrece «Schedule send» con momentos y la nota de que la app debe estar abierta; elegir uno cierra el compositor y guarda el mensaje como programado a esa hora', async () => {
  const subject = 'Verification run';
  const id = await sendButton('-schedule'); if (!id) return 'FAIL: el compositor no tiene el desplegable de Send';
  await sleep(600);
  const menu = await js((i) => document.querySelector(`[data-testid="${i}-schedule-menu"]`)?.innerText.replace(/\n/g, ' | ') || '', id);
  const preset = await js(() => { const x = document.querySelector('[data-testid*="-send-preset-"]'); if (!x) return null; const label = x.innerText.replace(/\n/g, ' '); x.click(); return label; });
  if (!preset) return `FAIL: sin momentos en el menú: ${menu}`;
  const t = await waitToast(/Scheduled for/, 5000); await sleep(500);
  const row = outboxRows(subject)[0];
  return ok(/Schedule send/.test(menu) && /must be open/.test(menu) && !!t && (await subjectValue()) === null && row?.origin === 'scheduled' && row.status === 'scheduled' && row.send_at > Date.now() / 1000,
    `«${preset}»; aviso «${t}»; outbox scheduled`, `menú «${menu.slice(0, 120)}», aviso=${t}, fila=${JSON.stringify(row)}`);
});
await step('Envío', 'vista Scheduled y borrar', 'Scheduled lista el mensaje con su hora; Delete lo quita con un aviso con Undo y la fila del outbox queda cancelled', async () => {
  await closeToasts(); await click('[data-testid="sidebar-scheduled"]'); await sleep(1500);
  const rows = await js(() => [...document.querySelectorAll('[data-testid="scheduled-row"]')].map((r) => r.innerText.replace(/\n/g, ' | ')));
  const mine = rows.find((r) => r.includes('Verification run'));
  if (!mine) return `FAIL: Scheduled no lista el mensaje: ${rows.join(' // ') || 'vacía'}`;
  await js(() => { const r = [...document.querySelectorAll('[data-testid="scheduled-row"]')].find((x) => x.innerText.includes('Verification run')); r.querySelector('[data-testid="scheduled-delete"]').click(); }); await sleep(1500);
  const t = await toastText(); const left = await js(() => [...document.querySelectorAll('[data-testid="scheduled-row"]')].some((r) => r.innerText.includes('Verification run')));
  const status = outboxRows('Verification run')[0]?.status;
  await closeToasts();
  return ok(/Sends/.test(mine) && /Scheduled message deleted/.test(t) && /Undo/.test(t) && !left && status === 'cancelled', `«${mine.slice(0, 80)}»; aviso «${t}»; BD cancelled`, `fila «${mine}», aviso «${t}», sigue=${left}, BD=${status}`);
});
await click('button=Inbox'); await sleep(1000);
await step('Compose', 'enviar sin credenciales', 'Send en la cuenta demo sin credenciales: al acabar la ventana de deshacer el envío falla con un aviso visible, el outbox queda failed con el error y Scheduled lo muestra «Not sent»; Delete lo retira', async () => {
  const subject = 'Verification run (no credentials)';
  await fillCompose(subject, 'hello from the verifier');
  if (!(await sendButton())) return 'FAIL: no hay botón Send en el compositor';
  const err = await waitToast(/could not be sent/i, 30000);
  const row = outboxRows(subject)[0];
  await closeToasts(); await click('[data-testid="sidebar-scheduled"]'); await sleep(1500);
  const shown = await js((s) => [...document.querySelectorAll('[data-testid="scheduled-row"]')].find((r) => r.innerText.includes(s))?.innerText.replace(/\n/g, ' | ') || null, subject);
  await js((s) => [...document.querySelectorAll('[data-testid="scheduled-row"]')].find((r) => r.innerText.includes(s))?.querySelector('[data-testid="scheduled-delete"]')?.click(), subject); await sleep(1200);
  const cleaned = outboxRows(subject)[0]?.status;
  await closeToasts(); await click('button=Inbox'); await sleep(800);
  return ok(!!err && row?.status === 'failed' && /Authentication required/.test(row.last_error || '') && /Not sent/.test(shown || '') && cleaned === 'cancelled',
    `aviso: ${err?.slice(0, 110)}; outbox failed; Scheduled «Not sent»; borrado`, `aviso=${err}, fila=${JSON.stringify(row)}, Scheduled=${shown}, tras borrar=${cleaned}`);
});
await step('Compose', 'cerrar y borrador', 'al cancelar, el borrador aparece en Drafts (los borradores se guardan automáticamente)', async () => {
  await fillCompose('Verification run', 'hello from the verifier');
  if (await exists('button=Cancel')) { await click('button=Cancel'); await sleep(800); for (const c of ['button=Discard', 'button=Keep', 'button=Save draft']) if (await exists(c)) { await click(c === 'button=Discard' ? 'button=Keep' : c).catch(() => {}); break; } }
  if (await exists('[role="dialog"]')) { await b.keys('Escape'); await sleep(800); }
  await click('button=Drafts'); await sleep(1500);
  const t = await bodyText();
  return ok(t.includes('Verification run'), 'borrador listado', 'el borrador "Verification run" no aparece en Drafts');
});
await step('Compose', 'descartar borrador', 'Continue editing + Discard elimina el borrador', async () => {
  if (!(await bodyText()).includes('Verification run')) return 'SKIP: no hay borrador que descartar (ver paso anterior)';
  // Delete every draft this run created, through the row's own "Delete draft" button.
  for (let i = 0; i < 30; i++) {
    const clicked = await js(() => { const row = [...document.querySelectorAll('[data-testid="draft-row"]')].find((r) => /Verification run|hello from the verifier/.test(r.textContent)); const btn = row?.querySelector('button[title="Delete draft"]'); if (btn) { btn.click(); return true; } return false; });
    if (!clicked) break; await sleep(1200);
    for (const c of ['button=Delete', 'button=Discard', 'button=OK', 'button=Confirm', 'button=Yes']) if (await exists(c)) { await click(c); await sleep(800); break; }
  }
  await click('button=Inbox'); await sleep(600); await click('button=Drafts'); await sleep(1200);
  return ok(!/Verification run|hello from the verifier/.test(await bodyText()), 'borradores del run eliminados', 'el borrador sigue en Drafts');
});
await step('Compose', 'borrador con tabla', 'abrir un borrador con una tabla la conserva en el editor; editar una celda guarda el borrador con la tabla intacta', async () => {
  const draftHtml = () => sql("SELECT body_html FROM drafts WHERE subject = 'Milestone dates (table)'")[0]?.body_html ?? null;
  const opened = await js(() => { const row = [...document.querySelectorAll('[data-testid="draft-row"]')].find((r) => /Milestone dates \(table\)/.test(r.textContent)); const b = row && [...row.querySelectorAll('button')].find((x) => (x.title || x.textContent.trim()) === 'Continue editing'); if (!b) return false; b.click(); return true; });
  if (!opened) return 'FAIL: no hay borrador «Milestone dates (table)» en Drafts (¿BD demo sin las fixtures de verificación?)';
  await sleep(1500);
  const table = () => js(() => { const t = document.querySelector('[contenteditable="true"] table'); return t ? { rows: t.querySelectorAll('tr').length, cells: [...t.querySelectorAll('th,td')].map((c) => c.textContent) } : null; });
  const waitSaved = async (want) => { for (let i = 0; i < 20; i++) { const h = draftHtml() || ''; if (h.includes('(tbc)') === want) return h; await sleep(500); } return draftHtml() || ''; };
  const closeTab = async () => { await js(() => [...document.querySelectorAll('button')].find((x) => x.getAttribute('aria-label') === 'Close tab')?.click()); await sleep(800); };
  const before = await table();
  if (!before || before.rows !== 3 || before.cells.join('|') !== 'Milestone|Date|M6|14 March|M7|28 March') { await closeTab(); return `FAIL: el editor no muestra la tabla del borrador: ${JSON.stringify(before)}`; }
  // Caret at the end of a cell, then typed text: the DOM mutations the editor listens to.
  await js(() => { const ed = document.querySelector('[contenteditable="true"]'); const cell = [...ed.querySelectorAll('td')].find((c) => c.textContent === '14 March'); ed.focus(); const r = document.createRange(); r.selectNodeContents(cell.querySelector('p') || cell); r.collapse(false); const sel = getSelection(); sel.removeAllRanges(); sel.addRange(r); document.execCommand('insertText', false, ' (tbc)'); });
  const edited = await table();
  const saved = await waitSaved(true);
  // Put the draft back as it was, so the next run starts from the same text.
  await js(() => { for (let i = 0; i < ' (tbc)'.length; i++) document.execCommand('delete'); });
  const restored = await waitSaved(false);
  await closeTab();
  const kept = edited?.rows === 3 && edited.cells[3] === '14 March (tbc)';
  const savedOk = saved.includes('14 March (tbc)') && (saved.match(/<tr>/g) || []).length === 3 && saved.includes('<th');
  return ok(kept && savedOk && !restored.includes('(tbc)') && restored.includes('<table>'), 'tabla de 3 filas en el editor y en el borrador guardado tras editar una celda',
    `editor=${JSON.stringify(edited)}, guardado con tabla=${savedOk}, restaurado=${!restored.includes('(tbc)')}`);
});

// ---------- Settings ----------
await step('Ajustes', 'abrir', 'el diálogo de ajustes abre con sus pestañas', async () => {
  await click('aria/Application settings'); await sleep(1500);
  const t = await bodyText(); const tabs = ['Appearance', 'AI Backend & Models', 'AI Classification', 'Privacy & Security', 'Junk'].filter(x => t.includes(x));
  return ok(tabs.length >= 4, `pestañas: ${tabs.join(', ')}`, `pestañas visibles: ${tabs.join(', ')}`);
});
for (const tab of ['AI Backend', 'AI Classification', 'AI Search', 'AI Drafts', 'AI Translation', 'Privacy', 'Junk', 'Signatures', 'Notifications', 'Calendar', 'Appearance']) {
  await step('Ajustes', `pestaña ${tab}`, `la pestaña ${tab} renderiza contenido`, async () => {
    // The sidebar has a "Calendar" view button behind the modal that `button*=` would hit first. The settings
    // dialog is portalled after the sidebar, so the last matching button in DOM order is the tab.
    const hit = await js((label) => { const b = [...document.querySelectorAll('button')].filter((x) => x.textContent.includes(label)).pop(); if (!b) return false; b.click(); return true; }, tab);
    if (!hit) return `FAIL: no hay pestaña ${tab}`;
    await sleep(1200);
    const t = (await js(() => { const d = [...document.querySelectorAll('[role="dialog"]')].pop(); return d ? d.innerText : document.body.innerText; })).replace(/\s+/g, ' ');
    return ok(t.length > 100, t.slice(0, 100), 'contenido vacío');
  });
}
// ---------- Notificaciones (Ajustes → Notifications) ----------
await step('Notificaciones', 'los ajustes persisten', 'cambiar «Only when EmailOps is not focused», una cuenta y el contenido a «Hide content» se guarda en user_preferences y sobrevive a cerrar y reabrir Ajustes; después se dejan como estaban', async () => {
  const UNF = 'Only when EmailOps is not focused', ACC = 'Notify for ulises@fastmail.com';
  const tab = async () => { await js(() => [...document.querySelectorAll('button')].filter((x) => x.textContent.includes('Notifications')).pop()?.click()); await sleep(1200); };
  const sw = (l, act) => js((label, a) => { const x = [...document.querySelectorAll('button')].find((y) => y.getAttribute('aria-label') === label && y.offsetParent); if (!x) return null; if (a) x.click(); return x.getAttribute('aria-checked'); }, l, act);
  const radio = (v, act) => js((val, a) => { const r = [...document.querySelectorAll('input[name="notification-content"]')].find((x) => x.value === val); if (!r) return null; if (a) r.click(); return r.checked; }, v, act);
  const prefs = () => Object.fromEntries(sql("SELECT key, value FROM user_preferences WHERE key LIKE 'notifications.new_mail.%'").map((r) => [r.key, r.value]));
  await tab();
  const start = { unf: await sw(UNF), acc: await sw(ACC), hidden: await radio('hidden') };
  if (start.unf === null || start.acc === null || start.hidden === null) return `FAIL: faltan controles en Notifications: ${JSON.stringify(start)}`;
  await sw(UNF, true); await sleep(600); await sw(ACC, true); await sleep(600); await radio(start.hidden ? 'preview' : 'hidden', true); await sleep(800);
  const saved = prefs();
  await closeSettings(); await click('aria/Application settings'); await sleep(1200); await tab();
  const reopened = { unf: await sw(UNF), acc: await sw(ACC), hidden: await radio('hidden') };
  await sw(UNF, true); await sleep(600); await sw(ACC, true); await sleep(600); await radio(start.hidden ? 'hidden' : 'preview', true); await sleep(800);
  const restored = prefs();
  const flipped = reopened.unf !== start.unf && reopened.acc !== start.acc && reopened.hidden !== start.hidden;
  const inDb = saved['notifications.new_mail.only_unfocused'] === String(start.unf !== 'true') && saved['notifications.new_mail.account:demo-acct-personal'] === String(start.acc !== 'true') && saved['notifications.new_mail.content'] === (start.hidden ? 'preview' : 'hidden');
  const back = restored['notifications.new_mail.only_unfocused'] === String(start.unf === 'true') && restored['notifications.new_mail.content'] === (start.hidden ? 'hidden' : 'preview');
  return ok(flipped && inDb && back, `guardado ${JSON.stringify(saved)}; persiste al reabrir; restaurado`, `inicio=${JSON.stringify(start)}, reabierto=${JSON.stringify(reopened)}, BD=${JSON.stringify(saved)}, restaurado=${JSON.stringify(restored)}`);
});
// ---------- IA: proveedores y modelos (dentro de Ajustes → AI Backend) ----------
// Elegir una pestaña de proveedor solo cambia el formulario; nada se guarda sin «Save», que ningún paso pulsa.
const settingsBox = () => js(() => { const d = document.querySelector('button[title="Close settings"]')?.closest('.fixed'); if (!d) return null;
  return { text: d.innerText, fields: [...d.querySelectorAll('input,select')].map((i) => ({ tag: i.tagName, type: i.type, label: i.getAttribute('aria-label'), placeholder: i.placeholder, value: i.value, options: i.tagName === 'SELECT' ? [...i.options].map((o) => o.textContent) : null })) }; });
const providerTab = (label) => js((l) => { const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim().startsWith(l) && x.querySelector('div')); if (!b) return false; b.click(); return true; }, label);
await step('IA', 'pestaña OpenRouter', 'OpenRouter ofrece el modelo de chat ya relleno con el predeterminado, el selector de Embeddings con «None» y los recomendados primero, y el presupuesto de contexto; no ofrece «Keep model loaded»', async () => {
  await js(() => [...document.querySelectorAll('button')].filter((x) => x.textContent.includes('AI Backend')).pop()?.click()); await sleep(1200);
  // With a key the tab asks OpenRouter for its model list. `verify.sh launch` starts the instance without one; refuse to go on if it has one anyway.
  const hasKey = await js(async () => (await window.__TAURI_INTERNALS__.invoke('get_ai_config')).hasApiKey);
  if (hasKey) return 'FAIL: la instancia tiene una clave de OpenRouter; no se abre la pestaña para no llamar al proveedor remoto';
  if (!(await providerTab('OpenRouter'))) return 'FAIL: no hay pestaña OpenRouter en AI Backend';
  await sleep(1200);
  const box = await settingsBox(); if (!box) return 'FAIL: el diálogo de Ajustes no está abierto';
  const chat = box.fields.find((f) => f.type === 'text' && f.placeholder.startsWith('e.g.'));
  const embed = box.fields.find((f) => f.label === 'Embedding Model');
  const budget = box.fields.find((f) => f.label === 'Context budget (tokens)');
  const recommended = (embed?.options || []).slice(1).filter((o) => /— recommended/.test(o)).length;
  const firstOther = (embed?.options || []).slice(1).findIndex((o) => !/— recommended/.test(o));
  const problems = [];
  if (!chat || !/^[a-z0-9.-]+\/[a-z0-9.:-]+$/i.test(chat.value)) problems.push(`modelo de chat sin predeterminado (${chat?.value})`);
  if (!embed || embed.options[0] !== 'None — keyword search only') problems.push(`primera opción de Embeddings: ${embed?.options?.[0]}`);
  if (recommended < 2 || (firstOther !== -1 && firstOther < recommended)) problems.push(`recomendados=${recommended}, primera no recomendada en ${firstOther}`);
  if (embed && embed.value !== '') problems.push(`Embeddings preseleccionado: ${embed.value}`);
  if (!budget || !(Number(budget.value) > 0)) problems.push(`presupuesto de contexto: ${budget?.value}`);
  if (/Keep model loaded/.test(box.text)) problems.push('«Keep model loaded» visible en OpenRouter');
  return ok(!problems.length, `chat ${chat?.value}; Embeddings: None + ${recommended} recomendados; presupuesto ${budget?.value}; sin Keep model loaded`, problems.join('; '));
});
await step('IA', 'pestaña In-app', 'al volver a In-app aparece «Keep model loaded» y desaparece el presupuesto de contexto; el proveedor guardado no ha cambiado', async () => {
  if (!(await providerTab('In-app'))) return 'FAIL: no hay pestaña In-app en AI Backend';
  await sleep(1200);
  const box = await settingsBox(); if (!box) return 'FAIL: el diálogo de Ajustes no está abierto';
  const saved = sql("SELECT value FROM user_preferences WHERE key = 'ai_provider'")[0]?.value;
  const keep = /Keep model loaded \(minutes\)/.test(box.text), budget = /Context budget/.test(box.text);
  return ok(keep && !budget && saved === 'llamacpp', `Keep model loaded visible; proveedor guardado: ${saved}`, `keepAlive=${keep}, presupuesto=${budget}, proveedor guardado=${saved}`);
});
await step('Ajustes', 'Escape cierra el diálogo', 'como en el resto de modales, Escape cierra Ajustes', async () => {
  await b.keys('Escape'); await sleep(800);
  return ok(!(await exists('aria/Close settings')), 'cerrado con Escape', 'Escape no cierra el diálogo de Ajustes (el resto de modales sí)');
});
await step('Ajustes', 'cerrar', 'el botón Close settings cierra el diálogo', async () => {
  if (await exists('aria/Close settings')) await click('aria/Close settings');
  await sleep(1000); return ok(!(await exists('aria/Close settings')), 'cerrado', 'el diálogo sigue abierto');
});

await step('IA', 'barra de Logs', 'la barra de estado muestra el backend como texto y solo ofrece cambiar el modelo: el backend se cambia en Ajustes', async () => {
  const bar = await js(() => { const label = document.querySelector('[data-testid="ai-backend"]'); if (!label) return null; const box = label.parentElement;
    return { backend: label.textContent.trim(), tag: label.tagName, selects: [...box.querySelectorAll('select')].map((x) => ({ label: x.getAttribute('aria-label'), options: [...x.options].map((o) => o.value) })) }; });
  if (!bar) return 'FAIL: la barra de Logs no muestra el backend (data-testid="ai-backend")';
  const providerNames = /^(llamacpp|ollama|openrouter|Embedded|Ollama|OpenRouter)$/;
  const backendSelect = bar.selects.some((x) => x.options.some((o) => providerNames.test(o)));
  const modelOnly = bar.selects.length === 1 && bar.selects[0].label === 'AI Model';
  return ok(bar.backend === 'Embedded' && bar.tag === 'SPAN' && !backendSelect && modelOnly, `backend «${bar.backend}» como texto; selector: ${bar.selects.map((x) => x.label).join(', ')}`, JSON.stringify(bar));
});

// ---------- Skills (experimental, apagado por defecto) ----------
// Enciende el interruptor si hacía falta, comprueba la entrada y la vista, y lo
// deja como estaba: la BD demo es compartida con el resto de capas.
const skillsSwitch = (action) => js((act) => {
  const sw = [...document.querySelectorAll('[role=switch]')].filter((e) => e.offsetParent).find((e) => {
    let r = e.parentElement; for (let i = 0; i < 4 && r && !/Enable skills/.test(r.innerText); i++) r = r.parentElement;
    return /Enable skills/.test(r?.innerText || '');
  });
  if (!sw) return null;
  if (act === 'click') sw.click();
  return sw.getAttribute('aria-checked');
}, action);
const openSkillsTab = async () => {
  await click('aria/Application settings'); await sleep(1200);
  await js(() => { const t = [...document.querySelectorAll('button')].filter((x) => x.textContent.includes('AI Skills')).pop(); t?.click(); });
  await sleep(1000);
};
let skillsWasOn = null;
await step('Skills', 'activar', 'con «Enable skills» encendido aparece Skills en la barra lateral', async () => {
  await openSkillsTab();
  skillsWasOn = (await skillsSwitch('read')) === 'true';
  if (skillsWasOn === null) return 'FAIL: no hay interruptor «Enable skills» en Ajustes → AI Skills';
  if (!skillsWasOn) await skillsSwitch('click');
  await sleep(800); await click('aria/Close settings'); await sleep(1000);
  return ok(await exists('[data-testid="sidebar-skills"]'), 'entrada Skills visible', 'no aparece Skills en la barra lateral');
});
await step('Skills', 'vista', 'la vista Skills abre con la lista y el botón New', async () => {
  if (!(await exists('[data-testid="sidebar-skills"]'))) return 'FAIL: no hay entrada Skills';
  await click('[data-testid="sidebar-skills"]'); await sleep(1500);
  return ok(await exists('[data-testid="skill-new"]'), 'lista y New visibles', 'la vista no muestra New');
});
if (skillsWasOn === false) { await openSkillsTab(); await skillsSwitch('click'); await sleep(800); await click('aria/Close settings'); await sleep(800); }

// ---------- Chat (último: carga el modelo) ----------
await click('button=Inbox'); await sleep(1000);
await step('Chat', 'panel visible', 'el panel de chat está acoplado con su cuadro de texto y Send', async () => {
  if (!(await exists('button=Send'))) { if (await exists('button=Chat')) { await click('button=Chat'); await sleep(1200); } }
  return ok(await exists('textarea[placeholder^="Ask about your emails"]') && await exists('button=Send'), 'panel listo', 'sin panel de chat');
});
await step('Chat', 'pregunta y respuesta', 'una pregunta recibe respuesta con fuentes en menos de 120 s', async () => {
  const before = await bodyText();
  await type('textarea[placeholder^="Ask about your emails"]', 'Which clients are asking about Ollama?'); await click('button=Send');
  let t = '', got = false; const t0 = Date.now();
  // Done when the bubble shows its sources footer ("1 source used" / "N sources used" / "Sources") or its token count, and nothing is still streaming.
  while (Date.now() - t0 < 120000) { await sleep(5000); t = await bodyText(); if ((/sources? used|Sources|tokens/i.test(t.replace(before, ''))) && !/Waiting for reply/.test(t)) { got = true; break; } }
  const secs = Math.round((Date.now() - t0) / 1000);
  const answer = t.split('\n').filter(l => /Ollama|Kwame/.test(l) && !before.includes(l)).slice(0, 3).join(' / ');
  return ok(got, `${secs} s; ${answer || 'respuesta sin mención explícita a Ollama'}`, `sin respuesta tras ${secs} s`);
});
await step('Chat', 'nuevo chat', 'New chat vacía la conversación', async () => { await click('aria/New chat'); await sleep(1200); return ok(!/sources? used|Sources/i.test(await bodyText()), 'conversación nueva', 'la respuesta anterior sigue visible'); });
await step('Chat', 'cerrar y reabrir', 'Close chat panel oculta el panel y "Open chat panel" en la cabecera del inbox lo vuelve a acoplar', async () => {
  if (!(await exists('aria/Close chat panel'))) { await click('button=Inbox'); await sleep(1000); }
  // The docked state is a persisted pref (`chat_panel_open`), so a previous run can leave it closed: dock it first.
  if (!(await exists('aria/Close chat panel')) && (await exists('aria/Open chat panel'))) { await click('aria/Open chat panel'); await sleep(1200); }
  if (!(await exists('aria/Close chat panel'))) return 'FAIL: el panel acoplado no está visible desde el inbox ni se puede acoplar desde la cabecera';
  await click('aria/Close chat panel'); await sleep(1000); const closed = !(await exists('button=Send'));
  if (!(await exists('aria/Open chat panel'))) return `FAIL: sin botón "Open chat panel" tras cerrar (cerrado=${closed})`;
  await click('aria/Open chat panel'); await sleep(1200);
  return ok(closed && await exists('button=Send'), 'cerrado y reabierto desde la cabecera', closed ? 'no se reabrió' : 'no se cerró');
});

// ---------- Chat → formularios (después del chat: reusa el modelo ya cargado) ----------
// Cubre las tres piezas nuevas de una sola pasada por la UI real: el veredicto `form` del
// query planner, el efecto FillForm que abre "Crear Lens" ya relleno, y el contrato de que
// el panel de chat sigue visible Y utilizable mientras el formulario está abierto.
await step('Chat/Formularios', 'rellenar Crear Lens desde el chat', 'pedir una lens abre el formulario con nombre y columnas ya puestos en menos de 120 s', async () => {
  if (!(await exists('button=Send'))) { if (await exists('aria/Open chat panel')) { await click('aria/Open chat panel'); await sleep(1200); } }
  if (!(await exists('button=Send'))) return 'FAIL: no hay panel de chat desde el que pedirlo';
  await type('textarea[placeholder^="Ask about your emails"]', 'crea una lens para seguir las facturas de mis proveedores con el importe, la fecha y el proveedor');
  await click('button=Send');
  const t0 = Date.now(); let opened = false;
  while (Date.now() - t0 < 120000) {
    await sleep(5000);
    if (await exists('[data-testid="lens-create-name"]')) { opened = true; break; }
  }
  const secs = Math.round((Date.now() - t0) / 1000);
  if (!opened) return `FAIL: el formulario no se abrió tras ${secs} s`;
  const name = await js(() => document.querySelector('[data-testid="lens-create-name"]')?.value || '');
  const cols = await js(() => document.querySelectorAll('[data-testid="lens-create-column-key"]').length);
  const filledCols = await js(() => [...document.querySelectorAll('[data-testid="lens-create-column-key"]')].filter(i => i.value.trim()).length);
  return ok(name.trim().length > 0 && filledCols >= 2,
    `${secs} s; nombre "${name}", ${filledCols}/${cols} columnas con clave`,
    `nombre "${name}", ${filledCols}/${cols} columnas con clave`);
});
await step('Chat/Formularios', 'el chat sigue visible y usable', 'con el formulario abierto el panel de chat se ve y acepta otro mensaje', async () => {
  if (!(await exists('[data-testid="lens-create-name"]'))) return 'SKIP: el formulario no está abierto';
  // Visible: el diálogo no bloqueante se aparta del dock en vez de taparlo.
  const sendVisible = await js(() => {
    const btns = [...document.querySelectorAll('button')].filter(x => x.textContent.trim() === 'Send');
    if (!btns.length) return false;
    const r = btns[0].getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  });
  if (!sendVisible) return 'FAIL: el botón Send del chat no está visible con el formulario abierto';
  // Usable: sin trampa de puntero — lo que hay bajo el cursor en el cuadro de texto ES el cuadro de texto.
  const reachable = await js(() => {
    const ta = document.querySelector('textarea[placeholder^="Ask about your emails"]');
    if (!ta) return false;
    const r = ta.getBoundingClientRect();
    const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return !!top && (top === ta || ta.contains(top));
  });
  return ok(reachable, 'panel visible y el cuadro de texto recibe el puntero', 'algo tapa el cuadro de texto del chat');
});
await step('Chat/Formularios', 'cerrar sin guardar', 'Cancel cierra el formulario sin crear la lens', async () => {
  if (!(await exists('[data-testid="lens-create-name"]'))) return 'SKIP: el formulario no está abierto';
  await click('button=Cancel'); await sleep(1000);
  return ok(!(await exists('[data-testid="lens-create-name"]')), 'formulario cerrado', 'el formulario sigue abierto');
});

// ---------- Chat → respuesta incorrecta ----------
await step('Chat', 'marcar respuesta incorrecta', 'el botón pide el motivo antes de reintentar y no reintenta en vacío', async () => {
  if (!(await exists('[data-testid="chat-mark-wrong"]'))) return 'SKIP: no hay respuesta terminada que marcar';
  await click('[data-testid="chat-mark-wrong"]'); await sleep(600);
  if (!(await exists('[data-testid="chat-wrong-reason"]'))) return 'FAIL: no pide el motivo';
  const disabled = await js(() => document.querySelector('[data-testid="chat-wrong-submit"]')?.disabled);
  return ok(disabled === true, 'pide el motivo y el botón está inhabilitado en vacío', `botón habilitado sin motivo (disabled=${disabled})`);
});
await step('Chat', 'reintento correctivo', 'con un motivo escrito, reintentar marca la respuesta y lanza un turno nuevo', async () => {
  if (!(await exists('[data-testid="chat-wrong-reason"]'))) return 'SKIP: el cuadro de motivo no está abierto';
  await type('[data-testid="chat-wrong-reason"]', 'faltan facturas de este mes');
  await click('[data-testid="chat-wrong-submit"]'); await sleep(3000);
  const marked = await exists('[data-testid="chat-rejected-note"]');
  const asked = (await bodyText()).includes('faltan facturas de este mes');
  return ok(marked && asked, 'respuesta marcada y motivo enviado como turno nuevo',
    `marcada=${marked}, motivo en la conversación=${asked}`);
});

fs.writeFileSync(path.join(out, 'results.json'), JSON.stringify(results, null, 2));
const fails = results.filter(r => r.status === 'fail').length;
console.log(`\n${results.length} pasos, ${fails} fallos → ${out}`);
await b.deleteSession().catch(() => {});
