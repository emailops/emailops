// Demo-phase doc claims: what the docs promise about a mailbox with mail in it.
// Loaded by doc_claims.mjs (`phase demo`), which passes its helpers in. Runs
// against the synthetic demo DB (.emailops-demo-data), never a real mailbox.
export default (h) => async function demoCases() {
  const { b, js, sleep, screen, press, claim, ok, labelsVisible, CLAIMS, norm, tab, closeSettings } = h;
  const view = async (name) => {
    await js((n) => [...document.querySelectorAll('button')].find((e) => e.offsetParent && e.innerText.trim().startsWith(n))?.click(), name);
    await sleep(1600);
  };
  const buttons = () => js(() => [...document.querySelectorAll('button,[role=button]')].filter((e) => e.offsetParent)
    .map((e) => (e.innerText.trim() || e.getAttribute('aria-label') || e.title || '').replace(/\s+/g, ' ')));
  const openThread = async (text) => {
    await js((t) => [...document.querySelectorAll('div[role=button]')].find((e) => e.offsetParent && e.innerText.includes(t))?.click(), text);
    await sleep(1600);
  };

  // ── sidebar ────────────────────────────────────────────────────────────
  await view('Inbox');
  const side = await buttons();
  await claim('start-4-connect-5', 'barra lateral', async () => ok(side.includes('Add account') && side.includes('All accounts'),
    '«Add account» y «All accounts» en la barra lateral', `falta: ${['Add account', 'All accounts'].filter((x) => !side.includes(x))}`));
  await view('All accounts');
  await claim('feat-unified-inbox-1', 'bandeja unificada', async () => {
    const s = await screen();
    const accounts = ['ulises@emailopslabs.dev', 'ulises@fastmail.com', 'ulises.emailopslabs@gmail.com'].filter((a) => s.includes(a));
    return ok(accounts.length >= 2 && side.includes('New folder'), `«All accounts» reúne ${accounts.length} cuentas; «New folder» crea carpetas`, 'falta la vista unificada o la creación de carpetas');
  });
  await claim('feat-smart-filters-1', 'filtros', async () => {
    const s = await screen();
    const groups = ['COMPANIES', 'INTENT', 'TOPIC'].filter((g) => s.includes(g));
    return ok(groups.length === 3, 'filtros por empresa (dominio), intención y tema', `solo: ${groups.join(', ')}`);
  });

  // Full-text search reaches bodies, not just subjects: "Ollama" appears only in a body.
  await claim('feat-search-1', 'texto completo', async () => {
    await js(() => { const i = document.querySelector('input[placeholder^="Search"]'); const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; set.call(i, 'Ollama'); i.dispatchEvent(new Event('input', { bubbles: true })); i.form?.requestSubmit(); });
    await sleep(1800);
    const hit = await js(() => [...document.querySelectorAll('div[role=button]')].filter((e) => e.offsetParent).map((e) => e.innerText));
    await js(() => { const i = document.querySelector('input[placeholder^="Search"]'); const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; set.call(i, ''); i.dispatchEvent(new Event('input', { bubbles: true })); i.form?.requestSubmit(); });
    await sleep(1200);
    return ok(hit.length >= 1, `buscar «Ollama» encuentra ${hit.length} correo(s)`, 'la búsqueda de texto no encuentra nada');
  });

  // ── reading pane ───────────────────────────────────────────────────────
  await view('Inbox');
  await openThread('Nadia Brunner');
  const pane = await buttons();
  const has = (l) => pane.some((p) => p.toLowerCase() === l.toLowerCase());
  await claim('reading-pane-forward', 'panel de lectura', async () => {
    const want = ['Forward', 'Reply', 'Reply all'].filter((l) => !has(l));
    return ok(!want.length, 'Forward junto a Reply y Reply all', `falta: ${want.join(', ')}`);
  });
  await claim('ai-ai-drafts-1', 'botón', async () => {
    const i = pane.findIndex((p) => p === 'Reply All');
    return ok(pane[i + 2] === 'AI Draft' || pane[i + 1] === 'AI Draft', '«AI Draft» junto a Reply All', `orden: ${pane.slice(i, i + 3).join(' · ')}`);
  });
  await claim('ai-chat-mailbox-2', 'contexto del hilo', async () => {
    const s = await screen();
    return ok(/Using as context/i.test(s) || pane.includes('Chat about this thread'), 'con un email abierto el chat ofrece el hilo como contexto', 'no hay contexto de hilo');
  });

  // ── chat ───────────────────────────────────────────────────────────────
  await claim('ai-chat-mailbox-2', 'panel acoplado y vista completa', async () => {
    const all = await buttons();
    return ok(all.includes('Close chat panel') || all.includes('Open chat panel'), 'panel de chat acoplable junto a la bandeja', 'no hay panel')
      + (all.includes('Open full chat view') ? '; vista a pantalla completa disponible' : '');
  });
  await view('Chat');
  await claim('ai-chat-mailbox-3', 'selector de cuenta', async () => {
    const s = await screen();
    return ok(/ulises@emailopslabs\.dev|ulises@fastmail\.com|ulises\.emailopslabs@gmail\.com/.test(s), 'el chat nombra la cuenta que consulta', 'el chat no muestra de qué cuenta responde');
  });

  // ── views ──────────────────────────────────────────────────────────────
  await view('Calendar');
  await claim('feat-calendar-1', 'vistas y Join', async () => {
    const s = await screen();
    const modes = ['Month', 'Week', 'Day'].filter((m) => !(s.includes(m)));
    await js(() => [...document.querySelectorAll('button')].find((e) => e.offsetParent && /Weekly ops sync/.test(e.innerText))?.click());
    await sleep(1200);
    const detail = await screen();
    return ok(!modes.length && /Calendar account/.test(s) && /Join/.test(detail), 'mes, semana y día por cuenta; «Join» en un evento con Meet',
      `falta: ${[...modes, /Join/.test(detail) ? '' : 'Join'].filter(Boolean).join(', ')}`);
  });
  await view('Attachments');
  await claim('feat-attachments-view-1', 'vista', async () => {
    const s = await screen();
    const count = Number(/Attachments\s*\((\d+)\)/.exec(s)?.[1] || 0);
    // The innermost element naming a file, then its clickable row: the first match
    // on a bare `div` can be a whole section, and clicking that opens nothing.
    await js(() => {
      const hits = [...document.querySelectorAll('main *')].filter((e) => e.offsetParent && /\.pdf$/.test(e.innerText?.trim() || ''));
      const leaf = hits.sort((a, b) => a.innerText.length - b.innerText.length)[0];
      (leaf?.closest('[role=button],button,li,[class*=cursor-pointer]') || leaf)?.click();
    });
    await sleep(1500);
    const after = await buttons();
    return ok(count > 0 && after.some((x) => /Open Externally/.test(x)), `${count} adjuntos en un sitio; se abren para previsualizar`, 'no se listan o no se abren');
  });
  await view('Tasks');
  await claim('ai-tasks-1', 'panel', async () => ok(/Tasks .*open/.test(await screen()), 'panel de tareas con lo extraído', 'no hay panel de tareas'));
  await view('Memory');
  await claim('ai-memory-1', 'vista', async () => {
    const s = await screen();
    const states = ['Consolidated', 'Candidate', 'Retired'].filter((x) => !s.includes(x));
    return ok(!states.length && /Edit/.test(s), 'hechos candidatos, consolidados y retirados, inspeccionables', `falta: ${states.join(', ')}`);
  });

  // ── tag board ──────────────────────────────────────────────────────────
  await view('Tag Board');
  await claim('tag-board-dimensions', 'dimensiones', async () => {
    const t = await screen();
    const promised = ['Company', 'Priority', 'Intent', 'Topic'].filter((d) => !t.includes(d));
    return ok(!promised.length, 'Company, Priority, Intent, Topic', `faltan: ${promised.join(', ')}`);
  });
  await claim('ai-classification-1', 'ejes en el Tag Board', async () => {
    const t = await screen();
    const missing = ['Priority', 'Intent', 'Topic'].filter((d) => !t.includes(d));
    return ok(!missing.length, 'prioridad, intención y tema como dimensiones', `falta: ${missing.join(', ')}`);
  });
  await claim('tag-board-toolbar', 'barra', async () => {
    const all = await buttons();
    const missing = ['Today', 'Yesterday', 'Last 7 days'].filter((l) => !all.includes(l));
    const hasSwitch = await js(() => !!document.querySelector('#tagboard-hide-junk'));
    if (!hasSwitch || !(await screen()).includes('Hide junk messages')) missing.push('Hide junk messages');
    return ok(!missing.length, 'Today, Yesterday, Last 7 days y el interruptor Hide junk messages', `falta: ${missing.join(', ')}`);
  });

  await claim('ai-tag-board-2', 'ocultar etiquetas', async () => {
    await js(() => [...document.querySelectorAll('button')].find((e) => e.offsetParent && /More|⋮|Actions/i.test(e.getAttribute('aria-label') || e.title || '') && e.closest('section,div'))?.click());
    await sleep(700);
    const menu = await screen();
    await b.keys('Escape').catch(() => {});
    return ok(/Hide/i.test(menu), 'el menú ⋮ de un bloque ofrece ocultar la etiqueta', 'no hay opción de ocultar');
  });

  // ── settings that need an account ──────────────────────────────────────
  const search = await tab('AI Search');
  const aiSearchOk = () => ok(/Embedding categories/.test(search) && /Rebuild search index/.test(search), 'categorías a indexar y «Rebuild search index» en AI Search', 'falta');
  await claim('ai-semantic-search-1', 'AI Search', async () => aiSearchOk());
  await claim('trbl-search-returns-1', 'AI Search', async () => aiSearchOk());
  await claim('start-after-wizard-first-sync-5', 'AI Search', async () => aiSearchOk());
  await tab('AI Lenses');
  const lensesOn = await h.toggles();
  if (!Object.entries(lensesOn).some(([k, v]) => /AI Lenses|Enable/.test(k) && v)) {
    await js(() => [...document.querySelectorAll('button[aria-pressed],button[aria-checked]')].filter((e) => e.offsetParent).find((e) => {
      let r = e.parentElement; for (let i = 0; i < 4 && r && !r.innerText.trim(); i++) r = r.parentElement;
      return /AI Lenses/.test(r?.innerText || '');
    })?.click());
    await sleep(1500);
  }
  await closeSettings();
  await claim('ai-lenses-1', 'barra lateral', async () => ok((await buttons()).some((x) => /^Lenses/.test(x)), 'al activarlas, Lenses aparece en la barra lateral', 'no aparece'));
  // Put the demo DB back: `make verify` shares it and expects Lenses off.
  if (!Object.entries(lensesOn).some(([k, v]) => /AI Lenses|Enable/.test(k) && v)) {
    await tab('AI Lenses');
    await js(() => [...document.querySelectorAll('button[aria-pressed],button[aria-checked]')].filter((e) => e.offsetParent).find((e) => {
      let r = e.parentElement; for (let i = 0; i < 4 && r && !r.innerText.trim(); i++) r = r.parentElement;
      return /AI Lenses/.test(r?.innerText || '');
    })?.click());
    await sleep(1200);
    await closeSettings();
  }
  void b; void press; void labelsVisible; void CLAIMS; void norm;
};
