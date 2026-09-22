// Verify the published docs against the running app.
//
//   node doc_claims.mjs <phase> <out_dir>
//     phase fresh   — a brand-new data dir: first-run wizard, factory defaults,
//                     every settings tab, what lands in the data dir
//     phase locked  — the same data dir relaunched after `fresh` set a main
//                     password: the lock screen, and the database still readable
//     phase demo    — the synthetic demo mailbox: reading pane, views, tag board
//   env CLAIMS_JSON  — {claim id: text} for docs/site/en, written by the runner
//   env DATA_DIR     — the instance's data dir (fresh and locked phases)
//
// The app is the ground truth for what the docs promise. Each case reads the
// claim's own text from the docs and checks what it quotes against the screen,
// so a check is tied to the published sentence, not to an expectation typed
// into this file: rewrite the sentence and the check follows it.
//
// Writes <out_dir>/<phase>.json: [{claim, status, detail, shot, fix}].
import { remote } from 'webdriverio';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const [phase, outDir] = process.argv.slice(2);
if (!phase || !outDir) { console.error('usage: doc_claims.mjs fresh|locked|demo <out_dir>'); process.exit(2); }
fs.mkdirSync(outDir, { recursive: true });
const CLAIMS = JSON.parse(fs.readFileSync(process.env.CLAIMS_JSON, 'utf8'));
const DATA_DIR = process.env.DATA_DIR || '';
const port = Number(process.env.TAURI_WEBDRIVER_PORT || 4445);
const b = await remote({ hostname: '127.0.0.1', port, path: '/', capabilities: {}, logLevel: 'error' });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const js = (fn, ...a) => b.execute(fn, ...a);
const screen = () => js(() => document.body.innerText.replace(/\s+/g, ' '));
const norm = (s) => s.replace(/\s+/g, ' ').trim();
let shots = 0;
async function shot(name) {
  const f = `${phase}-${String(++shots).padStart(2, '0')}-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`;
  await b.saveScreenshot(path.join(outDir, f)).catch(() => {});
  return f;
}

// Click the first visible button (or role=button) whose text starts with `text`.
async function press(text) {
  const ok = await js((t) => {
    const el = [...document.querySelectorAll('button,[role=button]')]
      .find((e) => e.offsetParent !== null && e.innerText.replace(/\s+/g, ' ').trim().startsWith(t));
    if (!el || el.disabled) return false;
    el.click();
    return true;
  }, text);
  if (!ok) throw new Error(`no clickable «${text}» on screen`);
  await sleep(900);
}
const buttonState = (text) => js((t) => {
  const el = [...document.querySelectorAll('button')].find((e) => e.offsetParent !== null && e.innerText.trim() === t);
  return el ? (el.disabled ? 'disabled' : 'enabled') : 'missing';
}, text);

// The bold spans of a claim: what the docs tell the reader to look for.
function bold(claimId) {
  const text = CLAIMS[claimId];
  if (text === undefined) throw new Error(`claim ${claimId} is not in the docs`);
  return [...text.matchAll(/\*\*(.+?)\*\*/gs)].map((m) => norm(m[1]));
}
// Assert every quoted label is on screen; `except` names spans that are prose.
async function labelsVisible(claimId, { except = [], only = null } = {}) {
  const want = (only || bold(claimId)).filter((l) => !except.includes(l));
  for (const l of only || []) if (!bold(claimId).includes(l) && !norm(CLAIMS[claimId]).includes(l)) {
    throw new Error(`the claim no longer mentions «${l}»; update doc_claims.mjs`);
  }
  const s = await screen();
  const missing = want.filter((l) => !s.includes(l));
  return { missing, want };
}

// ── results ────────────────────────────────────────────────────────────────
// A claim may be checked on several screens; it passes only if every part does.
const results = new Map();
async function claim(id, name, fn, fix = '') {
  if (!(id in CLAIMS)) throw new Error(`doc_claims.mjs checks claim:${id}, which the docs no longer have`);
  const rec = results.get(id) || { claim: id, parts: [], shots: [], fix };
  let status = 'ok', detail;
  try {
    detail = await fn();
    if (typeof detail === 'string' && detail.startsWith('FAIL:')) status = 'fail';
    if (typeof detail === 'string' && detail.startsWith('SKIP:')) status = 'skip';
  } catch (e) {
    status = 'fail'; detail = `FAIL: ${e.message.split('\n')[0]}`;
  }
  rec.parts.push({ name, status, detail: String(detail) });
  rec.shots.push(await shot(`${id}-${name}`));
  if (fix) rec.fix = fix;
  results.set(id, rec);
  console.log(`${status.toUpperCase().padEnd(4)} ${id} / ${name}: ${String(detail).slice(0, 150)}`);
}
const ok = (cond, good, bad) => (cond ? good : `FAIL: ${bad}`);
function write() {
  const out = [...results.values()].map((r) => {
    const failed = r.parts.filter((p) => p.status === 'fail');
    const status = failed.length ? 'fail' : r.parts.every((p) => p.status === 'skip') ? 'skip' : 'ok';
    const detail = (failed.length ? failed : r.parts).map((p) => `${p.name}: ${p.detail.replace(/^(FAIL|SKIP): /, '')}`).join(' · ');
    return { claim: r.claim, status, detail, shots: r.shots, fix: status === 'fail' ? r.fix : '', page: '' };
  });
  fs.writeFileSync(path.join(outDir, `${phase}.json`), JSON.stringify(out, null, 2));
  console.log(`\n${out.length} claims, ${out.filter((r) => r.status === 'fail').length} failing → ${outDir}/${phase}.json`);
}

