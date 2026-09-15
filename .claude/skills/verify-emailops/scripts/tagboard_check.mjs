// Tag Board oracle check: what the board renders must equal what the database
// says it should, block by block and thread by thread.
//
//   node tagboard_check.mjs <run_dir>        (env TAURI_WEBDRIVER_PORT, default 4445)
//
// The oracle reproduces `Database::tag_board_stats_query` (src-tauri/src/db/tags.rs):
// scope → tag type → is_deleted=0 → mailbox in (inbox, sent) → junk excluded →
// only the newest tagged message of a thread counts → window (categories, tag
// search, [since, until)) → GROUP BY account, tag → LIMIT max columns + hidden.
// The UI side is read through the data-* hooks on TagColumn / TagEmailCard.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { remote } from 'webdriverio';

const runDir = process.argv[2];
const out = path.join(runDir, 'tagboard'); fs.mkdirSync(out, { recursive: true });
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../../../..');
const dataDir = fs.readFileSync(path.join(runDir, 'data_dir'), 'utf8').trim();
const db = path.join(dataDir, 'emailops.db');
const MAX_COLUMNS = Number(/TAG_BOARD_MAX_COLUMNS = (\d+)/.exec(fs.readFileSync(path.join(repo, 'src/lib/tagBoard.ts'), 'utf8'))[1]);
const port = Number(process.env.TAURI_WEBDRIVER_PORT || 4445);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------- oracle ----------
function sql(query) {
  const txt = execFileSync('sqlite3', ['-json', '-readonly', db, query], { encoding: 'utf8' });
  return txt.trim() ? JSON.parse(txt) : [];
}
const q = (s) => `'${String(s).replace(/'/g, "''")}'`;
const enabledAccounts = sql('SELECT id, email FROM accounts WHERE enabled = 1 ORDER BY sort_order, id');

/** Threads the backend would count: one row per (account, tag, thread). */
function oracleThreads({ accountId, tagType, window = {} }) {
  const scope = accountId ? `e.account_id = ${q(accountId)}` : `e.account_id IN (${enabledAccounts.map((a) => q(a.id)).join(',')})`;
  const junkKinds = window.hideGraymail ? "('spam','phishing','graymail')" : "('spam','phishing')";
  const parts = [scope, `t.tag_type = ${q(tagType)}`, 'e.is_deleted = 0', "e.mailbox IN ('inbox','sent')",
    `NOT EXISTS (SELECT 1 FROM email_junk j WHERE j.email_id = e.id AND j.band = 'junk' AND j.primary_kind IN ${junkKinds} AND (j.user_override IS NULL OR j.user_override <> 'not_junk'))`,
    `NOT EXISTS (SELECT 1 FROM emails n JOIN email_tags nt ON nt.email_id = n.id AND nt.tag_type = ${q(tagType)} WHERE n.account_id = e.account_id AND n.thread_id = e.thread_id AND n.is_deleted = 0 AND n.mailbox IN ('inbox','sent') AND (n.timestamp > e.timestamp OR (n.timestamp = e.timestamp AND n.id > e.id)))`];
  if (window.search) parts.push(`LOWER(t.tag_value) LIKE ${q('%' + window.search.toLowerCase() + '%')}`);
  if (window.since != null) parts.push(`e.timestamp >= ${Math.floor(window.since)}`);
  if (window.until != null) parts.push(`e.timestamp < ${Math.floor(window.until)}`);
  if (window.categories?.length) parts.push(`e.category IN (${window.categories.map(q).join(',')})`);
  return sql(`SELECT e.account_id AS aid, t.tag_value AS tag, e.thread_id AS tid FROM email_tags t JOIN emails e ON e.id = t.email_id WHERE ${parts.join(' AND ')} GROUP BY aid, tag, tid`);
}
/** Blocks the board should show: top (MAX_COLUMNS + hidden) by count, minus hidden. */
function oracleBlocks(opts, hidden = new Set(), cap = MAX_COLUMNS + hidden.size) {
  const byBlock = new Map();
  for (const r of oracleThreads(opts)) {
    const k = `${r.aid}\n${r.tag}`;
    if (!byBlock.has(k)) byBlock.set(k, { aid: r.aid, tag: r.tag, threads: new Set() });
    byBlock.get(k).threads.add(r.tid);
  }
  const ranked = [...byBlock.values()].sort((a, b) => b.threads.size - a.threads.size || a.aid.localeCompare(b.aid) || a.tag.localeCompare(b.tag));
  return ranked.slice(0, cap).filter((b) => !hidden.has(`${b.aid}\n${b.tag}`));
}
const startOfLocalDay = (d = new Date()) => { const x = new Date(d); x.setHours(0, 0, 0, 0); return Math.floor(x.getTime() / 1000); };
const DAY = 86400;

