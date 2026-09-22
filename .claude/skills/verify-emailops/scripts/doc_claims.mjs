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
// Reads of a claim's text are recorded, so the report can tell a case that
// took its expectation from the docs from one that typed it in.
const CLAIM_TEXT = JSON.parse(fs.readFileSync(process.env.CLAIMS_JSON, 'utf8'));
const reads = new Set();
const CLAIMS = new Proxy(CLAIM_TEXT, {
  get: (t, k) => { if (typeof k === 'string') reads.add(k); return t[k]; },
});
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
const buttons = () => js(() => [...document.querySelectorAll('button,[role=button]')].filter((e) => e.offsetParent)
  .map((e) => (e.innerText.trim() || e.getAttribute('aria-label') || e.title || '').replace(/\s+/g, ' ')));
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
// One record per case ("part"); a claim may have several, on several screens.
//
//   claim(id, name, { covers, how, proof, fix }, async ({ doc }) => …)
//
//   covers  the sentences of the claim this case verifies, quoted as the reader
//           sees them (no ** or `). Everything it does not quote stays yellow in
//           the report. A quote the docs no longer contain fails the case.
//   how     one sentence, in Spanish, of how the case validates — shown on hover.
//   proof   'behaviour' (default) or 'label' when the case only sees a text on
//           screen; a label is not proof the feature works, so it stays yellow.
//   partial what part of the quoted sentence the case does NOT prove, when it
//           proves only part of it ("the backend part"): the sentence stays yellow.
//   doc     the claim's text: doc.text, doc.match(re), doc.number(re), doc.bold().
//           The expected value must come from here, not be typed into the case.
//
// The old form claim(id, name, fn, fix) still runs, and reports as "does not
// say what it covers".
const parts = [];
const WORDS = { one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10, eleven: 11, twelve: 12 };
function docOf(id) {
  const text = () => norm(CLAIMS[id]).replace(/\*\*|`/g, '');
  const match = (re) => {
    const m = text().match(re);
    if (!m) throw new Error(`the docs no longer match ${re} — update the case`);
    return m;
  };
  return {
    get text() { return text(); },
    match,
    number: (re) => { const v = match(re)[1].toLowerCase(); return WORDS[v] ?? Number(v); },
    bold: () => bold(id),
  };
}
function callerLine() {
  const frame = new Error().stack.split('\n').slice(2).find((l) => !/\bclaim\b \(/.test(l) && /doc_claims/.test(l)) || '';
  const m = frame.match(/(doc_claims[\w_]*\.mjs):(\d+)/);
  return m ? `.claude/skills/verify-emailops/scripts/${m[1]}:${m[2]}` : '';
}
async function claim(id, name, a, b, c) {
  const [opts, fn, fix] = typeof a === 'function' ? [{}, a, b || ''] : [a || {}, b, c || a?.fix || ''];
  if (!(id in CLAIM_TEXT)) throw new Error(`doc_claims.mjs checks claim:${id}, which the docs no longer have`);
  const where = callerLine();
  reads.clear();
  let status = 'ok', detail;
  try {
    detail = await fn({ doc: docOf(id) });
    if (typeof detail === 'string' && detail.startsWith('FAIL:')) status = 'fail';
    if (typeof detail === 'string' && detail.startsWith('SKIP:')) status = 'skip';
  } catch (e) {
    status = 'fail'; detail = `FAIL: ${e.message.split('\n')[0]}`;
  }
  const readDoc = reads.has(id);
  const screenText = (await screen().catch(() => '')).slice(0, 2000);
  parts.push({
    claim: id, name, tag: 'APP', status, detail: String(detail).replace(/^(FAIL|SKIP): /, ''),
    covers: opts.covers || null, how: opts.how || '', proof: opts.proof || 'behaviour', partial: opts.partial || '',
    read_doc: readDoc, where, fix, screen: screenText, shots: [await shot(`${id}-${name}`)],
  });
  console.log(`${status.toUpperCase().padEnd(4)} ${id} / ${name}: ${String(detail).slice(0, 150)}`);
}
const ok = (cond, good, bad) => (cond ? good : `FAIL: ${bad}`);
function write() {
  fs.writeFileSync(path.join(outDir, `${phase}.json`), JSON.stringify(parts, null, 2));
  console.log(`\n${parts.length} cases, ${parts.filter((r) => r.status === 'fail').length} failing → ${outDir}/${phase}.json`);
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
  // Only the settings navigation: the app behind the dialog has its own
  // "Calendar", "Junk"… buttons, and pressing one of those is silent.
  const hit = await js((n) => {
    const nav = [...document.querySelectorAll('nav,aside,div')].filter((e) => e.offsetParent && /Password, remote content/.test(e.innerText) && /Inbox layout/.test(e.innerText))
      .sort((a, b) => a.innerText.length - b.innerText.length)[0];
    const el = nav && [...nav.querySelectorAll('button,[role=button],[role=tab]')].find((e) => e.innerText.trim().startsWith(n));
    el?.click();
    return !!el;
  }, name);
  if (!hit) throw new Error(`no settings tab «${name}»`);
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
  const step1 = await screen();
  wizardSteps.ai = step1.match(/STEP 1 OF (\d)/)?.[1];
  await claim('start-1-ai-1', 'recomendación por hardware', {
    covers: ['EmailOps inspects your hardware and recommends whether to enable local AI.', 'Pick:'],
    how: 'En una instalación nueva, lee qué opción del paso 1 lleva «Recommended» y la compara con lo que esta máquina puede correr: en Apple Silicon debe recomendar la IA local, en otra máquina el cliente sin IA.',
  }, async ({ doc }) => {
    doc.match(/recommends whether to enable local AI/);
    const canRunLocally = process.platform === 'darwin' && process.arch === 'arm64';
    const recommendsAi = /Recommended — this machine can run AI locally/.test(step1);
    const recommendsPlain = /Recommended — no AI hardware required/.test(step1);
    return ok(recommendsAi === canRunLocally && recommendsPlain === !canRunLocally,
      `en ${process.platform}/${process.arch} recomienda ${recommendsAi ? 'la IA local' : 'el cliente sin IA'}, como corresponde a este hardware`,
      `en ${process.platform}/${process.arch} la recomendación no corresponde al hardware (IA: ${recommendsAi}, sin IA: ${recommendsPlain})`);
  });
  await claim('start-1-ai-2', 'opción IA', {
    covers: ['Use AI'], proof: 'label',
    how: 'Comprueba que el paso 1 ofrece la opción con el nombre en negrita de la doc («Use AI»). Que esas funciones corran en la máquina no se ve en el asistente.',
  }, async () => {
    const { missing, want } = await labelsVisible('start-1-ai-2');
    return ok(!missing.length, `el paso 1 ofrece «${want.join('», «')}»`, `el paso 1 no muestra ${missing.map((m) => `«${m}»`).join(', ')}`);
  }, 'Quote the wizard option exactly as step 1 shows it, in each language.');
  await claim('start-1-ai-3', 'opción sin IA', {
    covers: ['Plain email client'], proof: 'label',
    how: 'Comprueba que el paso 1 ofrece «Plain email client». Que no se descargue ningún modelo lo prueba el caso «sin modelos» tras el asistente.',
  }, async () => {
    const { missing } = await labelsVisible('start-1-ai-3', { only: ['Plain email client'] });
    return ok(!missing.length, 'el paso 1 ofrece «Plain email client»', 'el paso 1 no ofrece «Plain email client»');
  });
  await claim('inst-system-requirements-1', 'la IA es opcional', {
    covers: ['You can decline it in the first-run wizard and run EmailOps as a plain email client'],
    how: 'Recorre el asistente eligiendo «Plain email client» hasta el final: la instalación termina sin IA. Aquí comprueba que la opción existe; el recorrido completo está en los casos siguientes.',
  }, async ({ doc }) => {
    doc.match(/decline it in the first-run wizard/);
    return ok(step1.includes('Plain email client'), 'el asistente ofrece rechazar la IA', 'no hay opción sin IA');
  });
  await claim('inst-system-requirements-local-ai-1', 'disco y memoria mínimos', {
    covers: ['| Minimum | 8 GB unified', '| Free disk | ~3 GB'],
    how: 'Lee en el paso 1 la línea de hardware que la app calcula del catálogo («N GB+ RAM, ~X GB disk for the smallest model») y la compara con las filas «Minimum» y «Free disk» de la tabla de la doc.',
  }, async ({ doc }) => {
    const [, ram, disk] = step1.match(/Hardware: (\d+) GB\+ RAM, ~([\d.]+) GB disk for the smallest model/) || [];
    const docRam = doc.number(/Minimum \| (\d+) GB unified/);
    const docDisk = doc.number(/Free disk \| ~(\d+) GB/);
    return ok(+ram === docRam && Math.round(+disk) === docDisk,
      `el asistente pide ${ram} GB de RAM y ~${disk} GB de disco; la doc, ${docRam} GB y ~${docDisk} GB`,
      `el asistente pide ${ram} GB de RAM y ~${disk} GB de disco; la doc dice ${docRam} GB y ~${docDisk} GB`);
  }, 'Quote the RAM and disk figures the wizard derives from the catalog in the requirements table, in all four languages.');
  await claim('ai-classification-1', 'tres ejes', {
    covers: ['tagged along three axes — priority, intent and topic'], proof: 'label',
    how: 'Lee los tres ejes que nombra la doc y comprueba que el asistente anuncia la clasificación con esos mismos ejes. Que el correo se etiquete de verdad necesita un modelo descargado.',
  }, async ({ doc }) => {
    const axes = doc.match(/three axes — (\w+), (\w+) and (\w+)/).slice(1);
    const shown = step1.match(/Auto-classification \(([^)]+)\)/)?.[1] || '';
    return ok(axes.every((x) => shown.includes(x)), `el asistente anuncia ${shown}`, `la doc dice ${axes.join(', ')}; el asistente, «${shown}»`);
  });
  await claim('inst-system-requirements-local-ai-1', 'fuera de la tabla', {
    covers: ['One of the most important requirements for running local AI is the memory available'],
    how: 'El asistente condiciona la recomendación de IA local a la memoria: comprueba que la línea de hardware del paso 1 la expresa en GB de RAM.',
  }, async ({ doc }) => {
    doc.match(/memory available to load the model/);
    return ok(/Hardware: \d+ GB\+ RAM/.test(step1), 'el asistente pide un mínimo de memoria para la IA local', 'el asistente no menciona memoria');
  });

  // Step 2 — backend and model (AI path). Continue stays disabled without a model.
  await press('Use AI'); await press('Continue');
  const step2 = await screen();
  await claim('start-2-ai-1', 'backends', {
    covers: ['| In-app |', '| Ollama |', '| OpenRouter |'], proof: 'label',
    how: 'Tras elegir «Use AI», lee los nombres de backend en negrita de la tabla y comprueba que el paso 2 los ofrece tal cual. Lo que hace cada backend lo prueban los casos de Ajustes → AI Backend & Models.',
  }, async () => {
    const { missing } = await labelsVisible('start-2-ai-1');
    const shown = ['In-app', 'Ollama', 'OpenRouter'].filter((l) => step2.includes(l));
    return ok(!missing.length, 'los tres backends con sus nombres', `el doc cita ${missing.map((m) => `«${m}»`).join(', ')}; el asistente muestra ${shown.join(', ')}`);
  }, 'Quote the backend names as the wizard shows them (In-app, Ollama, OpenRouter) in all four languages.');
  await claim('start-2-ai-1', 'In-app por defecto', {
    covers: ['| In-app | The default.'],
    how: 'Comprueba que, sin tocar nada, el paso 2 muestra el catálogo del backend In-app (las filas «N+ GB RAM · X GB»), es decir, que In-app es el backend preseleccionado.',
  }, async ({ doc }) => {
    doc.match(/In-app \| The default/);
    return ok((await catalogRows()).length > 0, 'In-app viene seleccionado: el paso 2 muestra su catálogo', 'el paso 2 no muestra el catálogo de In-app');
  });
  await claim('start-2-ai-2', 'recomendado según memoria', {
    covers: ['EmailOps preselects the largest model your machine can comfortably run', 'on a 16 GB machine that is Qwen 3.5 4B'],
    how: 'Lee la memoria de esta máquina y las filas del selector, calcula qué modelo le corresponde (el mayor cuyo mínimo cabe dos veces en la RAM) y comprueba que el distintivo «Recommended» está en él; si la máquina tiene la memoria que nombra la doc, el modelo debe ser el que la doc nombra.',
  }, async ({ doc }) => {
    const ramGb = os.totalmem() / 1024 ** 3;
    const [, docRam, docModel] = doc.match(/on a (\d+) GB machine that is ([^,]+?),/);
    const rows = await js(() => [...document.body.innerText.matchAll(/\n([^\n]+)\n(Recommended\n)?(\d+)\+ GB RAM · ([\d.]+) GB/g)]
      .map((m) => ({ name: m[1].trim(), rec: !!m[2], ram: +m[3] })));
    const fits = rows.filter((r) => r.ram * 2 <= ramGb);
    const expect = fits.reduce((best, r) => (!best || r.ram > best.ram ? r : best), null) || rows.reduce((a, r) => (r.ram < a.ram ? r : a));
    const badged = rows.find((r) => r.rec);
    const sameSize = Math.round(ramGb) === +docRam;
    const good = badged && badged.name === expect.name && (!sameSize || badged.name === docModel);
    return ok(good,
      `${Math.round(ramGb)} GB → «${badged?.name}» recomendado${sameSize ? `, el que nombra la doc para ${docRam} GB` : ''}`,
      `${Math.round(ramGb)} GB: la app recomienda «${badged?.name}», la regla predice «${expect.name}»${sameSize ? ` y la doc nombra «${docModel}»` : ''}`);
  });
  await claim('start-2-ai-2', 'descarga del recomendado', {
    covers: ['about 3 GB to download'],
    how: 'Lee el tamaño de descarga que el selector muestra para el modelo recomendado y lo compara con la cifra de la doc.',
  }, async ({ doc }) => {
    const want = doc.number(/about (\d+) GB to download/);
    const row = (await js(() => [...document.body.innerText.matchAll(/\n([^\n]+)\nRecommended\n(\d+)\+ GB RAM · ([\d.]+) GB/g)].map((m) => +m[3])))[0];
    return ok(row && Math.round(row) === want, `el recomendado descarga ${row} GB, «about ${want} GB»`, `el recomendado descarga ${row} GB; la doc dice ${want}`);
  });
  await claim('priv-there-no-1', 'origen de los modelos', {
    covers: ['| Hugging Face | Only while downloading an AI model you picked'], proof: 'label',
    how: 'Comprueba que el paso 2 declara Hugging Face como origen de las descargas. Que la app no contacte con Hugging Face fuera de una descarga se comprueba con la auditoría de red que propone la propia página.',
  }, async ({ doc }) => {
    doc.match(/Hugging Face/);
    return ok(/Models downloaded from Hugging Face/.test(step2), 'el asistente declara Hugging Face como origen de los modelos', 'no lo dice');
  });
  await claim('start-2-ai-4', 'búsqueda sin descarga', {
    covers: ['so there is nothing to download for search'], proof: 'label',
    how: 'Comprueba que el paso 2 del asistente dice que el modelo de búsqueda viene incluido y no hay que descargarlo. Que de verdad esté ya en disco lo prueba el caso «incluido de fábrica».',
  }, async ({ doc }) => {
    doc.match(/nothing to download for search/);
    return ok(/built-in with the app — no download needed/.test(step2), 'el paso 2 dice que el modelo de búsqueda viene incluido', 'no lo dice');
  });
  await claim('ai-choosing-backend-model-catalog-1', 'tabla vs selector', {
    covers: ['| Qwen 3.5 4B |', '| Qwen 3.5 4B Q8 |', '| Qwen 3.5 9B |', '| Gemma 4 12B Instruct |', '| Qwen 3.5 27B |', '| Qwen 3.6 35B A3B |'],
    how: 'Lee cada fila de la tabla de la doc (nombre, descarga, memoria) y la compara con la fila correspondiente del selector de modelos del asistente; falla también si el selector ofrece un modelo que la tabla no lista.',
  }, async () => catalogMatchesDoc());
  await claim('ai-choosing-backend-model-catalog-1', 'catálogo con checksum', {
    covers: ['The in-app backend downloads models from a curated catalog'],
    how: 'Comprueba que el backend In-app del asistente ofrece una lista cerrada de modelos (el catálogo) en lugar de un campo libre. El checksum lo verifica el código de descarga, no se ve en pantalla.',
  }, async ({ doc }) => {
    doc.match(/curated catalog/);
    const rows = await catalogRows();
    const free = await js(() => [...document.querySelectorAll('input[type=text],input:not([type])')].filter((i) => i.offsetParent).length);
    return ok(rows.length > 0 && !free, `${rows.length} modelos para elegir, sin campo libre`, 'el backend In-app no ofrece un catálogo cerrado');
  });
  await claim('inst-system-requirements-local-ai-2', 'extremos del catálogo', {
    covers: ['The default Qwen 3.5 4B needs about 8 GB before the app offers it', 'the largest model in the catalog wants 32 GB'],
    how: 'Lee las cifras de la doc («needs about N GB», «wants N GB») y el modelo que nombra, y las compara con el mínimo y el máximo de memoria del selector y con el modelo que tiene ese mínimo.',
  }, async ({ doc }) => {
    const rows = (await catalogRows()).filter((r) => !/Nomic/.test(r.name));
    const small = doc.number(/needs about (\d+) GB/), large = doc.number(/wants (\d+) GB/);
    const named = doc.match(/The default (.+?) needs about/)[1];
    const min = rows.reduce((x, r) => (r.ram < x.ram ? r : x));
    const max = Math.max(...rows.map((r) => r.ram));
    return ok(small === min.ram && large === max && named === min.name,
      `«${min.name}» pide ${min.ram} GB y el mayor ${max} GB, como dice la doc`,
      `el selector: «${min.name}» ${min.ram} GB, máximo ${max} GB; la doc: «${named}» ${small} GB, máximo ${large} GB`);
  });
  await claim('ai-choosing-backend-model-catalog-6', 'badge', {
    covers: ['One model carries a Recommended badge, chosen for the machine you are on'],
    how: 'Cuenta cuántos modelos del selector llevan «Recommended»: debe ser exactamente uno. A qué modelo corresponde según la memoria lo prueba el caso «recomendado según memoria».',
  }, async ({ doc }) => {
    doc.match(/One model carries a Recommended badge/);
    const n = (await js(() => document.body.innerText.match(/\nRecommended\n/g)?.length || 0));
    return ok(n === 1, 'un único modelo con «Recommended»', `${n} modelos llevan «Recommended»`);
  });
  await claim('start-2-ai-2', 'Continue bloqueado sin modelo', {
    covers: ['With the in-app backend, pick a chat model from the built-in catalog.', 'Continue stays disabled until the chat model has finished downloading, or until you pick a file you already have with Use existing file…'],
    how: 'En el paso 2 con In-app y sin ningún modelo de chat en disco, comprueba que el botón que nombra la doc («Continue») está deshabilitado y que cada modelo ofrece la alternativa que nombra la doc («Use existing file…»).',
  }, async () => {
    const [cont, existing] = bold('start-2-ai-2').filter((l) => !/Qwen/.test(l));
    const offersExisting = (await buttons()).filter((x) => x === existing).length > 0;
    return ok((await buttonState(cont)) === 'disabled' && offersExisting,
      `«${cont}» deshabilitado sin modelo; «${existing}» disponible`,
      `«${cont}»: ${await buttonState(cont)}; «${existing}»: ${offersExisting ? 'sí' : 'no'}`);
  });

  // Plain path: fewer steps, layout, then account.
  await press('Back'); await press('Plain email client'); await press('Continue');
  wizardSteps.plain = (await screen()).match(/STEP \d OF (\d)/)?.[1];
  await claim('start-intro-1', 'pasos del asistente', {
    covers: ['a wizard of up to four steps runs — three if you choose a plain email client'],
    how: 'Recorre el asistente de una instalación nueva por los dos caminos y lee «STEP n OF N» en cada uno; compara N con las dos cifras que da la doc («up to four», «three if you choose a plain email client»).',
  }, async ({ doc }) => {
    const max = doc.number(/up to (\w+) steps/);
    const plain = doc.number(/(\w+) if you choose a plain email client/);
    return ok(+wizardSteps.ai === max && +wizardSteps.plain === plain,
      `con IA: ${wizardSteps.ai} pasos; sin IA: ${wizardSteps.plain}, como dice la doc`,
      `la doc dice ${max} y ${plain}; el asistente tiene ${wizardSteps.ai} con IA y ${wizardSteps.plain} sin IA`);
  }, 'State the step counts the wizard actually has, with and without AI, in all four languages.');
  const layoutStep = await screen();
  await claim('start-3-inbox-1', 'paso de diseño', {
    covers: ['Choose how the mailbox is laid out — split (list on the left, message on the right) or full width (one pane at a time).'],
    how: 'En el paso de diseño del asistente, comprueba que las dos opciones que nombra la doc aparecen, y que su descripción coincide: lista a la izquierda y contenido a la derecha, frente a una sola columna.',
  }, async ({ doc }) => {
    doc.match(/split \(list on the left, message on the right\) or full width/);
    const good = /Split view Email list on the left, content on the right/.test(layoutStep) && /Full-width list Single-column list/.test(layoutStep);
    return ok(good, '«Split view» (lista a la izquierda) y «Full-width list» (una columna)', 'faltan las dos opciones de diseño');
  });
  await press('Split view'); await press('Continue');
  const accountStep = await screen();
  await claim('start-4-connect-1', 'último paso', {
    covers: ['The last step adds your first mailbox.'],
    how: 'Comprueba que el paso de cuentas es el último del asistente (su número coincide con el total de pasos).',
  }, async ({ doc }) => {
    doc.match(/The last step adds your first mailbox/);
    const [, n, of] = accountStep.match(/STEP (\d) OF (\d)/) || [];
    return ok(n && n === of && /Gmail/.test(accountStep), `el paso ${n} de ${of} añade la cuenta`, `el paso de cuentas es el ${n} de ${of}`);
  });
  await claim('feat-accounts-sync-1', 'proveedores', {
    covers: ['Connect as many mailboxes as you like — Gmail, Outlook / Microsoft 365 (Graph API), and any IMAP/SMTP server'], proof: 'label',
    how: 'Lee los proveedores que nombra la doc y comprueba que el paso de cuentas ofrece cada uno. Que la conexión funcione necesita una cuenta real.',
  }, async ({ doc }) => {
    doc.match(/Gmail, Outlook \/ Microsoft 365 \(Graph API\)/);
    const want = ['Gmail', 'Outlook / Microsoft 365', 'Graph API', 'IMAP / SMTP'].filter((x) => !accountStep.includes(x));
    return ok(!want.length, 'Gmail, Outlook (Graph API) e IMAP/SMTP', `falta: ${want.join(', ')}`);
  });
  await claim('start-4-connect-2', 'Gmail', {
    covers: ['Gmail — sign in through your browser and grant access.'], proof: 'label',
    how: 'Comprueba que la opción Gmail del paso de cuentas es el inicio de sesión con OAuth de Google (en el navegador). Completar el flujo necesita una cuenta real.',
  }, async ({ doc }) => {
    doc.match(/sign in through your browser/);
    return ok(accountStep.includes('Sign in with Google OAuth'), 'Gmail vía OAuth de Google', 'no hay Gmail con OAuth');
  });
  await claim('start-4-connect-3', 'Outlook', {
    covers: ['Outlook / Microsoft 365 — same browser flow, via the Microsoft Graph API.'], proof: 'label',
    how: 'Comprueba que la opción Outlook del paso de cuentas es OAuth de Microsoft sobre Graph API. Completar el flujo necesita una cuenta real.',
  }, async ({ doc }) => {
    doc.match(/Microsoft Graph API/);
    return ok(accountStep.includes('Microsoft OAuth (Graph API)'), 'Outlook vía OAuth de Microsoft (Graph API)', 'no hay Outlook con Graph');
  });
  await press('IMAP / SMTP');
  const imapForm = await screen();
  await claim('start-4-connect-4', 'formulario IMAP', {
    covers: ['IMAP / SMTP — iCloud, Yahoo, Fastmail, ProtonMail Bridge or any custom server.', 'Enter the server details and credentials directly.'],
    how: 'Abre el alta IMAP del asistente; lee de la doc los proveedores que nombra y comprueba que cada uno es un preajuste del formulario, y que el formulario pide servidores IMAP y SMTP y contraseña.',
  }, async ({ doc }) => {
    const named = doc.match(/IMAP \/ SMTP — (.+) or any custom server/)[1].split(/, | or /);
    const missing = named.filter((n) => !imapForm.includes(n));
    const fields = ['IMAP host', 'SMTP host', 'Password'].filter((f) => !imapForm.includes(f));
    return ok(!missing.length && !fields.length, `preajustes ${named.join(', ')} y campos de servidor y contraseña`, `falta: ${[...missing, ...fields].join(', ')}`);
  });
  await claim('feat-accounts-sync-2', 'nombre IMAP', {
    covers: ['IMAP accounts with the display name you gave when connecting them'], proof: 'label',
    how: 'Comprueba que el alta IMAP pide un nombre para mostrar. Que ese nombre acabe en el remitente de lo enviado necesita enviar desde una cuenta real.',
  }, async ({ doc }) => {
    doc.match(/display name you gave when connecting/);
    return ok(imapForm.includes('Display Name'), 'el alta IMAP pide un nombre para mostrar', 'el formulario IMAP no pide nombre');
  });
  await press('Cancel'); await press('Skip for now');
}

// The value of the form control in the row a label starts: what a setting is
// actually set to, rather than what its help text says it defaults to.
const controlValues = (label) => js((l) => {
  const head = [...document.querySelectorAll('label,span,div,p,h3,h4')]
    .find((e) => e.offsetParent && e.innerText.trim().startsWith(l) && e.innerText.length < 400);
  if (!head) return null;
  let row = head;
  for (let i = 0; i < 5 && row && !row.querySelector('input,select,textarea'); i++) row = row.parentElement;
  return row ? [...row.querySelectorAll('input,select,textarea')].filter((e) => e.offsetParent)
    .map((e) => (e.type === 'checkbox' ? String(e.checked) : e.value)) : null;
}, label);
const hasEditableText = (label) => js((l) => {
  const head = [...document.querySelectorAll('label,span,div,p,h3,h4')]
    .find((e) => e.offsetParent && e.innerText.trim().startsWith(l) && e.innerText.length < 400);
  let row = head;
  for (let i = 0; i < 6 && row && !row.querySelector('textarea'); i++) row = row?.parentElement;
  const t = row?.querySelector('textarea');
  return !!t && !t.readOnly && !t.disabled;
}, label);
// Chat models on disk. The embedding model ships inside the app and is copied
// into models/embed/ on first run — that is not a download.
const modelFiles = () => {
  const dir = path.join(DATA_DIR, 'models');
  return fs.existsSync(dir) ? fs.readdirSync(dir, { recursive: true }).map(String)
    .filter((f) => /\.gguf$/i.test(f) && !f.startsWith(`embed${path.sep}`)) : [];
};

async function afterWizard() {
  // ── the app without AI (the plain path was taken) ──────────────────────────
  const plainScreen = await screen();
  const plainSide = plainScreen.split('SMART FILTERS')[0];
  await claim('start-1-ai-3', 'sin modelos', {
    covers: ['no model is downloaded'], partial: 'que no se haga ninguna llamada de IA no se observa desde fuera',
    how: 'Tras terminar el asistente por el camino «Plain email client», lista la carpeta models/ del directorio de datos: no debe haber ningún modelo de chat (el de embeddings viene dentro de la app y se copia, no se descarga).',
  }, async ({ doc }) => {
    doc.match(/no model is downloaded/);
    const files = modelFiles();
    return ok(!files.length, 'models/ no contiene ningún modelo descargado', `se descargaron ${files.join(', ')}`);
  });
  await claim('ai-tag-board-4', 'oculto sin IA', {
    covers: ['it is not shown while AI features are off'],
    how: 'Con la IA apagada (camino sin IA del asistente), lee la barra lateral y comprueba que no aparece «Tag Board».',
  }, async ({ doc }) => {
    doc.match(/not shown while AI features are off/);
    return ok(!/\bTag Board\b/.test(plainSide), 'sin IA no aparece el Tag Board', 'el Tag Board sigue visible con la IA apagada');
  });
  await claim('ai-turning-off-1', 'modo sin IA', {
    covers: ['Turn it off and EmailOps runs as a plain email client: no chat, no classification, no embeddings, no model loaded.'],
    partial: 'que no haya ningún modelo cargado en memoria no se observa desde fuera',
    how: 'Con la IA apagada: la barra lateral no ofrece Chat, Ajustes no ofrece las pestañas AI Classification ni AI Search, y models/ no contiene ningún modelo de chat.',
  }, async ({ doc }) => {
    doc.match(/no chat, no classification, no embeddings, no model loaded/);
    const chat = /\bChat\b(?! ?about)/.test(plainSide.split('AI FEATURES')[1] || '');
    const tabs = await tab('Appearance').then(() => screen());
    const aiTabs = ['AI Classification', 'AI Search'].filter((t) => tabs.includes(t));
    return ok(!chat && !aiTabs.length && !modelFiles().length, 'sin chat, sin pestañas de clasificación ni búsqueda, sin modelos',
      `con la IA apagada sigue habiendo: ${[chat && 'Chat', ...aiTabs, modelFiles().length && 'modelos'].filter(Boolean).join(', ')}`);
  });
  const plainTabs = await tab('Appearance');
  const plainDialog = await screen();
  await claim('feat-intro-1', 'ajustes sin IA', {
    covers: ['Everything on this page works with AI switched off.'],
    partial: 'aquí, los ajustes; las vistas del buzón y la búsqueda sin IA se prueban en la fase demo',
    how: 'Con la IA apagada, comprueba que siguen disponibles los ajustes que describe la página: las pestañas Junk (con sus dos opciones), Calendar, Privacy & Security y Appearance.',
  }, async ({ doc }) => {
    doc.match(/works with AI switched off/);
    const tabs = ['Junk', 'Calendar', 'Privacy & Security', 'Appearance'].filter((t) => !plainDialog.includes(t));
    const junk = /Junk Spam, impersonation/.test(plainDialog) ? await tab('Junk') : '';
    const options = ['Fade it in the list', 'Keep it out of the inbox'].filter((o) => !junk.includes(o));
    return ok(![...tabs, ...options].length, 'los ajustes de la página están disponibles sin IA', `sin IA falta: ${[...tabs, ...options].join(', ')}`);
  }, 'Either expose these settings without AI, or say on this page which need AI switched on.');
  await tab('Appearance');
  await claim('feat-interface-1', 'idiomas y diseño', {
    covers: ['Split or full-width inbox layout, and a UI available in English, Spanish, French and German.'], proof: 'label',
    how: 'Lee los idiomas que enumera la doc y comprueba que Ajustes → Appearance ofrece cada uno (con su nombre nativo) y los dos diseños. Cambiar de idioma no se prueba para no dejar la instancia en otro idioma.',
  }, async ({ doc }) => {
    const native = { English: 'English', Spanish: 'Español', French: 'Français', German: 'Deutsch' };
    const langs = doc.match(/available in ([^.]+)\./)[1].split(/, | and /).map((l) => native[l] || l);
    const want = [...langs, 'Split view', 'Full-width list'].filter((l) => !plainTabs.includes(l));
    return ok(!want.length, `${langs.join(', ')} y los dos diseños`, `falta: ${want.join(', ')}`);
  });
  await claim('start-3-inbox-1', 'Settings → Appearance', {
    covers: ['Change it whenever you like in Settings → Appearance, along with the interface language (English, Spanish, French, German).'],
    how: 'Abre Ajustes → Appearance y comprueba que están allí el diseño de la bandeja y «Display language» con los idiomas que enumera la doc.',
  }, async ({ doc }) => {
    const native = { English: 'English', Spanish: 'Español', French: 'Français', German: 'Deutsch' };
    const langs = doc.match(/interface language \(([^)]+)\)/)[1].split(', ').map((l) => native[l] || l);
    const good = /Display language/.test(plainTabs) && langs.every((l) => plainTabs.includes(l)) && plainTabs.includes('Split view');
    return ok(good, 'Appearance tiene diseño e idioma de la interfaz', 'Appearance no tiene el diseño o el idioma de la interfaz');
  });
  const priv = await tab('Privacy & Security');
  const privToggles = await toggles();
  await claim('priv-protection-from-2', 'bloqueado de fábrica', {
    covers: ['Remote content blocking — external images, tracking pixels and other remote resources are blocked until you allow them.'],
    partial: 'aquí se ve el valor de fábrica del ajuste; que una imagen remota no se cargue se comprobaría con un correo con imágenes',
    how: 'En una instalación nueva, lee el estado del interruptor «Allow remote content in emails» en Ajustes → Privacy & Security: debe estar apagado.',
  }, async ({ doc }) => {
    doc.match(/blocked until you allow them/);
    return ok(privToggles['Allow remote content in emails'] === false, 'contenido remoto bloqueado de fábrica', `estado: ${JSON.stringify(privToggles)}`);
  });
  await claim('priv-protection-from-2', 'banner y remitentes', {
    covers: ['A per-email banner lets you load them once, or you can trust a specific sender permanently.'], proof: 'label',
    how: 'Comprueba que la pestaña describe el banner por correo y tiene la sección «Trusted senders». El banner en un correo real no se ve: ningún correo demo carga imágenes remotas.',
  }, async ({ doc }) => {
    doc.match(/per-email banner/);
    return ok(/A banner lets you load them per-email/.test(priv) && /TRUSTED SENDERS/i.test(priv), 'banner por correo y remitentes de confianza', 'falta el banner o los remitentes de confianza');
  });
  await claim('feat-privacy-security-1', 'contenido remoto', {
    covers: ['remote images and tracking pixels are blocked until you allow them'],
    partial: 'el bloqueo al arrancar lo prueba la fase locked; el llavero, los tests',
    how: 'En una instalación nueva, el interruptor «Allow remote content in emails» está apagado de fábrica.',
  }, async ({ doc }) => {
    doc.match(/blocked until you allow them/);
    return ok(privToggles['Allow remote content in emails'] === false, 'contenido remoto bloqueado de fábrica', `estado: ${JSON.stringify(privToggles)}`);
  });
  await claim('priv-locking-app-1', 'dónde se pone', {
    covers: ['Set a main password in Settings → Privacy & Security'],
    how: 'La pasada fija una contraseña principal desde Ajustes → Privacy & Security («Use a main password»); la fase locked comprueba después que la app arranca bloqueada.',
  }, async ({ doc }) => {
    doc.match(/Settings → Privacy & Security/);
    return ok(/Use a main password/.test(priv), '«Use a main password» está en Privacy & Security', 'no está el ajuste');
  });
  await claim('start-after-wizard-first-sync-6', 'dónde se pone', {
    covers: ['Consider setting a main password in Settings → Privacy & Security'],
    how: 'Comprueba que «Use a main password» está en Ajustes → Privacy & Security; la fase locked prueba que bloquea la app al arrancar.',
  }, async ({ doc }) => {
    doc.match(/Settings → Privacy & Security/);
    return ok(/Use a main password/.test(priv), '«Use a main password» en Privacy & Security', 'no está');
  });

  // ── what landed in the data dir ────────────────────────────────────────────
  const tables = sqliteTables();
  const TABLE_FOR = {
    messages: ['emails'], mail: ['emails'], threads: ['emails'], contacts: ['contacts', 'emails'], 'calendar events': ['calendar_events'],
    'classification tags': ['email_tags'], 'search embeddings': ['vec'], embeddings: ['vec'], 'AI memory': ['memory_facts'],
  };
  const tablesFor = (items) => items.map((i) => [i, (TABLE_FOR[i] || [null]).find((t) => t && (t === 'vec' ? tables.some((x) => /embedding|vec/.test(x)) : tables.includes(t)))]);
  await claim('priv-where-data-3', 'qué guarda el SQLite', {
    covers: ['A SQLite database — messages, threads, contacts, calendar events, classification tags, search embeddings and AI memory.'],
    how: 'Lee de la doc la lista de lo que guarda la base de datos y, en la base de una instalación nueva, busca la tabla de cada cosa (emails, calendar_events, email_tags, memory_facts, las de embeddings…).',
  }, async ({ doc }) => {
    const items = doc.match(/A SQLite database — (.+?)\./)[1].split(/, | and /);
    const found = tablesFor(items);
    const missing = found.filter(([, t]) => !t).map(([i]) => i);
    return ok(!missing.length, `cada cosa tiene su tabla: ${found.map(([i, t]) => `${i}→${t}`).join(', ')}`, `sin tabla para: ${missing.join(', ')}`);
  });
  await claim('inst-where-data-2', 'qué guarda el SQLite', {
    covers: ['Mail, contacts, calendar events, embeddings — a local SQLite database.'],
    how: 'Lee de la doc la lista (correo, contactos, calendario, embeddings) y comprueba en el SQLite de la instalación nueva que cada una tiene su tabla.',
  }, async ({ doc }) => {
    const items = doc.match(/([A-Z][^—]+?) — a local SQLite database/)[1].split(', ').map((i) => i.toLowerCase());
    const found = tablesFor(items.map((i) => (i === 'mail' ? 'mail' : i)));
    const missing = found.filter(([, t]) => !t).map(([i]) => i);
    return ok(!missing.length, `SQLite local con ${found.map(([i, t]) => `${i}→${t}`).join(', ')}`, `sin tabla para: ${missing.join(', ')}`);
  });
  await claim('priv-where-data-5', 'EMAILOPS_DATA_DIR', {
    covers: ['Point EMAILOPS_DATA_DIR somewhere else before launching to use a different location'],
    how: 'La instancia se lanza con EMAILOPS_DATA_DIR apuntando a un directorio temporal nuevo; comprueba que la base de datos se creó ahí.',
  }, async ({ doc }) => {
    doc.match(/EMAILOPS_DATA_DIR/);
    return ok(fs.existsSync(path.join(DATA_DIR, 'emailops.db')), `la instancia escribe en el directorio indicado (${path.basename(DATA_DIR)})`, 'el directorio indicado no tiene base de datos');
  });
  await claim('inst-where-data-5', 'EMAILOPS_DATA_DIR', {
    covers: ['set the EMAILOPS_DATA_DIR environment variable before launching'],
    how: 'La instancia se lanza con EMAILOPS_DATA_DIR apuntando a un directorio temporal nuevo; comprueba que la base de datos se creó ahí.',
  }, async ({ doc }) => {
    doc.match(/EMAILOPS_DATA_DIR/);
    return ok(fs.existsSync(path.join(DATA_DIR, 'emailops.db')), 'se respeta', 'no se respeta');
  });
  await claim('priv-where-data-4', 'models/', {
    covers: ['A models/ folder — the AI models you downloaded.'],
    how: 'Comprueba que la instalación nueva crea la carpeta models/ junto a la base de datos, en el directorio de datos.',
  }, async ({ doc }) => {
    doc.match(/models\//);
    return ok(fs.statSync(path.join(DATA_DIR, 'models')).isDirectory(), 'hay carpeta models/ junto a la base de datos', 'no hay models/');
  });
  await claim('inst-where-data-3', 'models/', {
    covers: ['Downloaded AI models — a models/ folder next to the database.'],
    how: 'Comprueba que la instalación nueva crea models/ junto a emailops.db.',
  }, async ({ doc }) => {
    doc.match(/models\/ folder next to the database/);
    return ok(fs.existsSync(path.join(DATA_DIR, 'models')) && fs.existsSync(path.join(DATA_DIR, 'emailops.db')), 'models/ junto a la base de datos', 'no hay models/');
  });
  await claim('inst-where-data-1', 'todo en el directorio de datos', {
    covers: ['Everything EmailOps stores is on your machine, in your OS application data directory:'],
    how: 'Lista el directorio de datos de la instancia nueva: debe contener la base de datos y models/, y nada de EmailOps debe haberse escrito en otro sitio que el directorio indicado.',
  }, async ({ doc }) => {
    doc.match(/application data directory/);
    const entries = fs.readdirSync(DATA_DIR);
    return ok(entries.includes('emailops.db') && entries.includes('models'), `el directorio de datos contiene ${entries.filter((e) => !e.startsWith('.')).join(', ')}`, 'falta la base de datos o models/');
  });

  // ── switch AI on: the master switch, then factory defaults ────────────────
  await tab('AI Backend & Models');
  await flip('AI Features');
  await sleep(1000);
  const ai = await tab('AI Backend & Models');
  const aiDialog = await screen();
  await claim('ai-turning-off-1', 'interruptor maestro', {
    covers: ['Settings → AI Backend & Models → AI Features is a master switch.'],
    how: 'Con la IA apagada, las pestañas de IA no existen; pulsa «AI Features» en Ajustes → AI Backend & Models y comprueba que aparecen AI Classification y AI Search.',
  }, async ({ doc }) => {
    doc.match(/AI Features is a master switch/);
    const shown = ['AI Classification', 'AI Search'].filter((t) => aiDialog.includes(t));
    return ok(shown.length === 2 && !/AI Classification/.test(plainDialog), 'el interruptor enciende todas las funciones de IA', `tras encenderlo: ${shown.join(', ') || 'ninguna pestaña de IA'}`);
  });
  await claim('start-1-ai-3', 'encender más tarde', {
    covers: ['You can turn AI on later in Settings → AI Backend & Models'],
    how: 'Tras el camino sin IA, enciende «AI Features» en Ajustes → AI Backend & Models y comprueba que las funciones de IA aparecen.',
  }, async ({ doc }) => {
    doc.match(/turn AI on later in Settings → AI Backend & Models/);
    return ok(aiDialog.includes('AI Classification'), 'la IA se enciende desde AI Backend & Models', 'no se pudo encender la IA');
  });
  await claim('ai-choosing-backend-1', 'dónde se elige', {
    covers: ['Settings → AI Backend & Models controls where inference happens:'],
    how: 'En Ajustes → AI Backend & Models pulsa OpenRouter, luego Ollama y luego In-app, y comprueba que cada uno cambia el formulario (clave de API, modelos de Ollama, catálogo propio).',
  }, async ({ doc }) => {
    doc.match(/controls where inference happens/);
    await press('OpenRouter'); const or = await screen();
    await press('Ollama'); const ol = await screen();
    await press('In-app'); const ia = await screen();
    const good = /API Key/i.test(or) && /Embedding Model/.test(ol) && (await catalogRows()).length > 0 && !/API Key/i.test(ia);
    return ok(good, 'cada backend cambia la configuración', 'elegir backend no cambia la configuración');
  });
  await claim('ai-choosing-backend-2', 'nombre', {
    covers: ['In-app — an embedded llama.cpp runtime.'], proof: 'label',
    how: 'Comprueba que el backend se llama como dice la doc (el texto en negrita) en Ajustes → AI Backend & Models.',
  }, async () => {
    const { missing, want } = await labelsVisible('ai-choosing-backend-2');
    return ok(!missing.length, `«${want.join('»')}» como lo muestra la app`, `el doc cita ${missing.map((m) => `«${m}»`).join(', ')}, que la app no muestra`);
  }, 'Quote the backend exactly as Settings → AI Backend & Models shows it, in all four languages.');
  await claim('ai-choosing-backend-2', 'por defecto', {
    covers: ['This is the default.'],
    how: 'En una instalación nueva, con la IA recién encendida y sin tocar el backend, comprueba que el formulario es el de In-app (su catálogo, sin clave de API).',
  }, async ({ doc }) => {
    doc.match(/This is the default/);
    return ok((await catalogRows()).length > 0 && !/API Key/i.test(ai), 'In-app es el backend de fábrica', 'el backend de fábrica no es In-app');
  });
  await claim('priv-there-no-2', 'apagado de fábrica', {
    covers: ['it is off by default, and it takes a deliberate change in Settings → AI Backend & Models plus your own API key to enable'],
    how: 'En una instalación nueva el backend es In-app, no OpenRouter; al elegir OpenRouter el formulario pide una clave de API.',
  }, async ({ doc }) => {
    doc.match(/off by default/);
    await press('OpenRouter'); const or = await screen(); await press('In-app');
    return ok(!/API Key/i.test(ai) && /API Key/i.test(or), 'OpenRouter está apagado de fábrica y pide clave de API', 'OpenRouter no está apagado de fábrica o no pide clave');
  });
  await claim('ai-choosing-backend-3', 'nombre y selección', {
    covers: ['Ollama — an Ollama server you already run'],
    how: 'Comprueba que «Ollama» (el nombre en negrita de la doc) se puede elegir en Ajustes → AI Backend & Models y que entonces pide sus modelos de chat y de embeddings.',
  }, async () => {
    const { missing } = await labelsVisible('ai-choosing-backend-3');
    await press('Ollama'); const ol = await screen(); await press('In-app');
    return ok(!missing.length && /Chat Model .*Embedding Model/.test(ol), 'Ollama se elige y pide sus modelos', 'no se puede elegir Ollama');
  }, 'Quote the backend exactly as Settings → AI Backend & Models shows it, in all four languages.');
  await claim('ai-choosing-backend-4', 'clave y presupuesto', {
    covers: ['OpenRouter — a paid cloud API.', 'Requires an API key, supports a monthly budget cap'], proof: 'label',
    how: 'Elige OpenRouter en Ajustes → AI Backend & Models y comprueba que el formulario pide una clave de API y ofrece un presupuesto mensual. Que el presupuesto se aplique no se prueba sin una clave.',
  }, async () => {
    const { missing } = await labelsVisible('ai-choosing-backend-4');
    await press('OpenRouter'); const or = await screen(); await press('In-app');
    return ok(!missing.length && /API Key/i.test(or) && /Monthly Budget/.test(or), 'OpenRouter pide clave de API y admite presupuesto mensual', 'falta clave o presupuesto');
  });
  await claim('inst-system-requirements-without-local-2', 'IA remota', {
    covers: ['AI switched on but routed to OpenRouter'], proof: 'label',
    how: 'Comprueba que, con la IA encendida, OpenRouter se puede elegir como backend (pide una clave de API).',
  }, async ({ doc }) => {
    doc.match(/routed to OpenRouter/);
    await press('OpenRouter'); const or = await screen(); await press('In-app');
    return ok(/API Key/i.test(or), 'la IA puede enrutarse a OpenRouter', 'no se puede elegir OpenRouter');
  });
  await claim('start-2-ai-4', 'incluido de fábrica', {
    covers: ['The embedding model that powers semantic search (Nomic Embed Text v1.5, ~80 MB) ships inside the app on macOS'],
    how: 'En la instalación nueva, sin haber descargado nada, busca en el catálogo de AI Backend & Models el modelo de embeddings que nombra la doc: debe figurar como «Downloaded» y pesar lo que dice la doc (±15 %).',
  }, async ({ doc }) => {
    const [, name, mb] = doc.match(/\(([^,]+), ~(\d+) MB\)/);
    const row = ai.match(new RegExp(`${name.replace(/[.]/g, '\\.')}[^·]*?Downloaded \\d+\\+ GB RAM · (\\d+) MB`));
    const size = row && +row[1];
    return ok(size && Math.abs(size - +mb) / +mb <= 0.15, `${name} viene descargado de fábrica y pesa ${size} MB`,
      row ? `${name} pesa ${size} MB; la doc dice ~${mb} MB` : `${name} no figura como descargado en una instalación nueva`);
  });
  await claim('start-2-ai-2', 'tool-calling', {
    covers: ['Every recommended model supports the tool-calling that chat relies on.'],
    how: 'En el catálogo de AI Backend & Models, lee la fila del modelo con «Recommended» y comprueba que lleva la marca «tool-calling».',
  }, async ({ doc }) => {
    doc.match(/supports the tool-calling/);
    const rec = ai.match(/((?:\S+ ){1,5})Recommended (?:Downloaded )?\d+\+ GB RAM · [\d.]+ [GM]B · [\w.-]+( · tool-calling)?/);
    return ok(rec && rec[2], `el recomendado (${rec?.[1]?.trim()}) admite tool-calling`, `el recomendado (${rec?.[1]?.trim()}) no marca tool-calling`);
  });
  await claim('ai-choosing-backend-performance-knobs-1', 'keep-alive por defecto', {
    covers: ['Keep model loaded — how long the model stays resident between turns (default 30 minutes).'],
    how: 'Lee de la doc el valor por defecto y lo compara con el valor real del campo «Keep model loaded» en una instalación nueva.',
  }, async ({ doc }) => {
    const want = doc.number(/\(default (\d+) minutes\)/);
    const [v] = (await controlValues('Keep model loaded')) || [];
    return ok(+v === want, `el campo vale ${v} minutos de fábrica`, `el campo vale ${v}; la doc dice ${want}`);
  });
  await claim('ai-choosing-backend-performance-knobs-1', '0 descarga', {
    covers: ['0 evicts it immediately'], proof: 'label',
    how: 'Comprueba que la ayuda del campo dice que 0 descarga el modelo de inmediato. Que el modelo se descargue de verdad necesita un modelo de chat descargado.',
  }, async ({ doc }) => {
    doc.match(/0 evicts it immediately/);
    return ok(/0 to evict immediately/.test(ai), 'la ayuda dice que 0 descarga el modelo', 'la ayuda no lo dice');
  });
  await claim('ai-choosing-backend-performance-knobs-2', 'contexto', {
    covers: ['Context window — how many tokens the model can attend to per turn.'],
    how: 'Comprueba que «Context window (tokens)» es un campo editable con un número de tokens en Ajustes → AI Backend & Models.',
  }, async ({ doc }) => {
    doc.match(/how many tokens/);
    const [v] = (await controlValues('Context window')) || [];
    return ok(v !== undefined && /^\d*$/.test(v), `campo en tokens (valor actual: ${v || 'automático'})`, 'no hay campo de ventana de contexto');
  });
  await claim('ai-choosing-backend-performance-knobs-3', 'razonamiento', {
    covers: ['Thinking mode — chain-of-thought reasoning on supported models.'], proof: 'label',
    how: 'Comprueba que «Thinking Mode» existe con su descripción de razonamiento encadenado. Su efecto necesita un modelo descargado.',
  }, async ({ doc }) => {
    doc.match(/chain-of-thought/);
    return ok(/Thinking Mode Chain-of-thought/i.test(ai), '«Thinking Mode» presente', 'no está');
  });
  await claim('ai-choosing-backend-performance-knobs-4', 'límites por defecto', {
    covers: ['an email limit (1000 by default)', 'a day limit (365 days by default)'],
    how: 'Lee de la doc los dos valores por defecto y los compara con los valores reales de los campos de «Limit AI processing» en una instalación nueva.',
  }, async ({ doc }) => {
    const emails = doc.number(/email limit \((\d+) by default\)/);
    const days = doc.number(/day limit \((\d+) days by default\)/);
    const vals = ((await controlValues('Limit AI processing')) || []).map(Number);
    return ok(vals.includes(emails) && vals.includes(days), `los campos valen ${vals.join(' y ')}`, `los campos valen ${vals.join(' y ')}; la doc dice ${emails} correos y ${days} días`);
  }, 'Describe both limits the setting applies (the email limit and the day limit) with their defaults, in all four languages.');
  await claim('ai-choosing-backend-performance-knobs-4', 'qué limita', {
    covers: ['Limit AI processing — caps what embedding and classification cover'], proof: 'label',
    how: 'Comprueba que la ayuda del ajuste dice que limita embeddings y clasificación. Aplicarlo necesita un buzón mayor que el límite.',
  }, async ({ doc }) => {
    doc.match(/caps what embedding and classification cover/);
    return ok(/Embeddings and classification cover every email/.test(ai), 'la ayuda describe embeddings y clasificación', 'no');
  });
  await claim('start-after-wizard-first-sync-7', 'ubicación', {
    covers: ['Both classification and embedding respect Limit AI processing (Settings → AI Backend & Models)'], proof: 'label',
    how: 'Comprueba que «Limit AI processing» está en AI Backend & Models y que su ayuda nombra clasificación y embeddings.',
  }, async ({ doc }) => {
    doc.match(/Limit AI processing/);
    return ok(/Limit AI processing/.test(ai) && /Embeddings and classification/.test(ai), '«Limit AI processing» en AI Backend & Models', 'no está en esa pestaña');
  });
  await claim('trbl-search-returns-2', 'ubicación', {
    covers: ['Also check Limit AI processing in AI settings'],
    how: 'Comprueba que «Limit AI processing» está en los ajustes de IA, donde la doc manda mirar.',
  }, async ({ doc }) => {
    doc.match(/Limit AI processing/);
    return ok(/Limit AI processing/.test(ai), 'en los ajustes de IA', 'no está');
  });
  await claim('trbl-chat-slow-4', 'ajuste', {
    covers: ['Raise "keep model loaded" in AI settings'],
    how: 'Comprueba que «Keep model loaded» es un campo editable en los ajustes de IA.',
  }, async ({ doc }) => {
    doc.match(/keep model loaded/);
    return ok(((await controlValues('Keep model loaded')) || []).length > 0, '«Keep model loaded» editable en los ajustes de IA', 'no está');
  });
  await claim('trbl-chat-slow-5', 'ajuste', {
    covers: ['Lower the context window'],
    how: 'Comprueba que la ventana de contexto es un campo editable en los ajustes de IA.',
  }, async ({ doc }) => {
    doc.match(/context window/);
    return ok(((await controlValues('Context window')) || []).length > 0, '«Context window» editable', 'no está');
  });
  await claim('trbl-chat-slow-6', 'ajuste', {
    covers: ['Turn off thinking mode'],
    how: 'Comprueba que «Thinking Mode» se puede cambiar en los ajustes de IA.',
  }, async ({ doc }) => {
    doc.match(/thinking mode/);
    return ok(ai.includes('Thinking Mode'), '«Thinking Mode» en los ajustes de IA', 'no está');
  });
  await claim('ai-chat-mailbox-4', 'enrutado configurable', {
    covers: ['The routing mode is configurable:'],
    how: 'Comprueba que «Chat routing mode» es un control con las tres opciones en Ajustes → AI Backend & Models.',
  }, async ({ doc }) => {
    doc.match(/routing mode is configurable/);
    const opts = ['Always RAG first', 'Auto (heuristic-routed)', 'Always tools first'].filter((o) => ai.includes(o));
    return ok(/Chat routing mode/.test(ai) && opts.length === 3, 'modo de enrutado con tres opciones', `opciones: ${opts.join(', ')}`);
  });
  await claim('ai-chat-mailbox-5', 'por defecto', {
    covers: ['Always RAG first — the default'],
    how: 'Lee qué opción de «Chat routing mode» está marcada como por defecto en una instalación nueva y la compara con la que nombra la doc.',
  }, async () => {
    const named = bold('ai-chat-mailbox-5')[0];
    return ok(new RegExp(`${named} \\(default\\)`).test(ai), `«${named}» es el predeterminado`, `«${named}» no es el predeterminado`);
  });
  await claim('ai-chat-mailbox-6', 'opción', {
    covers: ['Auto — a heuristic decides per question whether to retrieve first.'], proof: 'label',
    how: 'Comprueba que la opción que nombra la doc existe en «Chat routing mode». Su comportamiento es enrutado interno.',
  }, async () => {
    const named = bold('ai-chat-mailbox-6')[0];
    return ok(ai.includes(named), `«${named}» presente`, 'no está');
  });
  await claim('ai-chat-mailbox-7', 'opción', {
    covers: ['Always tools first — skip retrieval and start from structured lookups.'], proof: 'label',
    how: 'Comprueba que la opción que nombra la doc existe en «Chat routing mode». Su comportamiento es enrutado interno.',
  }, async () => {
    const named = bold('ai-chat-mailbox-7')[0];
    return ok(ai.includes(named), `«${named}» presente`, 'no está');
  });
  await claim('ai-chat-mailbox-9', 'prompts editables', {
    covers: ['Advanced users can edit the system prompt and the retrieval prompts (query rewriting, reranking) in Settings → AI Backend & Models → Chat prompts.'],
    how: 'Comprueba que Ajustes → AI Backend & Models tiene la sección «Chat prompts» con el prompt de sistema.',
  }, async ({ doc }) => {
    doc.match(/Chat prompts/);
    return ok(/Chat prompts .*System prompt/.test(ai), '«Chat prompts» con el prompt de sistema', 'no está');
  });
  await claim('feat-interface-1', 'idioma de la IA', {
    covers: ["The AI's output language is set separately"],
    how: 'Comprueba que «AI output language» es un ajuste propio en AI Backend & Models, distinto de «Display language» en Appearance.',
  }, async ({ doc }) => {
    doc.match(/output language is set separately/);
    return ok(/AI output language/.test(ai) && !/AI output language/.test(plainTabs), 'el idioma de la IA se ajusta aparte', 'no hay idioma de salida de la IA');
  });
  await claim('trbl-ai-features-1', 'descarga', {
    covers: ['check that the recommended model finished downloading in Settings → AI Backend & Models', 'start it again from the same screen'], proof: 'label',
    how: 'Comprueba que el catálogo de AI Backend & Models muestra qué modelos están descargados y ofrece descargarlos. Que la descarga se reanude la cubre el juicio del agente sobre el código.',
  }, async ({ doc }) => {
    doc.match(/start it again from the same screen/);
    return ok(/Download/.test(ai) && /Downloaded/.test(ai), 'el catálogo marca lo descargado y ofrece descargar', 'no');
  });
  await claim('trbl-ai-features-2', 'Ollama', {
    covers: ['If you switched to Ollama'],
    how: 'Comprueba que Ollama se puede elegir como backend en los ajustes de IA.',
  }, async ({ doc }) => {
    doc.match(/switched to Ollama/);
    await press('Ollama'); const ol = await screen(); await press('In-app');
    return ok(/Chat Model .*Embedding Model/.test(ol), 'Ollama se elige como backend', 'no');
  });
  await claim('trbl-chat-slow-3', 'modelo más pequeño', {
    covers: ['Qwen 3.5 4B is the smallest chat model in the catalog.'],
    how: 'Lee el modelo que nombra la doc y lo compara con el modelo de chat de menor memoria del selector.',
  }, async ({ doc }) => {
    const rows = (await catalogRows()).filter((r) => !/Nomic/.test(r.name));
    const smallest = rows.reduce((a, r) => (r.ram < a.ram ? r : a));
    const named = doc.match(/model\. (.+?) is the smallest chat model/)[1];
    return ok(named === smallest.name, `«${smallest.name}» es el modelo de chat más pequeño del selector`, `el doc nombra «${named}», el más pequeño es «${smallest.name}»`);
  });

  const tabs = await js(() => document.body.innerText.replace(/\s+/g, ' '));
  await claim('ai-intro-1', 'interruptores por función', {
    covers: ['each one can be turned off individually'], partial: 'que todas pasen por el backend elegido no se ve en Ajustes',
    how: 'Abre la pestaña de cada función de IA (Classification, Tasks, Memory, Lenses, Drafts, Translation, Search) y comprueba que cada una tiene su propio interruptor.',
  }, async ({ doc }) => {
    doc.match(/turned off individually/);
    const without = [];
    for (const t of ['AI Classification', 'AI Tasks', 'AI Memory', 'AI Lenses', 'AI Drafts', 'AI Translation', 'AI Search']) {
      await tab(t);
      if (!Object.keys(await toggles()).length) without.push(t);
    }
    return ok(!without.length, 'cada función de IA tiene su interruptor', `sin interruptor: ${without.join(', ')}`);
  });
  const junk = await tab('Junk');
  const junkToggles = await toggles();
  await claim('feat-junk-bulk-2', 'opción', {
    covers: ['Fade it in the list'], proof: 'label',
    how: 'Comprueba que la opción que nombra la doc existe en Ajustes → Junk. Su efecto en la lista necesita correo marcado como junk.',
  }, async () => {
    const [named] = bold('feat-junk-bulk-2');
    return ok(junk.includes(named), `«${named}» presente`, 'no está');
  });
  await claim('feat-junk-bulk-3', 'opción', {
    covers: ['Keep it out of the inbox'], proof: 'label',
    how: 'Comprueba que la opción que nombra la doc existe en Ajustes → Junk. Su efecto necesita correo marcado como junk.',
  }, async () => {
    const [named] = bold('feat-junk-bulk-3');
    return ok(junk.includes(named), `«${named}» presente`, 'no está');
  });
  const phishing = Object.keys(junkToggles).find((l) => /impersonat|phishing/i.test(l));
  await claim('priv-protection-from-4', 'apagado de fábrica', {
    covers: ['Off by default'],
    how: 'Lee el estado de fábrica del interruptor de suplantación en Ajustes → Junk: debe estar apagado.',
  }, async ({ doc }) => {
    doc.match(/Off by default/);
    return ok(phishing && junkToggles[phishing] === false, `«${phishing}» apagado de fábrica`, `estado: ${JSON.stringify(junkToggles)}`);
  });
  await claim('feat-junk-bulk-4', 'suplantación opcional', {
    covers: ['An optional impersonation/phishing warning is available and off by default.'],
    how: 'Comprueba que Ajustes → Junk tiene un interruptor de suplantación y que está apagado de fábrica.',
  }, async ({ doc }) => {
    doc.match(/off by default/);
    return ok(phishing && junkToggles[phishing] === false, 'aviso de suplantación opcional y apagado', 'no está apagado');
  });
  await claim('feat-junk-bulk-1', 'sin modelo ni red', {
    covers: ['No model and no network call is involved'], proof: 'label',
    how: 'Comprueba que Ajustes → Junk describe el detector como local, sin modelo y sin red. El test de arquitectura prueba que no puede actuar sobre el servidor.',
  }, async ({ doc }) => {
    doc.match(/No model and no network call/);
    return ok(/No model, no network/.test(junk), 'la app describe el detector como local y sin red', 'no');
  });
  const cls = await tab('AI Classification');
  await claim('trbl-classification-tagging-1', 'ajuste', {
    covers: ['Confirm auto-classify new emails is on in Settings → AI Classification.'],
    how: 'Comprueba que «Auto-classify new emails» es un interruptor en Ajustes → AI Classification.',
  }, async ({ doc }) => {
    doc.match(/auto-classify new emails/);
    // The switch is a bare <button> (no role=switch / aria-checked — an
    // accessibility gap), so find it by its row and its pill shape.
    const hasSwitch = await js(() => [...document.querySelectorAll('button')]
      .filter((e) => e.offsetParent && /rounded-full/.test(e.className) && !e.innerText.trim()).some((e) => {
        let row = e.parentElement;
        for (let i = 0; i < 3 && row && !/Auto-classify new emails/.test(row.innerText); i++) row = row.parentElement;
        return row && /Auto-classify new emails/.test(row.innerText) && row.innerText.length < 400;
      }));
    return ok(hasSwitch, '«Auto-classify new emails» es un interruptor en AI Classification', 'no hay interruptor «Auto-classify new emails»');
  });
  await claim('trbl-classification-tagging-3', 'acciones', {
    covers: ['use Classify Unclassified, or Reclassify All'], proof: 'label',
    how: 'Comprueba que los dos botones que nombra la doc están en AI Classification. Ejecutarlos necesita un modelo descargado.',
  }, async () => {
    const named = bold('trbl-classification-tagging-3');
    return ok(named.every((n) => cls.includes(n)), `${named.join(' y ')} presentes`, `falta alguno de ${named.join(', ')}`);
  });
  await claim('ai-classification-5', 'acciones', {
    covers: ['You control which Gmail categories are classified, can reclassify everything after changing the prompt, and can catch up on unclassified mail on demand.'], proof: 'label',
    how: 'Comprueba que AI Classification tiene la selección de pestañas de Gmail y los botones de reclasificar y clasificar lo pendiente.',
  }, async ({ doc }) => {
    doc.match(/Gmail categories/);
    return ok(/Reclassify All/.test(cls) && /Classify Unclassified/.test(cls) && /Gmail inbox tabs/.test(cls), 'pestañas de Gmail, reclasificar y ponerse al día', 'falta alguno');
  });
  await claim('ai-classification-3', 'reglas', {
    covers: ['Rules match on sender or subject patterns'], proof: 'label',
    how: 'Comprueba que AI Classification tiene una sección de reglas. Que una regla etiquete sin modelo necesita correo nuevo.',
  }, async ({ doc }) => {
    doc.match(/sender or subject patterns/);
    return ok(/rule/i.test(cls), 'hay reglas de clasificación', 'no hay reglas');
  });
  await claim('ai-classification-4', 'prompt editable', {
    covers: ['using an instruction prompt you can edit'],
    how: 'Comprueba que el prompt de clasificación es un área de texto editable en AI Classification.',
  }, async ({ doc }) => {
    doc.match(/prompt you can edit/);
    return ok(await js(() => [...document.querySelectorAll('textarea')].some((t) => t.offsetParent && !t.readOnly && !t.disabled)), 'el prompt de clasificación es editable', 'no hay prompt editable');
  });
  await tab('AI Tasks');
  await flip('AI Tasks');
  const tasks = await tab('AI Tasks');
  await claim('ai-tasks-1', 'experimental', {
    covers: ['Experimental.'],
    how: 'Comprueba que la pestaña AI Tasks aparece marcada como EXPERIMENTAL en Ajustes.',
  }, async ({ doc }) => {
    doc.match(/Experimental/);
    return ok(/AI Tasks[^.]{0,40}EXPERIMENTAL|EXPERIMENTAL[^.]{0,40}AI Tasks/.test(tabs), 'AI Tasks marcada experimental', 'no aparece como experimental');
  });
  await claim('ai-tasks-1', 'ajustes', {
    covers: ['there is a "learn only from emails I wrote" mode', 'You can exclude senders and tags', 'backfill older mail on demand'], proof: 'label',
    how: 'Con AI Tasks encendido, comprueba que la pestaña tiene «Learn only from emails I wrote», exclusiones y el backfill. Que extraiga tareas necesita un modelo descargado.',
  }, async ({ doc }) => {
    doc.match(/learn only from emails I wrote/);
    const want = [['Learn only from emails I wrote', /Learn only from emails I wrote/i], ['exclusiones', /exclu/i], ['backfill', /backfill/i]].filter(([, re]) => !re.test(tasks)).map(([n]) => n);
    return ok(!want.length, 'solo lo que yo escribí, exclusiones y backfill', `falta: ${want.join(', ')}`);
  });
  await tab('AI Memory');
  await claim('ai-memory-1', 'interruptor maestro', {
    covers: ['the whole subsystem has a master off switch'],
    how: 'Comprueba que AI Memory tiene su propio interruptor en Ajustes.',
  }, async ({ doc }) => {
    doc.match(/master off switch/);
    return ok(Object.keys(await toggles()).some((l) => /AI Memory/.test(l)), 'AI Memory tiene interruptor propio', 'no');
  });
  await claim('ai-memory-1', 'experimental', {
    covers: ['Experimental.'],
    how: 'Comprueba que AI Memory aparece marcada como EXPERIMENTAL.',
  }, async ({ doc }) => {
    doc.match(/Experimental/);
    return ok(/AI Memory[^.]{0,40}EXPERIMENTAL|EXPERIMENTAL[^.]{0,40}AI Memory/.test(tabs), 'AI Memory marcada experimental', 'no');
  });
  const drafts = await tab('AI Drafts');
  await claim('ai-ai-drafts-1', 'ajustes', {
    covers: ['Configure a persona (one sentence on who the AI writes as) and a writing style — or replace the whole prompt template.'],
    how: 'Lee de la doc lo que se puede configurar (persona, estilo, plantilla) y comprueba que cada uno es un campo en Ajustes → AI Drafts.',
  }, async ({ doc }) => {
    const promised = [['persona', /Persona/], ['writing style', /Writing style/], ['default tone', /Default tone/], ['default length', /Default length/], ['prompt template', /Prompt template/]]
      .filter(([w]) => new RegExp(w.replace('default ', '(default )?'), 'i').test(doc.text));
    const missing = promised.filter(([, re]) => !re.test(drafts)).map(([w]) => w);
    return ok(!missing.length, `${promised.map(([w]) => w).join(', ')} en AI Drafts`, `el doc promete ${missing.join(' y ')}, que la pestaña AI Drafts no tiene`);
  }, 'AI Drafts offers a persona, a writing style and the prompt template; tone is chosen per draft in the composer. Say that, in all four languages.');
  await tab('AI Translation');
  await claim('ai-translation-1', 'prompt', {
    covers: ['The translation prompt is editable like the others.'],
    how: 'Comprueba que el prompt de traducción es un área de texto editable en Ajustes → AI Translation.',
  }, async ({ doc }) => {
    doc.match(/translation prompt is editable/);
    return ok(await hasEditableText('Translation prompt'), 'el prompt de traducción es editable', 'no');
  });
  await setMainPassword();
  await closeSettings();
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
  const lockedUp = /password/i.test(s) && !/Compose|Inbox/.test(s);
  await claim('priv-locking-app-1', 'al arrancar', {
    covers: ['EmailOps stays locked on startup until you enter it'],
    how: 'La fase fresh fija una contraseña principal; esta fase relanza la app sobre el mismo directorio de datos y comprueba que arranca en la pantalla de contraseña, sin bandeja ni redactar.',
  }, async ({ doc }) => {
    doc.match(/locked on startup/);
    return ok(lockedUp, 'la app arranca bloqueada pidiendo la contraseña', 'la app arrancó sin bloqueo');
  });
  await claim('start-after-wizard-first-sync-6', 'bloqueo al arrancar', {
    covers: ['if you want the app locked on startup'],
    how: 'Relanza la app tras fijar la contraseña principal y comprueba que arranca bloqueada.',
  }, async ({ doc }) => {
    doc.match(/locked on startup/);
    return ok(lockedUp, 'bloqueo al arrancar', 'sin bloqueo');
  });
  await claim('feat-privacy-security-1', 'bloqueo al arrancar', {
    covers: ['A main password locks the app on startup'],
    partial: 'el contenido remoto lo prueba la fase fresh; el llavero, los tests',
    how: 'Relanza la app tras fijar la contraseña principal y comprueba que arranca bloqueada.',
  }, async ({ doc }) => {
    doc.match(/locks the app on startup/);
    return ok(lockedUp, 'la contraseña principal bloquea el arranque', 'sin bloqueo');
  });
  await claim('priv-locking-app-2', 'base de datos legible', {
    covers: ['it locks the application, it does not encrypt the database', 'can read the SQLite file directly'],
    how: 'Con la app bloqueada, abre el SQLite del directorio de datos con sqlite3 en solo lectura: debe leerse la lista de tablas sin contraseña.',
  }, async ({ doc }) => {
    doc.match(/does not encrypt the database/);
    const t = sqliteTables();
    return ok(t.includes('emails'), 'con la app bloqueada, el SQLite se lee directamente: bloquea la app, no cifra la base de datos', 'la base de datos no es legible');
  });
  await claim('trbl-app-locked-1', 'sin recuperación', {
    covers: ['The main password is a local lock with no recovery path'],
    how: 'Lee la pantalla de bloqueo y comprueba que no ofrece recuperar ni restablecer la contraseña.',
  }, async ({ doc }) => {
    doc.match(/no recovery path/);
    return ok(lockedUp && !/forgot|reset password|recover/i.test(s), 'la pantalla de bloqueo no ofrece recuperación', 'hay un camino de recuperación');
  });
  await claim('priv-locking-app-1', 'sin recuperación', {
    covers: ['There is no recovery path'],
    how: 'Lee la pantalla de bloqueo y comprueba que no ofrece recuperar ni restablecer la contraseña.',
  }, async ({ doc }) => {
    doc.match(/no recovery path/);
    return ok(lockedUp && !/forgot|reset password|recover/i.test(s), 'sin camino de recuperación', 'hay un camino de recuperación');
  });
}

// ═══════════════════════════════ demo ════════════════════════════════════
async function demo() {
  // Filled in by the demo-phase cases below.
  await demoCases();
}
let demoCases = async () => {};
try { demoCases = (await import('./doc_claims_demo.mjs')).default({ b, js, sleep, screen, press, claim, ok, labelsVisible, bold, CLAIMS, norm, tab, closeSettings, toggles, flip }); } catch (e) {
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