// ── settings helpers ───────────────────────────────────────────────────────
async function openSettings() {
  if (!(await screen()).includes('Password, remote content')) {
    await js(() => document.querySelector('[aria-label="Application settings"],[title="Application settings"]').click());
    await sleep(1200);
  }
}
async function tab(name) {
  await openSettings();
  await press(name);
  await sleep(600);
  return js(() => {
    const t = document.body.innerText;
    const i = t.indexOf('Password, remote content');
    return (i >= 0 ? t.slice(i) : t).replace(/\s+/g, ' ');
  });
}
async function closeSettings() {
  if ((await screen()).includes('Password, remote content')) { await press('Close').catch(() => {}); await sleep(600); }
}
// Toggle states in the open panel, keyed by the first line of their row.
const toggles = () => js(() => Object.fromEntries(
  [...document.querySelectorAll('button[aria-pressed],button[aria-checked],[role=switch],input[type=checkbox]')]
    .filter((e) => e.offsetParent !== null)
    .map((e) => {
      let row = e.parentElement;
      for (let i = 0; i < 4 && row && !row.innerText.trim(); i++) row = row.parentElement;
      const label = (row?.innerText || e.getAttribute('aria-label') || '').trim().split('\n')[0];
      const on = e.getAttribute('aria-pressed') ?? e.getAttribute('aria-checked') ?? String(e.checked);
      return [label, on === 'true'];
    })));

// Flip the toggle whose row starts with `label`. Never "the first toggle on
// screen": the sidebar has its own (Chat), and clicking it instead is silent.
async function flip(label) {
  const done = await js((l) => {
    const el = [...document.querySelectorAll('button[aria-pressed],button[aria-checked],[role=switch]')]
      .filter((e) => e.offsetParent !== null)
      .find((e) => {
        let row = e.parentElement;
        for (let i = 0; i < 4 && row && !row.innerText.trim(); i++) row = row.parentElement;
        return (row?.innerText || '').trim().startsWith(l);
      });
    if (el) el.click();
    return !!el;
  }, label);
  if (!done) throw new Error(`no toggle labelled «${label}»`);
  await sleep(1500);
}

// ═══════════════════════════════ fresh ═══════════════════════════════════
async function fresh() {
  // Always a first run in the real pipeline; the guard only lets a developer
  // re-run the settings half against an instance whose wizard is already done.
  if (/STEP 1 OF/.test(await screen())) await wizard();
  await afterWizard();
}