// ---------- UI ----------
const b = await remote({ hostname: '127.0.0.1', port, path: '/', capabilities: {}, logLevel: 'error' });
const js = (fn, ...a) => b.execute(fn, ...a);
const exists = async (sel) => (await b.$(sel)).isExisting();
async function click(sel, t = 5000) { const el = await b.$(sel); await el.waitForClickable({ timeout: t }); await el.click(); }
const results = []; let n = 0;
async function shot(name) { const f = `${String(++n).padStart(2, '0')}-${name}.png`; await b.saveScreenshot(path.join(out, f)); return f; }
async function step(name, kind, expect, fn) {
  const rec = { feature: 'Tag Board', kind, step: name, expect, status: 'ok', detail: '', shot: null, t: new Date().toISOString().slice(11, 19) };
  try { const d = await fn(); rec.detail = typeof d === 'string' ? d : JSON.stringify(d); if (rec.detail.startsWith('FAIL:')) rec.status = 'fail'; if (rec.detail.startsWith('SKIP:')) rec.status = 'skip'; }
  catch (e) { rec.status = 'fail'; rec.detail = `FAIL: ${e.message.split('\n')[0]}`; }
  rec.shot = await shot(name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()).catch(() => null);
  results.push(rec); console.log(`${rec.status.toUpperCase().padEnd(4)} [${kind}] ${name}: ${rec.detail.slice(0, 160)}`);
}
const ok = (c, good, bad) => (c ? good : `FAIL: ${bad}`);

