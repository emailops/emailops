// One-session UI sweep of EmailOps through the embedded WebDriver.
// Usage: node sweep.mjs <run_dir>   (env TAURI_WEBDRIVER_PORT, default 4445)
// Writes <run_dir>/sweep/*.png and <run_dir>/sweep/results.json
import { remote } from 'webdriverio';
import fs from 'node:fs';
import path from 'node:path';

const runDir = process.argv[2];
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

// A claim the published docs make about the UI. `id` matches a
// `<!-- claim:id -->` marker in docs/site/<lang>/<page>; check-docs-claims.sh
// fails if either side loses the other, so a claim cannot be quietly orphaned
// by rewriting the paragraph it guards. `fix` is what the report shows when the
// app and the page disagree: the page is as likely to be the wrong one, and a
// bare assertion failure does not say which.
// Recorded as a normal sweep step under a `doc:` name so it rides the existing
// session, screenshots and results.json — verify_all.py reads that prefix to
// file it under the "doc" test type.
async function docClaim(id, feature, page, expect, fix, fn) {
  await step(feature, `doc:${id}`, `${page}: ${expect}`, fn);
  const rec = results[results.length - 1];
  rec.claim = id; rec.page = page;
  if (rec.status === 'fail') rec.fix = fix;
}

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
await docClaim('reading-pane-forward', 'Inbox', 'features.md',
  'Forward sits next to Reply and Reply all in the reading pane',
  'si Forward ya no está junto a Reply/Reply all, corregir el párrafo "Forwarding" en los 4 idiomas',
  async () => {
    const promised = ['Forward', 'Reply', 'Reply All'];
    const missing = [];
    for (const label of promised) if (!(await exists(`button=${label}`))) missing.push(label);
    return ok(!missing.length, promised.join(', '), `el doc promete ${missing.join(', ')} en el panel de lectura`);
  });