async function wizard() {
  const wizardSteps = {};

  // Step 1 — AI or plain.
  wizardSteps.ai = (await screen()).match(/STEP 1 OF (\d)/)?.[1];
  await claim('start-1-ai-1', 'paso 1', async () => {
    const s = await screen();
    return ok(/Recommended — (this machine can run AI locally|no AI hardware required)/.test(s),
      'el asistente recomienda según el hardware', 'no hay recomendación basada en el hardware');
  });
  await claim('start-1-ai-2', 'opción IA', async () => {
    const { missing } = await labelsVisible('start-1-ai-2');
    const s = await screen();
    const feats = ['Chat with your inbox', 'semantic search', 'classification'].filter((f) => !s.includes(f));
    return ok(!missing.length && !feats.length, 'opción y funciones visibles', `falta: ${[...missing, ...feats].join(', ')}`);
  }, 'Quote the wizard option exactly as step 1 shows it, in each language.');
  await claim('start-1-ai-3', 'opción sin IA', async () => {
    const s = await screen();
    const { missing } = await labelsVisible('start-1-ai-3', { only: ['Plain email client'] });
    return ok(!missing.length && s.includes('No AI calls, ever'), '«Plain email client» y «No AI calls, ever»', `falta: ${missing}`);
  });
  await claim('ai-classification-1', 'asistente', async () => ok(/Auto-classification \(priority, intent, topic\)/.test(await screen()),
    'el asistente anuncia prioridad, intención y tema', 'no'));
  await claim('inst-system-requirements-1', 'paso 1', async () => {
    const s = await screen();
    return ok(s.includes('Plain email client'), 'el asistente ofrece rechazar la IA', 'no hay opción sin IA');
  });

  // Step 2 — backend and model (AI path). Continue stays disabled without a model.
  await press('Use AI'); await press('Continue');
  await claim('start-2-ai-1', 'paso 2', async () => {
    const { missing } = await labelsVisible('start-2-ai-1');
    const s = await screen();
    const shown = ['In-app', 'Ollama', 'OpenRouter'].filter((l) => s.includes(l));
    return ok(!missing.length, 'los tres backends con sus nombres', `el doc cita ${missing.map((m) => `«${m}»`).join(', ')}; el asistente muestra ${shown.join(', ')}`);
  }, 'Quote the backend names as the wizard shows them (In-app, Ollama, OpenRouter) in all four languages.');
  await claim('start-2-ai-2', 'recomendado según memoria', async () => {
    // The badge must sit on the largest model with twice its RAM floor free —
    // the rule the page states — for THIS machine's memory.
    const ramGb = os.totalmem() / 1024 ** 3;
    const rows = await js(() => [...document.body.innerText.matchAll(/\n([^\n]+)\n(Recommended\n)?(\d+)\+ GB RAM · ([\d.]+) GB/g)]
      .map((m) => ({ name: m[1].trim(), rec: !!m[2], ram: +m[3] })));
    const fits = rows.filter((r) => r.ram * 2 <= ramGb);
    const expect = fits.reduce((best, r) => (!best || r.ram > best.ram ? r : best), null) || rows.reduce((a, r) => (r.ram < a.ram ? r : a));
    const badged = rows.find((r) => r.rec);
    const doc = norm(CLAIMS['start-2-ai-2']);
    const anchored = doc.match(/on a (\d+) GB machine that is \*\*([^*]+)\*\*/);
    return ok(badged && badged.name === expect.name && anchored,
      `${Math.round(ramGb)} GB → «${badged?.name}» recomendado, como predice la regla del doc`,
      `${Math.round(ramGb)} GB: la app recomienda «${badged?.name}», la regla predice «${expect.name}»`);
  });
  await claim('priv-there-no-1', 'origen de los modelos', async () => ok(/Models downloaded from Hugging Face/.test(await screen()),
    'el asistente declara Hugging Face como origen de los modelos', 'no lo dice'));
  await claim('start-2-ai-4', 'embeddings incluidos', async () => {
    const s = await screen();
    return ok(/built-in with the app — no download needed/.test(s), 'el paso 2 dice que el modelo de búsqueda viene incluido', 'no lo dice');
  });
  await claim('ai-choosing-backend-model-catalog-1', 'tabla vs selector', async () => catalogMatchesDoc());
  await claim('inst-system-requirements-local-ai-2', 'extremos del catálogo', async () => {
    const rows = await catalogRows();
    const ram = rows.map((r) => r.ram);
    const doc = norm(CLAIMS['inst-system-requirements-local-ai-2']);
    const small = doc.match(/needs about (\d+) GB/)?.[1], large = doc.match(/wants (\d+) GB/)?.[1];
    return ok(+small === Math.min(...ram) && +large === Math.max(...ram),
      `${Math.min(...ram)} GB y ${Math.max(...ram)} GB, como dice el doc`, `el selector va de ${Math.min(...ram)} a ${Math.max(...ram)} GB; el doc dice ${small} y ${large}`);
  });
  await claim('ai-choosing-backend-model-catalog-6', 'badge y atenuado', async () => {
    const ramGb = os.totalmem() / 1024 ** 3;
    const rows = await catalogRows();
    const s = await screen();
    const tooBig = rows.filter((r) => r.ram > ramGb);
    if (!s.includes('Recommended')) return 'FAIL: ningún modelo lleva el distintivo Recommended';
    if (!tooBig.length) return `SKIP: con ${Math.round(ramGb)} GB ningún modelo excede la memoria; el atenuado no se puede observar en esta máquina`;
    return 'distintivo presente; modelos que no caben atenuados';
  });
  await claim('start-intro-1', 'pasos con IA', async () => ok(wizardSteps.ai === '4', `con IA: ${wizardSteps.ai} pasos`, `con IA el asistente tiene ${wizardSteps.ai} pasos`));
  await claim('start-2-ai-2', 'Continue bloqueado sin modelo', async () => ok((await buttonState('Continue')) === 'disabled',
    'no se puede continuar sin elegir modelo', 'Continue está habilitado sin modelo'));

  // Plain path: 3 steps, layout, then account.
  await press('Back'); await press('Plain email client'); await press('Continue');
  wizardSteps.plain = (await screen()).match(/STEP \d OF (\d)/)?.[1];
  await claim('start-intro-1', 'pasos sin IA', async () => {
    const doc = norm(CLAIMS['start-intro-1']);
    const saysFour = /four-step wizard/.test(doc);
    return ok(!(saysFour && wizardSteps.plain !== '4'), `sin IA: ${wizardSteps.plain} pasos`,
      `el doc dice «four-step wizard», pero sin IA el asistente tiene ${wizardSteps.plain} pasos`);
  }, 'Say the wizard has up to four steps — three when you choose a plain email client — in all four languages.');
  await claim('start-3-inbox-1', 'paso de diseño', async () => {
    const s = await screen();
    const want = ['Split view', 'Full-width list', 'Settings → Appearance'].filter((l) => !s.includes(l));
    return ok(!want.length, 'dividido / ancho completo / Settings → Appearance', `falta: ${want.join(', ')}`);
  });
  await press('Split view'); await press('Continue');
  await claim('feat-accounts-sync-1', 'proveedores', async () => {
    const s = await screen();
    const want = ['Gmail', 'Outlook / Microsoft 365', 'Graph API', 'IMAP / SMTP'].filter((x) => !s.includes(x));
    return ok(!want.length, 'Gmail, Outlook (Graph API) e IMAP/SMTP', `falta: ${want.join(', ')}`);
  });
  await claim('start-4-connect-2', 'Gmail', async () => ok((await screen()).includes('Sign in with Google OAuth'), 'Gmail vía OAuth en el navegador', 'no hay Gmail con OAuth'));
  await claim('start-4-connect-3', 'Outlook', async () => ok((await screen()).includes('Microsoft OAuth (Graph API)'), 'Outlook vía Graph API', 'no hay Outlook con Graph'));
  await press('IMAP / SMTP');
  await claim('start-4-connect-4', 'formulario IMAP', async () => {
    const doc = norm(CLAIMS['start-4-connect-4']);
    const named = ['iCloud', 'Yahoo', 'Fastmail', 'ProtonMail Bridge'].filter((n) => doc.includes(n));
    const s = await screen();
    const missing = named.filter((n) => !s.includes(n));
    const fields = ['IMAP host', 'SMTP host', 'Password'].filter((f) => !s.includes(f));
    return ok(!missing.length && !fields.length, `presets ${named.join(', ')} y campos de servidor`, `falta: ${[...missing, ...fields].join(', ')}`);
  });
  await claim('feat-accounts-sync-2', 'nombre IMAP', async () => ok((await screen()).includes('Display Name'),
    'el alta IMAP pide un nombre para mostrar', 'el formulario IMAP no pide nombre'));
  await press('Cancel'); await press('Skip for now');
}