/** Ranked stats straight from the backend command the view calls. */
const backendStats = (accountId, tagType, window, limit) => js(async (a, t, w, l) => {
  const r = await window.__TAURI_INTERNALS__.invoke('get_tag_board_stats', { accountId: a, tagType: t, window: { latestTagOnly: true, hideGraymail: false, ...w }, limit: l });
  return r.map((x) => ({ aid: x.accountId, tag: x.tagValue, count: x.count, score: x.score }));
}, accountId, tagType, window || {}, limit);
/** Blocks as rendered: [{aid, tag, count, loaded, hasMore}]. */
const uiBlocks = () => js(() => [...document.querySelectorAll('[data-testid="tag-column"]')].map((s) => ({
  aid: s.dataset.accountId, tag: s.dataset.tagValue, count: Number(s.dataset.threadCount), loaded: Number(s.dataset.loaded), hasMore: s.dataset.hasMore === 'true',
})));
/** Expand every block fully, then return thread ids per block. */
async function uiThreads() {
  for (let i = 0; i < 40; i++) {
    const more = await js(() => { const btn = [...document.querySelectorAll('[data-testid="tag-column"][data-has-more="true"] button')].find((x) => x.textContent.trim() === 'Show more'); if (btn) { btn.click(); return true; } return false; });
    if (!more) break; await sleep(700);
  }
  return js(() => Object.fromEntries([...document.querySelectorAll('[data-testid="tag-column"]')].map((s) => [`${s.dataset.accountId}\n${s.dataset.tagValue}`, [...s.querySelectorAll('[data-testid="tag-card"]')].map((c) => c.dataset.threadId)])));
}
const sameSet = (a, b) => a.length === b.length && a.every((x) => b.includes(x));
function diffBlocks(want, got, wantCount, gotCount) {
  const key = (x) => `${x.aid} · ${x.tag}`;
  const missing = want.filter((w) => !got.some((g) => g.aid === w.aid && g.tag === w.tag)).map(key);
  const extra = got.filter((g) => !want.some((w) => w.aid === g.aid && w.tag === g.tag)).map(key);
  const badCount = want.filter((w) => { const g = got.find((x) => x.aid === w.aid && x.tag === w.tag); return g && gotCount(g) !== wantCount(w); })
    .map((w) => `${key(w)}: ${gotCount(got.find((x) => x.aid === w.aid && x.tag === w.tag))} ≠ ${wantCount(w)}`);
  return { ok: !missing.length && !extra.length && !badCount.length, missing, extra, badCount };
}
const fmtDiff = (d, gotN, wantN) => `${gotN} vs ${wantN} esperados${d.missing.length ? `; faltan: ${d.missing.join(', ')}` : ''}${d.extra.length ? `; sobran: ${d.extra.join(', ')}` : ''}${d.badCount.length ? `; recuentos: ${d.badCount.join(', ')}` : ''}`;
/** Layer 1 — backend ↔ DB: every candidate block and its count, no truncation. */
async function compareBackendToDb(accountId, tagType, window) {
  const want = oracleBlocks({ accountId, tagType, window }, new Set(), Infinity);
  const got = await backendStats(accountId, tagType, window, 100000);
  const d = diffBlocks(want, got, (w) => w.threads.size, (g) => g.count);
  return { ok: d.ok, summary: `backend↔BD: ${fmtDiff(d, got.length, want.length)}` };
}
/** Blocks the user hid earlier (smart_filter_prefs, status "removed"), keyed like the oracle. */
const hiddenPrefs = (accountId, tagType) => js(async (a, t) => {
  const prefs = await window.__TAURI_INTERNALS__.invoke('get_filter_prefs', { accountId: a });
  return prefs.filter((p) => p.status === 'removed' && p.filterType === t).map((p) => `${p.accountId}\n${p.filterValue}`);
}, accountId, tagType);
/** Layer 2 — UI ↔ backend: the visible blocks are the backend's ranked top MAX_COLUMNS after hiding. */
async function compareUiToBackend(accountId, tagType, window, extraHidden = new Set()) {
  const hidden = new Set([...(await hiddenPrefs(accountId, tagType)), ...extraHidden]);
  const ranked = await backendStats(accountId, tagType, window, MAX_COLUMNS + hidden.size);
  const want = ranked.filter((r) => !hidden.has(`${r.aid}\n${r.tag}`)).slice(0, MAX_COLUMNS);
  const got = await uiBlocks();
  const d = diffBlocks(want, got, (w) => w.count, (g) => g.count);
  const orderOk = want.every((w, i) => got[i] && got[i].aid === w.aid && got[i].tag === w.tag);
  return { ok: d.ok, orderOk, summary: `UI↔backend: ${fmtDiff(d, got.length, want.length)}${d.ok && !orderOk ? ' (orden distinto: hay orden manual guardado)' : ''}` };
}
async function compareBlocks(label, opts, hidden) {
  const a = await compareBackendToDb(opts.accountId, opts.tagType, opts.window);
  const u = await compareUiToBackend(opts.accountId, opts.tagType, opts.window, hidden);
  return { ok: a.ok && u.ok, summary: `${a.summary}; ${u.summary}` };
}
async function selectType(type) { await click(`button=${type}`); await sleep(1500); }
async function selectRange(label) { await click(`button=${label}`); await sleep(1500); }

// ---------- cases ----------
await click('button*=ulises@emailopslabs.dev'); await sleep(800);
const chatWasDocked = await exists('aria/Close chat panel');
if (chatWasDocked) { await click('aria/Close chat panel'); await sleep(600); }
await click('button=Tag Board'); await sleep(1800);
const work = enabledAccounts.find((a) => a.email === 'ulises@emailopslabs.dev')?.id;
const personal = enabledAccounts.find((a) => a.email === 'ulises@fastmail.com')?.id;
if (!(await exists('button=All time'))) await click('button=Custom').catch(() => {});
await selectRange('All time');

for (const [label, type] of [['Company', 'company'], ['Intent', 'intent'], ['Topic', 'topic'], ['Priority', 'priority']]) {
  await selectType(label);
  await step(`bloques completos · ${label}`, 'oráculo UI↔BD', `cada (cuenta, tag) con hilos en la BD tiene bloque, ninguno sobra, y el recuento coincide (${MAX_COLUMNS} máx.)`, async () => {
    const r = await compareBlocks(label, { accountId: work, tagType: type });
    if (r.ok && (await uiBlocks()).length === 0) return oracleThreads({ accountId: work, tagType: type }).length === 0 ? 'SKIP: la BD demo no tiene tags de este tipo' : 'FAIL: la BD tiene tags pero la UI no muestra bloques';
    return ok(r.ok, r.summary, r.summary);
  });
  await step(`hilos completos · ${label}`, 'oráculo UI↔BD', 'tras "Show more" hasta el final, cada bloque lista exactamente los hilos que la BD le asigna, sin duplicados', async () => {
    const all = oracleBlocks({ accountId: work, tagType: type }, new Set(), Infinity); if (!all.length) return 'SKIP: sin bloques';
    const got = await uiThreads(); const problems = []; let checked = 0, threads = 0;
    for (const [k, g] of Object.entries(got)) {
      const [aid, tag] = k.split('\n'); const w = all.find((x) => x.aid === aid && x.tag === tag);
      if (!w) { problems.push(`${tag}: bloque sin hilos en la BD`); continue; }
      checked++; threads += w.threads.size;
      const dup = g.length !== new Set(g).size;
      const miss = [...w.threads].filter((t) => !g.includes(t)); const extra = g.filter((t) => !w.threads.has(t));
      if (dup || miss.length || extra.length) problems.push(`${tag}: ${miss.length} faltan, ${extra.length} sobran${dup ? ', duplicados' : ''}`);
    }
    return ok(!problems.length, `${checked} bloques visibles, ${threads} hilos verificados uno a uno`, problems.join(' | '));
  });
}