await step('Inbox', 'volver con Back', 'Back devuelve a la lista', async () => { await click('button=Back'); await sleep(1200); return ok(await exists('h2*=Inbox') && !(await exists('h1*=How do I add')), 'lista visible', 'el hilo sigue abierto'); });
await step('Inbox', 'menú ⋮ de una fila', 'el menú de acciones de la fila abre con opciones', async () => {
  const btn = await b.$('aria/More actions'); if (!(await btn.isExisting())) return 'FAIL: no hay botón "More actions"';
  await js(el => el.click(), btn); await sleep(800);
  const t = await bodyText(); const items = ['Open in new tab', 'Chat about this thread', 'Block sender'].filter(i => t.includes(i));
  await b.keys('Escape'); await sleep(400);
  return ok(items.length >= 2, `opciones: ${items.join(', ')}`, `opciones visibles: ${items.join(', ') || 'ninguna'}`);
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
const optional = { Calendar: 'el calendario solo se activa en cuentas Gmail/Outlook', Lenses: 'AI Lenses es experimental y está desactivado' };
for (const view of ['Tag Board', 'Attachments', 'Drafts', 'Sent', 'Calendar', 'Spam', 'Deleted', 'Contacts', 'Dashboard', 'Tasks', 'Lenses', 'Memory']) {
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
await docClaim('tag-board-toolbar', 'Tag Board y clasificación', 'ai-features.md',
  'the toolbar narrows the board by time (Today, Yesterday, Last 7 days) and carries the same Hide junk messages switch as the inbox',
  'si un control ya no existe, reescribir el párrafo de la barra de herramientas en los 4 idiomas citando las etiquetas reales de src/locales/<lang>/',
  async () => {
    const promised = ['Today', 'Yesterday', 'Last 7 days', 'Hide junk messages'];
    const missing = [];
    for (const label of promised) if (!(await exists(`button=${label}`))) missing.push(label);
    return ok(!missing.length, promised.join(', '), `el doc promete ${missing.join(', ')} en la barra`);
  });
await docClaim('tag-board-dimensions', 'Tag Board y clasificación', 'ai-features.md',
  'pick one dimension — Company, Priority, Intent or Topic',
  'si el juego de dimensiones cambió, actualizar la lista del párrafo del Tag Board en los 4 idiomas',
  async () => {
    const t = await bodyText();
    const promised = ['Company', 'Priority', 'Intent', 'Topic'];
    const missing = promised.filter(d => !t.includes(d));
    return ok(!missing.length, promised.join(', '), `el doc promete las dimensiones ${missing.join(', ')} y no aparecen`);
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
await step('Compose', 'enviar sin credenciales', 'Send falla con un aviso visible, no en silencio', async () => {
  if (!(await clickComposeSend())) return 'FAIL: no hay botón Send en el compositor';
  await sleep(5000);
  // The notice must be inside the composer (the sidebar's sync banner also says "Authentication required").
  const t = await js(() => document.querySelector('input[placeholder^="Email subject"]')?.closest('.fixed, [role="dialog"]')?.innerText || '');
  const m = t.match(/Failed to send[^\n]*|not authenticated[^\n]*|Authentication required[^\n]*|could not[^\n]*|error[^\n]*/i);
  return ok(!!m, `aviso: ${m?.[0]}`, 'ningún aviso de error en el compositor tras Send');
});
await step('Compose', 'cerrar y borrador', 'al cancelar, el borrador aparece en Drafts (los borradores se guardan automáticamente)', async () => {
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

// ---------- Settings ----------
await step('Ajustes', 'abrir', 'el diálogo de ajustes abre con sus pestañas', async () => {
  await click('aria/Application settings'); await sleep(1500);
  const t = await bodyText(); const tabs = ['Appearance', 'AI Backend & Models', 'AI Classification', 'Privacy & Security', 'Junk'].filter(x => t.includes(x));
  return ok(tabs.length >= 4, `pestañas: ${tabs.join(', ')}`, `pestañas visibles: ${tabs.join(', ')}`);
});
for (const tab of ['AI Backend', 'AI Classification', 'AI Search', 'AI Drafts', 'AI Translation', 'Privacy', 'Junk', 'Calendar', 'Appearance']) {
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
await step('Ajustes', 'Escape cierra el diálogo', 'como en el resto de modales, Escape cierra Ajustes', async () => {
  await b.keys('Escape'); await sleep(800);
  return ok(!(await exists('aria/Close settings')), 'cerrado con Escape', 'Escape no cierra el diálogo de Ajustes (el resto de modales sí)');
});
await step('Ajustes', 'cerrar', 'el botón Close settings cierra el diálogo', async () => {
  if (await exists('aria/Close settings')) await click('aria/Close settings');
  await sleep(1000); return ok(!(await exists('aria/Close settings')), 'cerrado', 'el diálogo sigue abierto');
});

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
  // Done when the bubble shows its sources footer ("N sources used" / "Sources") or its token count, and nothing is still streaming.
  while (Date.now() - t0 < 120000) { await sleep(5000); t = await bodyText(); if ((/sources used|Sources|tokens/i.test(t.replace(before, ''))) && !/Waiting for reply/.test(t)) { got = true; break; } }
  const secs = Math.round((Date.now() - t0) / 1000);
  const answer = t.split('\n').filter(l => /Ollama|Kwame/.test(l) && !before.includes(l)).slice(0, 3).join(' / ');
  return ok(got, `${secs} s; ${answer || 'respuesta sin mención explícita a Ollama'}`, `sin respuesta tras ${secs} s`);
});
await step('Chat', 'nuevo chat', 'New chat vacía la conversación', async () => { await click('aria/New chat'); await sleep(1200); return ok(!/sources used|Sources/i.test(await bodyText()), 'conversación nueva', 'la respuesta anterior sigue visible'); });
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

fs.writeFileSync(path.join(out, 'results.json'), JSON.stringify(results, null, 2));
const fails = results.filter(r => r.status === 'fail').length;
console.log(`\n${results.length} pasos, ${fails} fallos → ${out}`);
await b.deleteSession().catch(() => {});