async function afterWizard() {
  // The app without AI.
  await claim('ai-tag-board-4', 'barra lateral sin IA', async () => {
    const side = await js(() => (document.querySelector('aside,nav')?.innerText || document.body.innerText).replace(/\s+/g, ' '));
    return ok(!/\bTag Board\b/.test(side), 'sin IA no aparece el Tag Board', 'el Tag Board sigue visible con la IA apagada');
  });
  await claim('ai-turning-off-1', 'modo sin IA', async () => {
    const side = await screen();
    return ok(!/\bChat\b(?! ?about)/.test(side.split('SMART FILTERS')[0].split('AI FEATURES')[1] || ''),
      'sin IA no hay chat en la barra lateral', 'el chat sigue disponible con la IA apagada');
  });
  const plainTabs = await tab('Appearance');
  await claim('feat-intro-1', 'ajustes sin IA', async () => ok(/Junk Spam, impersonation/.test(plainTabs),
    'los ajustes de correo no deseado están disponibles sin IA',
    'con la IA apagada no existe la pestaña Junk, así que las opciones de correo no deseado que describe esta página no se pueden elegir'),
  'Either expose the Junk settings without AI (the detector uses no model), or say on this page that junk handling needs AI switched on.');
  await claim('feat-interface-1', 'idiomas y diseño', async () => {
    const want = ['English', 'Español', 'Français', 'Deutsch', 'Split view', 'Full-width list'].filter((l) => !plainTabs.includes(l));
    return ok(!want.length, 'cuatro idiomas y dos diseños', `falta: ${want.join(', ')}`);
  });
  await claim('start-3-inbox-1', 'Settings → Appearance', async () => ok(/Display language.*English.*Español.*Français.*Deutsch/.test(plainTabs),
    'Appearance tiene diseño e idioma', 'Appearance no tiene el idioma de la interfaz'));
  const priv = await tab('Privacy & Security');
  const privToggles = await toggles();
  await claim('priv-protection-from-2', 'por defecto', async () => ok(privToggles['Allow remote content in emails'] === false && /A banner lets you load them per-email/.test(priv) && /TRUSTED SENDERS/i.test(priv),
    'contenido remoto bloqueado de fábrica, banner por email, remitentes de confianza', `estado: ${JSON.stringify(privToggles)}`));
  await claim('feat-privacy-security-1', 'ajustes', async () => ok(/main password locks the app on startup/.test(priv) && privToggles['Allow remote content in emails'] === false,
    'contraseña principal y contenido remoto bloqueado', 'faltan los controles de privacidad'));
  await claim('priv-locking-app-1', 'ajuste', async () => ok(/Use a main password/.test(priv), '«Use a main password» en Privacy & Security', 'no está el ajuste'));
  const aiOff = await tab('AI Backend & Models');
  await claim('ai-turning-off-1', 'interruptor', async () => ok(/AI Features When off, EmailOps runs as a plain email client/.test(aiOff),
    'el interruptor AI Features apaga todo', 'no hay interruptor maestro'));

  // What landed in the data dir.
  await claim('priv-where-data-5', 'EMAILOPS_DATA_DIR', async () => ok(fs.existsSync(path.join(DATA_DIR, 'emailops.db')),
    `la instancia escribe en el directorio indicado (${path.basename(DATA_DIR)})`, 'el directorio indicado no tiene base de datos'));
  await claim('inst-where-data-5', 'EMAILOPS_DATA_DIR', async () => ok(fs.existsSync(path.join(DATA_DIR, 'emailops.db')), 'se respeta', 'no se respeta'));
  await claim('priv-where-data-4', 'models/', async () => ok(fs.statSync(path.join(DATA_DIR, 'models')).isDirectory(), 'hay carpeta models/ junto a la base de datos', 'no hay models/'));
  await claim('inst-where-data-3', 'models/', async () => ok(fs.existsSync(path.join(DATA_DIR, 'models')), 'models/ junto a la base de datos', 'no hay models/'));
  const tables = sqliteTables();
  await claim('priv-where-data-3', 'tablas', async () => {
    const want = ['emails', 'calendar_events', 'email_tags', 'memory_facts'].filter((t) => !tables.includes(t));
    const vec = tables.some((t) => /embedding|vec/.test(t));
    return ok(!want.length && vec, 'mensajes, calendario, etiquetas, memoria y embeddings en un SQLite', `falta: ${want.join(', ')}${vec ? '' : ' embeddings'}`);
  });
  await claim('inst-where-data-1', 'todo en el directorio de datos', async () => {
    const entries = fs.readdirSync(DATA_DIR);
    return ok(entries.includes('emailops.db') && entries.includes('models'), `el directorio de datos contiene ${entries.filter((e) => !e.startsWith('.')).join(', ')}`, 'falta la base de datos o models/');
  });
  await claim('inst-where-data-2', 'SQLite', async () => ok(tables.includes('emails'), 'una base de datos SQLite local', 'no es SQLite'));

  // Switch AI on; every AI tab is now reachable. Factory defaults are checked here.
  await tab('AI Backend & Models');
  await flip('AI Features');
  await sleep(1000);
  const ai = await tab('AI Backend & Models');
  const all = await js(() => document.body.innerText.replace(/\s+/g, ' '));
  await claim('ai-choosing-backend-1', 'pestaña', async () => ok(/AI Backend In-app .* Ollama .* OpenRouter/.test(ai), 'la pestaña elige el backend', 'no hay selector de backend'));
  await claim('ai-choosing-backend-2', 'nombre', async () => {
    const { missing, want } = await labelsVisible('ai-choosing-backend-2');
    return ok(!missing.length, `«${want.join('»')}» como lo muestra la app`, `el doc cita ${missing.map((m) => `«${m}»`).join(', ')}, que la app no muestra`);
  }, 'Quote the backend exactly as Settings → AI Backend & Models shows it, in all four languages.');
  await claim('ai-choosing-backend-3', 'nombre', async () => {
    const { missing, want } = await labelsVisible('ai-choosing-backend-3');
    return ok(!missing.length, `«${want.join('»')}» como lo muestra la app`, `el doc cita ${missing.map((m) => `«${m}»`).join(', ')}, que la app no muestra`);
  }, 'Quote the backend exactly as Settings → AI Backend & Models shows it, in all four languages.');
  await claim('ai-choosing-backend-4', 'nombre', async () => {
    const { missing, want } = await labelsVisible('ai-choosing-backend-4');
    return ok(!missing.length, `«${want.join('»')}» como lo muestra la app`, `el doc cita ${missing.map((m) => `«${m}»`).join(', ')}, que la app no muestra`);
  }, 'Quote the backend exactly as Settings → AI Backend & Models shows it, in all four languages.');
  await claim('start-2-ai-4', 'embeddings descargados de fábrica', async () => ok(/Nomic Embed Text v1\.5 Recommended Downloaded/.test(ai),
    'Nomic ya está descargado en una instalación nueva', 'Nomic no viene incluido'));
  await claim('ai-choosing-backend-performance-knobs-1', 'keep-alive', async () => ok(/Keep model loaded.*0 to evict immediately.*Default: 30 minutes/.test(ai),
    '«Keep model loaded»: 30 min por defecto, 0 descarga', 'no coincide'));
  await claim('ai-choosing-backend-performance-knobs-2', 'contexto', async () => ok(/Context window/.test(ai), '«Context window» presente', 'no está'));
  await claim('ai-choosing-backend-performance-knobs-3', 'razonamiento', async () => ok(/Thinking Mode Chain-of-thought/i.test(ai), '«Thinking mode» presente', 'no está'));
  await claim('ai-choosing-backend-performance-knobs-4', 'límite', async () => {
    const { missing } = await labelsVisible('ai-choosing-backend-performance-knobs-4');
    const doc = norm(CLAIMS['ai-choosing-backend-performance-knobs-4']);
    const emailLimit = /Defaults: (\d+) emails/.exec(ai)?.[1];
    const complete = !emailLimit || /emails?\b.*limit|\d+ emails/i.test(doc);
    return ok(!missing.length && complete, 'etiqueta y comportamiento como en la app',
      missing.length ? `falta la etiqueta ${missing}` : `la app aplica un límite de ${emailLimit} correos por cuenta además de los días; el doc solo menciona los días`);
  }, 'Describe both limits the setting applies (the email limit and the day limit) in all four languages.');
  await claim('start-after-wizard-first-sync-7', 'ubicación', async () => ok(/Limit AI processing/.test(ai), '«Limit AI processing» en AI Backend & Models', 'no está en esa pestaña'));
  await claim('trbl-search-returns-2', 'ubicación', async () => ok(/Limit AI processing/.test(ai), 'en los ajustes de IA', 'no está'));
  await claim('trbl-chat-slow-4', 'ajuste', async () => ok(ai.includes('Keep model loaded'), '«Keep model loaded» en los ajustes de IA', 'no está'));
  await claim('trbl-chat-slow-5', 'ajuste', async () => ok(ai.includes('Context window'), '«Context window» en los ajustes de IA', 'no está'));
  await claim('trbl-chat-slow-6', 'ajuste', async () => ok(ai.includes('Thinking Mode'), '«Thinking mode» en los ajustes de IA', 'no está'));
  await claim('ai-chat-mailbox-4', 'enrutado configurable', async () => ok(/Chat routing mode How the chat decides between RAG retrieval and direct tool calls/.test(ai),
    'recuperación y herramientas, con modo de enrutado configurable', 'no hay modo de enrutado'));
  await claim('trbl-chat-slow-3', 'modelo más pequeño', async () => {
    const rows = await catalogRows();
    const chat = rows.filter((r) => !/Nomic/.test(r.name));
    const smallest = chat.reduce((a, r) => (r.ram < a.ram ? r : a));
    const named = norm(CLAIMS['trbl-chat-slow-3']).match(/(Qwen [\d.]+ \w+|Gemma [\w .]+?) is the smallest/)?.[1];
    return ok(named === smallest.name, `«${smallest.name}» es el modelo de chat más pequeño del selector`, `el doc nombra «${named}», el más pequeño es «${smallest.name}»`);
  });
  await claim('ai-chat-mailbox-5', 'por defecto', async () => ok(/Always RAG first \(default\)/.test(ai), '«Always RAG first» es el predeterminado', 'no lo es'));
  await claim('ai-chat-mailbox-6', 'opción', async () => ok(/Auto \(heuristic-routed\)/.test(ai), '«Auto» presente', 'no está'));
  await claim('ai-chat-mailbox-7', 'opción', async () => ok(/Always tools first/.test(ai), '«Always tools first» presente', 'no está'));
  await claim('ai-chat-mailbox-9', 'prompts', async () => ok(/Chat prompts .*System prompt/.test(ai), '«Chat prompts» en AI Backend & Models', 'no está'));
  await claim('ai-turning-off-1', 'ubicación', async () => ok(/^.{0,80}AI Features When off/.test(ai.split('Close')[1] || ''), '«AI Features» al principio de AI Backend & Models', 'no está'));
  await claim('feat-interface-1', 'idioma de la IA', async () => ok(/AI output language/.test(ai), 'el idioma de la IA se ajusta aparte', 'no hay idioma de salida de la IA'));
  await claim('trbl-ai-features-1', 'descarga', async () => ok(/Chat Model .*Download/.test(ai) && /Delete/.test(ai), 'los modelos se descargan y borran desde AI Backend & Models', 'no hay descarga'));
  await claim('priv-there-no-2', 'predeterminado', async () => {
    const provider = await js(() => [...document.querySelectorAll('button')].find((e) => e.offsetParent && /^In-app/.test(e.innerText.trim()))?.className || '');
    return ok(/(ring|border-(blue|sky|indigo))/.test(provider) || /In-app Bundled models/.test(ai), 'In-app es el backend de fábrica', 'el backend de fábrica no es In-app');
  });
  await press('OpenRouter');
  const or = await screen();
  await claim('inst-system-requirements-without-local-2', 'IA remota', async () => ok(/API Key/i.test(or), 'la IA puede enrutarse a OpenRouter con una clave de API', 'no'));
  await claim('ai-choosing-backend-4', 'clave y presupuesto', async () => ok(/API Key/i.test(or) && /Monthly Budget/.test(or), 'OpenRouter pide clave de API y admite presupuesto mensual', 'falta clave o presupuesto'));
  await press('Ollama');
  await claim('trbl-ai-features-2', 'Ollama', async () => ok(/Chat Model .*Embedding Model/.test(await screen()), 'Ollama se elige como backend', 'no'));
  await claim('ai-choosing-backend-3', 'seleccionable', async () => ok(/Chat Model .*Embedding Model/.test(await screen()), 'Ollama se elige y pide sus modelos', 'no se puede elegir Ollama'));
  await press('In-app');

  const tabs = await js(() => document.body.innerText.replace(/\s+/g, ' '));
  await claim('ai-intro-1', 'interruptores por función', async () => {
    const perFeature = [];
    for (const t of ['AI Classification', 'AI Tasks', 'AI Memory', 'AI Lenses', 'AI Drafts', 'AI Translation', 'AI Search']) {
      const body = await tab(t);
      if (Object.keys(await toggles()).length) perFeature.push(t);
      void body;
    }
    return ok(perFeature.length >= 6, `${perFeature.length} funciones con su propio interruptor`, `solo ${perFeature.join(', ')} tienen interruptor`);
  });
  const junk = await tab('Junk');
  const junkToggles = await toggles();
  await claim('feat-junk-bulk-2', 'opción', async () => ok(junk.includes('Fade it in the list'), 'opción presente', 'no está'));
  await claim('feat-junk-bulk-3', 'opción', async () => ok(junk.includes('Keep it out of the inbox'), 'opción presente', 'no está'));
  await claim('priv-protection-from-4', 'por defecto', async () => {
    const k = Object.keys(junkToggles).find((l) => /impersonat|phishing/i.test(l));
    return ok(k && junkToggles[k] === false, `«${k}» apagado de fábrica`, `estado: ${JSON.stringify(junkToggles)}`);
  });
  await claim('feat-junk-bulk-4', 'impersonación', async () => {
    const k = Object.keys(junkToggles).find((l) => /impersonat|phishing/i.test(l));
    return ok(k && junkToggles[k] === false, 'aviso de suplantación opcional y apagado', 'no está apagado');
  });
  const cls = await tab('AI Classification');
  await claim('trbl-classification-tagging-1', 'ajuste', async () => ok(/Auto-classify new emails/.test(cls), '«Auto-classify new emails» en AI Classification', 'no está'));
  await claim('trbl-classification-tagging-3', 'acciones', async () => ok(/Classify Unclassified/.test(cls) && /Reclassify All/.test(cls), 'ambas acciones presentes', 'falta alguna'));
  await claim('ai-classification-1', 'intenciones y temas', async () => ok(/Intents .*Topics/.test(cls), 'intenciones y temas configurables', 'no'));
  await claim('ai-classification-5', 'acciones', async () => ok(/Reclassify All/.test(cls) && /Classify Unclassified/.test(cls) && /Gmail inbox tabs/.test(cls), 'pestañas de Gmail a clasificar, reclasificar y ponerse al día', 'falta'));
  await claim('ai-classification-3', 'reglas', async () => ok(/rule/i.test(cls), 'hay reglas de clasificación', 'no hay reglas'));
  await claim('ai-classification-4', 'prompt', async () => ok(/prompt/i.test(cls), 'el prompt de clasificación es editable', 'no hay prompt'));
  await tab('AI Tasks');
  await flip('AI Tasks');
  const tasks = await tab('AI Tasks');
  await claim('ai-tasks-1', 'ajustes', async () => {
    const want = [['Learn only from emails I wrote', /Learn only from emails I wrote/i], ['exclusiones', /exclu/i], ['backfill', /backfill/i]].filter(([, re]) => !re.test(tasks)).map(([n]) => n);
    return ok(!want.length && /EXPERIMENTAL/.test(tabs), 'experimental; solo lo que yo escribí, exclusiones y backfill', `falta: ${want.join(', ')}`);
  });
  const memory = await tab('AI Memory');
  await claim('ai-memory-1', 'ajustes', async () => ok(Object.keys(await toggles()).length > 0 && /EXPERIMENTAL/.test(tabs), 'interruptor propio, marcado experimental', 'falta'));
  void memory;
  const drafts = await tab('AI Drafts');
  await claim('ai-ai-drafts-1', 'ajustes', async () => {
    const doc = norm(CLAIMS['ai-ai-drafts-1']);
    const promised = [['persona', /Persona/], ['writing style', /Writing style/], ['default tone', /Default tone/], ['default length', /Default length/], ['prompt template', /Prompt template/]]
      .filter(([w]) => new RegExp(w.replace('default ', '(default )?'), 'i').test(doc));
    const missing = promised.filter(([, re]) => !re.test(drafts)).map(([w]) => w);
    return ok(!missing.length, `${promised.map(([w]) => w).join(', ')} en AI Drafts`, `el doc promete ${missing.join(' y ')}, que la pestaña AI Drafts no tiene`);
  }, 'AI Drafts offers a persona, a writing style and the prompt template; tone is chosen per draft in the composer. Say that, in all four languages.');
  const tr = await tab('AI Translation');
  await claim('ai-translation-1', 'prompt', async () => ok(/prompt/i.test(tr), 'el prompt de traducción es editable', 'no'));
  await setMainPassword();
  await closeSettings();
  void all;
}