await selectType('Company');
const now = new Date(); const t0 = startOfLocalDay(now);
for (const [label, window] of [['Today', { since: t0 }], ['Yesterday', { since: t0 - DAY, until: t0 }], ['Last 7 days', { since: t0 - 6 * DAY }]]) {
  await step(`rango ${label}`, 'oráculo UI↔BD', 'los bloques son exactamente los que tienen un hilo cuyo último mensaje etiquetado cae en la ventana [desde, hasta)', async () => {
    await selectRange(label); const r = await compareBlocks(label, { accountId: work, tagType: 'company', window });
    return ok(r.ok, r.summary, r.summary);
  });
}
await step('rango Custom', 'oráculo UI↔BD', 'un rango escrito a mano (hace 8 → hace 2 días) filtra como [desde 00:00, hasta+1 día)', async () => {
  await selectRange('Custom');
  // Local calendar date, not toISOString(): at UTC+2 a local midnight prints as the previous UTC day.
  const fmt = (ts) => { const d = new Date(ts * 1000); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`; };
  const from = t0 - 8 * DAY, to = t0 - 2 * DAY;
  const setDate = (label, value) => js((l, v) => { const i = document.querySelector(`input[aria-label="${l}"]`); const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; set.call(i, v); i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new Event('change', { bubbles: true })); return i.value; }, label, value);
  await setDate('From', fmt(from)); await setDate('to', fmt(to)); await sleep(1500);
  const r = await compareBlocks('custom', { accountId: work, tagType: 'company', window: { since: from, until: to + DAY } });
  return ok(r.ok, `${fmt(from)} → ${fmt(to)}: ${r.summary}`, r.summary);
});
await selectRange('All time');

await step('buscar tag', 'oráculo UI↔BD', 'el buscador deja solo los bloques cuyo tag contiene el texto (sin distinguir mayúsculas)', async () => {
  const s = await b.$('input[placeholder^="Search tags"]'); await s.click(); await s.setValue('Fly'); await sleep(1500);
  const r = await compareBlocks('search', { accountId: work, tagType: 'company', window: { search: 'Fly' } });
  await s.setValue(''); await sleep(800); return ok(r.ok, r.summary, r.summary);
});

await step('ocultar junk', 'oráculo UI↔BD', 'con "Hide junk messages" el graymail deja de contar', async () => {
  const cb = await b.$('//label[contains(., "Hide junk messages")]//input'); if (!(await cb.isExisting())) return 'FAIL: no hay casilla Hide junk';
  await cb.click(); await sleep(1500);
  const r = await compareBlocks('junk', { accountId: work, tagType: 'company', window: { hideGraymail: true } });
  await cb.click(); await sleep(800);
  const junk = sql("SELECT COUNT(*) AS n FROM email_junk WHERE band = 'junk'")[0]?.n ?? 0;
  return ok(r.ok, `${r.summary} (veredictos junk en la BD demo: ${junk}${junk ? '' : ', el caso no discrimina hasta que el detector marque algo'})`, r.summary);
});

await step('abrir hilo', 'DOM↔BD', 'pulsar una tarjeta abre el hilo cuyo asunto es el del último mensaje de ese hilo en la BD', async () => {
  const first = await js(() => { const c = document.querySelector('[data-testid="tag-card"]'); return c ? { aid: c.dataset.accountId, tid: c.dataset.threadId } : null; });
  if (!first) return 'FAIL: sin tarjetas';
  await click('[data-testid="tag-card"]'); await sleep(1500);
  const subject = sql(`SELECT subject FROM emails WHERE account_id = ${q(first.aid)} AND thread_id = ${q(first.tid)} AND is_deleted = 0 ORDER BY timestamp DESC, id DESC LIMIT 1`)[0]?.subject;
  const h1 = await js(() => [...document.querySelectorAll('h1')].map((h) => h.textContent.trim()).filter((t) => !/^(EmailOps|Tag Board)$/.test(t))[0] || '');
  const closeBtn = await b.$('main [aria-label="Close"], main button[title="Close"]'); if (await closeBtn.isExisting()) await closeBtn.click(); await sleep(600);
  return ok(h1 === subject, `"${h1}"`, `UI "${h1}" ≠ BD "${subject}"`);
});

await step('ocultar bloque y persistencia', 'DOM↔BD', 'Hide this tag quita el bloque, el siguiente por recuento entra, sobrevive a una recarga y "Show 1 hidden tag" lo devuelve', async () => {
  const before = await uiBlocks(); if (!before.length) return 'SKIP: sin bloques';
  const target = before[0];
  await js((tag) => document.querySelector(`[data-testid="tag-column"][data-tag-value="${tag}"] [aria-label="Block options"]`).click(), target.tag); await sleep(500);
  await js(() => [...document.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Hide this tag')?.click()); await sleep(1500);
  const hidden = new Set([`${target.aid}\n${target.tag}`]);
  const r1 = await compareBlocks('hidden', { accountId: work, tagType: 'company' }, hidden);
  await js(() => location.reload()); await sleep(3000);
  if (!(await exists('button=Tag Board'))) return 'FAIL: la app no volvió tras recargar';
  await click('button=Tag Board'); await sleep(1800);
  const afterReload = await uiBlocks();
  const stillHidden = !afterReload.some((x) => x.aid === target.aid && x.tag === target.tag);
  const restore = await b.$('button*=hidden tag'); const hasRestore = await restore.isExisting();
  if (hasRestore) { await restore.click(); await sleep(1500); }
  const back = (await uiBlocks()).some((x) => x.aid === target.aid && x.tag === target.tag);
  const okAll = r1.ok && stillHidden && hasRestore && back;
  return ok(okAll, `ocultado "${target.tag}", oráculo con ocultos: ${r1.summary}; persiste tras recarga; restaurado`, `oráculo: ${r1.summary}; persiste=${stillHidden}; control restaurar=${hasRestore}; restaurado=${back}`);
});

await step('cambio de cuenta', 'oráculo UI↔BD', 'la cuenta personal y All accounts muestran sus propios bloques por (cuenta, tag)', async () => {
  await click('button*=ulises@fastmail.com'); await sleep(1800);
  const r1 = await compareBlocks('personal', { accountId: personal, tagType: 'company' });
  await click('button=All accounts'); await sleep(1800);
  const r2 = await compareBlocks('all', { accountId: null, tagType: 'company' });
  await click('button*=ulises@emailopslabs.dev'); await sleep(1200);
  return ok(r1.ok && r2.ok, `personal: ${r1.summary}; todas: ${r2.summary}`, `personal: ${r1.summary}; todas: ${r2.summary}`);
});

await step('anchura de bloque', 'DOM', '"Wide blocks" pone menos bloques por fila que "Narrow blocks"', async () => {
  const perRow = () => js(() => new Set([...document.querySelectorAll('[data-testid="tag-column"]')].map((x) => Math.round(x.getBoundingClientRect().left))).size);
  await click('aria/Narrow blocks'); await sleep(800); const narrow = await perRow();
  await click('aria/Wide blocks'); await sleep(800); const wide = await perRow();
  await click('aria/Narrow blocks'); await sleep(500);
  return ok(wide < narrow, `${narrow} por fila estrechos, ${wide} anchos`, `${narrow} estrechos vs ${wide} anchos`);
});

if (chatWasDocked) { await click('button=Inbox'); await sleep(600); if (await exists('aria/Open chat panel')) await click('aria/Open chat panel'); }
fs.writeFileSync(path.join(out, 'results.json'), JSON.stringify(results, null, 2));
console.log(`\n${results.length} casos, ${results.filter((r) => r.status === 'fail').length} fallos, ${results.filter((r) => r.status === 'skip').length} n/a → ${out}`);
await b.deleteSession().catch(() => {});