async function setMainPassword() {
  await tab('Privacy & Security');
  await flip('Use a main password');
  const filled = await js(() => {
    const pw = [...document.querySelectorAll('input[type=password]')].filter((i) => i.offsetParent);
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    for (const i of pw) { set.call(i, 'docs-check-pw-1'); i.dispatchEvent(new Event('input', { bubbles: true })); }
    return pw.length;
  });
  if (filled) {
    const saved = await js(() => {
      const bt = [...document.querySelectorAll('button')].find((e) => e.offsetParent && /^(Set password|Save|Enable)/.test(e.innerText.trim()));
      if (bt) bt.click();
      return bt ? bt.innerText.trim() : '';
    });
    await sleep(1500);
    console.log(`main password: ${filled} field(s), saved via «${saved}»`);
  } else console.log('main password: no password fields appeared');
}

async function catalogRows() {
  return js(() => [...document.body.innerText.matchAll(/\n([^\n]+)\n(?:Recommended\n)?(?:Downloaded\n)?(\d+)\+ GB RAM · ([\d.]+) (GB|MB)/g)]
    .map((m) => ({ name: m[1].trim(), ram: +m[2], size: `~${m[3]} ${m[4]}` })));
}
async function catalogMatchesDoc() {
  // The doc table's rows, parsed from the claim text, against the picker.
  const doc = CLAIMS['ai-choosing-backend-model-catalog-1'];
  const table = [...doc.matchAll(/^\| ([^|*]+?)(?: \*\([^)]*\)\*)? \| (~[\d.]+ (?:GB|MB)) \| (\d+) GB \|$/gm)]
    .map((m) => ({ name: m[1].trim(), size: m[2], ram: +m[3] }));
  const rows = await catalogRows();
  const problems = [];
  for (const t of table) {
    const r = rows.find((x) => x.name === t.name);
    if (!r) { if (!/Nomic/.test(t.name)) problems.push(`${t.name} no está en el selector`); continue; }
    if (r.ram !== t.ram) problems.push(`${t.name}: doc ${t.ram} GB, app ${r.ram} GB`);
    if (r.size !== t.size) problems.push(`${t.name}: doc ${t.size}, app ${r.size}`);
  }
  for (const r of rows) if (!table.find((t) => t.name === r.name)) problems.push(`${r.name} está en la app pero no en la tabla`);
  return ok(!problems.length && table.length, `${table.length} filas coinciden con el selector`, problems.join('; '));
}
function sqliteTables() {
  const db = path.join(DATA_DIR, 'emailops.db');
  return execFileSync('/usr/bin/sqlite3', ['-readonly', db, "select name from sqlite_master where type in ('table','view')"], { encoding: 'utf8' }).split('\n').filter(Boolean);
}

// ═══════════════════════════════ locked ══════════════════════════════════
// The runner set a main password at the end of `fresh`, then relaunched on the
// same data dir: this is what a user sees on the next start.
async function locked() {
  const s = await screen();
  await claim('priv-locking-app-1', 'al arrancar', async () => ok(/password/i.test(s) && !/Compose/.test(s.split('\n')[0] || s.slice(0, 200)),
    'la app arranca bloqueada pidiendo la contraseña', 'la app arrancó sin bloqueo'));
  await claim('start-after-wizard-first-sync-6', 'bloqueo al arrancar', async () => ok(/password/i.test(s), 'bloqueo al arrancar', 'sin bloqueo'));
  await claim('priv-locking-app-2', 'base de datos legible', async () => {
    const t = sqliteTables();
    return ok(t.includes('emails'), 'con la app bloqueada, el SQLite se lee directamente: bloquea la app, no cifra la base de datos', 'la base de datos no es legible');
  });
  await claim('trbl-app-locked-1', 'sin recuperación', async () => ok(!/forgot|reset password|recover/i.test(s), 'la pantalla de bloqueo no ofrece recuperación', 'hay un camino de recuperación'));
}

// ═══════════════════════════════ demo ════════════════════════════════════
async function demo() {
  // Filled in by the demo-phase cases below.
  await demoCases();
}
let demoCases = async () => {};
try { demoCases = (await import('./doc_claims_demo.mjs')).default({ b, js, sleep, screen, press, claim, ok, labelsVisible, bold, CLAIMS, norm, tab, closeSettings, toggles }); } catch (e) {
  if (phase === 'demo') throw e;
}

try {
  if (phase === 'fresh') await fresh();
  else if (phase === 'locked') await locked();
  else if (phase === 'demo') await demo();
  else throw new Error(`unknown phase ${phase}`);
} finally {
  write();
  await b.deleteSession().catch(() => {});
}
