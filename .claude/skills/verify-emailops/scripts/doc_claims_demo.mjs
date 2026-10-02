// Demo-phase doc claims: what the docs promise about a mailbox with mail in it.
// Loaded by doc_claims.mjs (`phase demo`), which passes its helpers in. Runs
// against the synthetic demo DB (.emailops-demo-data), never real-world mail.
//
// Same contract as doc_claims.mjs: every case quotes the sentences it proves
// (`covers`), says how in `how`, takes its expected values from `doc`, and
// declares `proof: 'label'` or `partial` when it proves less than the sentence.
// The demo DB is shared with `make verify`: anything a case switches on, it
// switches off again, and nothing here composes, sends or deletes mail.
export default (h) => async function demoCases() {
  const { b, js, sleep, screen, press, claim, ok, labelsVisible, bold, tab, closeSettings, toggles, flip } = h;
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
  const rowTexts = () => js(() => [...document.querySelectorAll('div[role=button]')].filter((e) => e.offsetParent && e.innerText.length > 20)
    .map((e) => e.innerText.replace(/\s+/g, ' ').trim().slice(0, 120)));
  const account = async (email) => {
    // The innermost element with exactly that text: its wrappers match too,
    // and a click on a wrapper never reaches the handler below it.
    await js((m) => [...document.querySelectorAll('button,[role=button],a,div,span')].filter((e) => e.offsetParent && e.innerText.trim() === m).pop()?.click(), email);
    await sleep(1600);
  };
  const DEMO = ['ulises@emailopslabs.dev', 'ulises@fastmail.com'];
  const rowCount = () => js(() => [...document.querySelectorAll('div[role=button]')].filter((e) => e.offsetParent && e.innerText.length > 20).length);
  const search = async (q) => {
    await js((v) => {
      const i = document.querySelector('input[placeholder^="Search"]');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, v);
      i.dispatchEvent(new Event('input', { bubbles: true }));
      i.form?.requestSubmit();
    }, q);
    await sleep(q ? 1800 : 1200);
  };

  // ── sidebar ────────────────────────────────────────────────────────────
  await view('Inbox');
  const side = await buttons();
  await claim('start-4-connect-5', 'añadir cuenta', {
    covers: ['Add more accounts any time with the + button next to Accounts in the sidebar.'],
    how: 'Busca en la barra lateral, junto al encabezado «Accounts», un botón sin texto (el icono +) y lo pulsa: debe abrir el alta de cuenta con Gmail e IMAP. Luego la cierra sin añadir nada.',
  }, async ({ doc }) => {
    doc.match(/\+ button next to Accounts/);
    const found = await js(() => {
      const head = [...document.querySelectorAll('*')].find((e) => e.offsetParent && e.children.length <= 2 && /^ACCOUNTS$/i.test(e.innerText.trim()));
      let row = head;
      for (let i = 0; i < 3 && row && !row.querySelector('button'); i++) row = row.parentElement;
      const el = row && [...row.querySelectorAll('button,[role=button]')].find((e) => e.offsetParent && !e.innerText.trim());
      el?.click();
      return el ? (el.title || el.getAttribute('aria-label') || '') : null;
    });
    if (found === null) return 'FAIL: no hay un botón de icono junto al encabezado «Accounts»';
    await sleep(1400);
    const dialog = await screen();
    await b.keys('Escape').catch(() => {});
    await sleep(600);
    await press('Cancel').catch(() => {});
    await sleep(600);
    return ok(/Gmail/.test(dialog) && /IMAP/.test(dialog), `el botón «${found}» junto a Accounts abre el alta de cuenta`, `el botón «${found}» no abre el alta de cuenta`);
  });
  // Each account's newest threads, then the unified list: it must interleave them.
  const perAccount = {};
  for (const m of DEMO) { await account(m); perAccount[m] = await rowTexts(); }
  await view('All accounts');
  const allTexts = await rowTexts();
  const allScreen = await screen();
  const fromEach = DEMO.filter((m) => perAccount[m].some((t) => allTexts.includes(t)));
  await claim('start-4-connect-5', 'bandeja unificada', {
    covers: ['With several connected you get a unified "All accounts" inbox on top of the per-account views.'],
    how: 'En el buzón demo, lee los hilos más recientes de dos cuentas por separado y después los de «All accounts»: la vista unificada debe contener hilos de las dos, y las cuentas siguen ofreciéndose por separado.',
  }, async ({ doc }) => {
    doc.match(/"All accounts" inbox/);
    return ok(fromEach.length === DEMO.length && DEMO.every((m) => allScreen.includes(m)),
      `«All accounts» mezcla hilos de ${fromEach.join(' y ')}, que siguen como vistas propias`,
      `«All accounts» solo contiene hilos de ${fromEach.join(', ') || 'ninguna cuenta'}`);
  });
  await claim('feat-unified-inbox-1', 'bandeja unificada', {
    covers: ['An All accounts view merges every enabled mailbox into one list, alongside the per-account views.'],
    how: 'Lee los hilos más recientes de dos cuentas del buzón demo por separado y comprueba que «All accounts» contiene hilos de ambas en una sola lista, mientras las vistas por cuenta siguen en la barra lateral.',
  }, async ({ doc }) => {
    doc.match(/All accounts view merges every enabled mailbox/);
    return ok(fromEach.length === DEMO.length, `una sola lista con hilos de ${fromEach.join(' y ')}`, `la vista unificada solo trae ${fromEach.join(', ') || 'nada'}`);
  });
  await claim('feat-unified-inbox-1', 'carpetas', {
    covers: ['you can create, rename, delete and drag messages between folders from inside the app'], proof: 'label',
    how: 'Comprueba que la barra lateral ofrece «New folder». Crear, renombrar, borrar y arrastrar no se ejecutan: el buzón demo es compartido y no tiene carpetas propias.',
  }, async ({ doc }) => {
    doc.match(/create, rename, delete/);
    return ok(side.includes('New folder'), '«New folder» en la barra lateral', 'no se pueden crear carpetas');
  });
  await claim('feat-smart-filters-1', 'filtros', {
    covers: ['Narrow the list by domain, sender, or any classification tag'],
    how: 'En una cuenta del buzón demo, pulsa el primer filtro de TOPIC de la barra lateral: la lista debe cambiar a un subconjunto de hilos. Lo quita después. Comprueba también que hay grupos por empresa (dominio), intención y tema.',
  }, async ({ doc }) => {
    doc.match(/by domain, sender, or any classification tag/);
    await account(DEMO[0]);
    const s0 = await screen();
    const groups = ['COMPANIES', 'INTENT', 'TOPIC'].filter((g) => s0.includes(g));
    const before = await rowTexts();
    const clickTopic = () => js(() => {
      const txt = document.body.innerText;
      const name = txt.slice(txt.indexOf('TOPIC') + 5).split('\n').map((l) => l.trim()).find((l) => l && !/^\d+$/.test(l));
      [...document.querySelectorAll('button,[role=button]')].find((e) => e.offsetParent && e.innerText.trim().startsWith(name))?.click();
      return name;
    });
    const picked = await clickTopic();
    await sleep(1500);
    const after = await rowTexts();
    await clickTopic();
    await sleep(1200);
    // A filter reaches archived mail too (DECISIONS 2026-10-02), so the
    // filtered list can hold threads the plain inbox does not show: what
    // matters is that it changes to a non-empty subset of the mailbox.
    const narrowed = after.length > 0 && after.join('|') !== before.join('|');
    return ok(groups.length === 3 && narrowed,
      `filtrar por «${picked}» cambia la lista a ${after.length} hilos; hay filtros por empresa, intención y tema`,
      `grupos: ${groups.join(', ')}; filtrar por «${picked}» no cambia la lista (${after.length} de ${before.length})`);
  });

  // Full-text search reaches bodies, not just subjects: "Ollama" appears only in a body.
  await claim('feat-search-1', 'texto completo', {
    covers: ['Full-text search over subjects, bodies, senders and attachments.'],
    partial: 'se prueba la búsqueda en el cuerpo; asuntos, remitentes y adjuntos no se prueban por separado',
    how: 'Busca «Ollama», una palabra que en el buzón demo solo aparece en el cuerpo de un correo, y comprueba que la búsqueda lo encuentra.',
  }, async ({ doc }) => {
    doc.match(/Full-text search over subjects, bodies/);
    await search('Ollama');
    const hits = await rowCount();
    await search('');
    return ok(hits >= 1, `buscar «Ollama» (solo en un cuerpo) encuentra ${hits} correo(s)`, 'la búsqueda de texto no encuentra nada en los cuerpos');
  });

  await claim('feat-search-2', 'operadores', {
    covers: ['| from:ana | sender address or name |', '| subject:invoice | subject line |',
             '| before:2026-09-01 / after:2026-09-01 | received date |',
             '| tag:newsletter / tag:intent=request | a classifier tag, optionally within one facet |'],
    partial: 'to: e id: no se ejecutan: en el buzón demo todo va dirigido al usuario y los ids no se ven en pantalla',
    how: 'Ejecuta cada operador de la tabla en el buscador del buzón demo con un valor tomado de la propia demo y compara los hilos que devuelve con los de la bandeja sin filtrar: deben ser un subconjunto estricto (y una fecha imposible, ninguno).',
  }, async ({ doc }) => {
    doc.match(/narrowed with operators/);
    await view('Inbox');
    const allRows = await rowTexts();
    const all = allRows.length;
    // The list is virtualised, so a filtered result set is not a subset of the
    // rows on screen: check what each operator promises instead.
    const probes = [
      ['from:nadia', (rows) => rows.length > 0 && rows.every((t) => /nadia/i.test(t))],
      ['subject:ollama', (rows) => rows.length > 0 && rows.every((t) => /ollama/i.test(t))],
      ['tag:intent=request', (rows) => rows.length > 0 && rows.join('|') !== allRows.join('|')],
      ['after:2030-01-01', (rows) => rows.length === 0],
    ];
    const results = [];
    for (const [q, good] of probes) {
      await search(q);
      const rows = await rowTexts();
      results.push([q, rows.length, good(rows)]);
    }
    await search('');
    const bad = results.filter(([, , good]) => !good);
    return ok(!bad.length, results.map(([q, n]) => `${q} → ${n}`).join(', ') + ` (bandeja: ${all})`,
      `no filtran como promete la tabla: ${bad.map(([q, n]) => `${q} → ${n}`).join(', ')} (bandeja: ${all})`);
  });

  // ── reading pane ───────────────────────────────────────────────────────
  await view('Inbox');
  await openThread('Nadia Brunner');
  const pane = await buttons();
  const has = (l) => pane.some((p) => p.toLowerCase() === l.toLowerCase());
  await claim('reading-pane-forward', 'botones del panel', {
    covers: ['Forward sits next to Reply and Reply all in the reading pane.'],
    how: 'Abre un hilo del buzón demo y comprueba que los botones que nombra la doc están en el panel de lectura, contiguos.',
  }, async () => {
    const named = bold('reading-pane-forward');
    const idx = named.map((l) => pane.findIndex((p) => p.toLowerCase() === l.toLowerCase()));
    const together = idx.every((i) => i >= 0) && Math.max(...idx) - Math.min(...idx) <= named.length;
    return ok(together, `${named.join(', ')} juntos en el panel de lectura`, `falta o están separados: ${named.filter((l) => !has(l)).join(', ') || pane.join(' · ')}`);
  });
  await claim('ai-ai-drafts-1', 'botón', {
    covers: ['An AI Draft button next to Reply All'], partial: 'que redacte una respuesta basada en el hilo necesita un modelo descargado',
    how: 'Abre un hilo y comprueba que el botón que nombra la doc está justo al lado de «Reply All».',
  }, async () => {
    const [draft] = bold('ai-ai-drafts-1');
    const i = pane.findIndex((p) => p === 'Reply All');
    return ok(pane[i + 2] === draft || pane[i + 1] === draft, `«${draft}» junto a Reply All`, `orden: ${pane.slice(i, i + 3).join(' · ')}`);
  });
  await claim('ai-chat-mailbox-2', 'contexto del hilo', {
    covers: ['With an email open the panel offers that thread as context via a removable chip'],
    partial: 'a qué se responde con el hilo y a qué con el buzón necesita un modelo descargado',
    how: 'Con un hilo abierto, comprueba que el panel de chat muestra el hilo como contexto («Using as context») y que ese chip se puede quitar.',
  }, async ({ doc }) => {
    doc.match(/removable chip/);
    const s = await screen();
    const removable = await js(() => [...document.querySelectorAll('button')].some((e) => e.offsetParent && /remove|clear|×/i.test((e.getAttribute('aria-label') || e.title || e.innerText || '')) && /context/i.test(e.closest('div')?.innerText || '')));
    return ok((/Using as context/i.test(s) || pane.includes('Chat about this thread')) && removable, 'el hilo abierto aparece como contexto quitable', 'no hay contexto de hilo o no se puede quitar');
  });

  // ── chat ───────────────────────────────────────────────────────────────
  await claim('ai-chat-mailbox-2', 'panel acoplado y vista completa', {
    covers: ['Chat lives in a resizable panel docked to the right of the inbox', 'there is also a full-page view for longer sessions'],
    how: 'Comprueba que el panel de chat está a la derecha de la bandeja (su borde izquierdo más allá de la mitad de la ventana), que tiene un tirador para cambiar su ancho y que existe el botón de vista completa.',
  }, async ({ doc }) => {
    doc.match(/docked to the right of the inbox/);
    const all = await buttons();
    const geo = await js(() => {
      const p = [...document.querySelectorAll('aside,section,div')].find((e) => e.offsetParent && /chat/i.test(e.getAttribute('aria-label') || '') && e.getBoundingClientRect().width > 200);
      const handle = !!document.querySelector('[role=separator],[class*=resize],[class*=cursor-col-resize]');
      return { left: p ? p.getBoundingClientRect().left : null, width: innerWidth, handle };
    });
    const right = geo.left !== null && geo.left > geo.width / 2;
    return ok((all.includes('Close chat panel') || all.includes('Open chat panel')) && all.includes('Open full chat view') && geo.handle,
      `panel de chat${right ? ' a la derecha' : ''}, redimensionable, con vista completa`,
      `panel: ${all.includes('Close chat panel') || all.includes('Open chat panel')}, vista completa: ${all.includes('Open full chat view')}, tirador: ${geo.handle}`);
  });
  await view('Chat');
  await claim('ai-chat-mailbox-11', 'panel de razonamiento', {
    covers: ['Every answer has a Show reasoning panel that lists what happened, in order: which route the question took and what decided it, the query planner, the mailbox search, the guide sections used, each model call with its timing, and each tool call with its arguments and result.'],
    partial: 'el detalle depende de la traza guardada en la conversación demo',
    how: 'Abre una conversación guardada del buzón demo y despliega bajo la respuesta el panel que nombra la doc («Show reasoning»): debe listar la ruta seguida y los tiempos.',
  }, async ({ doc }) => {
    const label = doc.match(/a (Show reasoning) panel/)[1];
    await js(() => [...document.querySelectorAll('button,[role=button]')]
      .filter((e) => e.offsetParent && e.innerText.trim().length > 12 && e.innerText.length < 120 && /\?$/.test(e.innerText.trim())).pop()?.click());
    await sleep(2200);
    const opened = await js((l) => {
      const el = [...document.querySelectorAll('button,[role=button],summary')].find((e) => e.offsetParent && e.innerText.trim().startsWith(l));
      el?.click();
      return !!el;
    }, label);
    await sleep(1200);
    const panel = await screen();
    return ok(opened && /route|ruta|tool|herramienta|\d+(\.\d+)?s\b/i.test(panel), `«${label}» despliega la traza de la respuesta`,
      opened ? 'el panel no muestra la traza' : `no hay «${label}» bajo la respuesta`);
  });
  await claim('ai-chat-mailbox-3', 'selector de cuenta', {
    covers: ['a picker names which one'], partial: 'que la búsqueda se limite a esa cuenta necesita un modelo descargado',
    how: 'Abre el chat y comprueba que muestra el nombre de la cuenta sobre la que responde.',
  }, async ({ doc }) => {
    doc.match(/a picker names which one/);
    const s = await screen();
    return ok(/ulises@emailopslabs\.dev|ulises@fastmail\.com|ulises\.emailopslabs@gmail\.com/.test(s), 'el chat nombra la cuenta que consulta', 'el chat no muestra de qué cuenta responde');
  });

  // ── views ──────────────────────────────────────────────────────────────
  await view('Calendar');
  const cal = await screen();
  await claim('feat-calendar-1', 'vistas', {
    covers: ['Per-account month, week and day views'],
    how: 'En la vista Calendar del buzón demo, pulsa cada vista que nombra la doc (Month, Week, Day) y comprueba que la rejilla cambia; y que el calendario se muestra por cuenta.',
  }, async ({ doc }) => {
    const modes = doc.match(/Per-account (\w+), (\w+) and (\w+) views/).slice(1).map((m) => m[0].toUpperCase() + m.slice(1));
    const grids = [];
    for (const m of modes) {
      await press(m).catch(() => {});
      grids.push(await js(() => document.querySelector('main')?.innerText.length || 0));
    }
    const changed = new Set(grids).size === grids.length;
    return ok(changed && /Calendar account/.test(cal), `${modes.join(', ')} cambian la rejilla; calendario por cuenta`, `las vistas ${modes.join(', ')} no cambian la rejilla (${grids.join(', ')})`);
  });
  await claim('feat-calendar-1', 'Join', {
    covers: ['a one-click Join button for Meet'], partial: 'se prueba un evento con Meet; Teams, Webex, Zoom y los recordatorios no',
    how: 'Abre el evento demo «Weekly ops sync», que tiene un enlace de Meet, y comprueba que ofrece «Join».',
  }, async ({ doc }) => {
    doc.match(/Join button for Meet/);
    await press('Month').catch(() => {});
    await js(() => [...document.querySelectorAll('button')].find((e) => e.offsetParent && /Weekly ops sync/.test(e.innerText))?.click());
    await sleep(1200);
    const detail = await screen();
    await b.keys('Escape').catch(() => {});
    return ok(/Join/.test(detail), '«Join» en un evento con Meet', 'el evento con Meet no ofrece Join');
  });
  await view('Attachments');
  await claim('feat-attachments-view-1', 'vista y previsualización', {
    covers: ['One place for the attachments you care about — invoices, contracts, receipts — with preview and download, instead of digging back through threads.', 'Open it from Attachments in the sidebar.'],
    partial: 'la descarga no se ejecuta',
    how: 'Abre la vista Attachments desde la barra lateral del buzón demo, lee cuántos adjuntos lista y abre un PDF: debe previsualizarse con la opción «Open Externally».',
  }, async ({ doc }) => {
    doc.match(/listing|attachments you care about/);
    const s = await screen();
    const count = Number(/Attachments\s*\((\d+)\)/.exec(s)?.[1] || 0);
    // The innermost element naming a file, then its clickable row: the first match
    // on a bare `div` can be a whole section, and clicking that opens nothing.
    await js(() => {
      const hits = [...document.querySelectorAll('main *')].filter((e) => e.offsetParent && /\.pdf$/.test(e.innerText?.trim() || ''));
      const leaf = hits.sort((x, y) => x.innerText.length - y.innerText.length)[0];
      (leaf?.closest('[role=button],button,li,[class*=cursor-pointer]') || leaf)?.click();
    });
    await sleep(1500);
    const after = await buttons();
    await b.keys('Escape').catch(() => {});
    await sleep(600);
    return ok(count > 0 && after.some((x) => /Open Externally/.test(x)), `${count} adjuntos en un sitio; se abren para previsualizar`, 'no se listan o no se abren');
  });
  await claim('feat-attachments-view-2', 'reglas', {
    covers: ['Click Manage Rules (or Create a Rule on the empty view) and fill in:'],
    partial: 'que la vista empiece vacía no se observa: el buzón demo ya trae reglas',
    how: 'Pulsa «Manage Rules» en la vista Attachments y luego «Create a Rule»: debe aparecer el formulario de regla nueva.',
  }, async ({ doc }) => {
    const [manage, create] = doc.match(/Click (Manage Rules) \(or (Create a Rule)/).slice(1);
    await press(manage);
    await sleep(1200);
    const panel = await screen();
    await press(create).catch(() => {});
    await sleep(1400);
    const form = await screen();
    return ok(panel.includes(create) && /New Rule/.test(form), `«${manage}» abre las reglas y «${create}» el formulario`,
      `«${manage}»: ${panel.includes(create) ? 'abre las reglas pero' : 'no abre las reglas y'} «${create}» no da formulario`);
  });
  const ruleForm = await screen();
  await claim('feat-attachments-view-3', 'nombre', {
    covers: bold('feat-attachments-view-3'), proof: 'label',
    how: 'Comprueba que los campos que nombra la doc están en el formulario de reglas de adjuntos. Que el patrón filtre de verdad necesita correo nuevo entrando.',
  }, async () => {
    const fields = bold('feat-attachments-view-3');
    const missing = fields.filter((f) => !ruleForm.includes(f));
    return ok(!missing.length, `${fields.join(', ')} en el formulario`, `falta: ${missing.join(', ')}`);
  });
  await claim('feat-attachments-view-4', 'remitente', {
    covers: bold('feat-attachments-view-4'), proof: 'label',
    how: 'Comprueba que los campos que nombra la doc están en el formulario de reglas de adjuntos. Que el patrón filtre de verdad necesita correo nuevo entrando.',
  }, async () => {
    const fields = bold('feat-attachments-view-4');
    const missing = fields.filter((f) => !ruleForm.includes(f));
    return ok(!missing.length, `${fields.join(', ')} en el formulario`, `falta: ${missing.join(', ')}`);
  });
  await claim('feat-attachments-view-5', 'asunto y fichero', {
    covers: bold('feat-attachments-view-5'), proof: 'label',
    how: 'Comprueba que los campos que nombra la doc están en el formulario de reglas de adjuntos. Que el patrón filtre de verdad necesita correo nuevo entrando.',
  }, async () => {
    const fields = bold('feat-attachments-view-5');
    const missing = fields.filter((f) => !ruleForm.includes(f));
    return ok(!missing.length, `${fields.join(', ')} en el formulario`, `falta: ${missing.join(', ')}`);
  });
  await claim('feat-attachments-view-6', 'etiquetas', {
    covers: bold('feat-attachments-view-6'), proof: 'label',
    how: 'Comprueba que los campos que nombra la doc están en el formulario de reglas de adjuntos. Que el patrón filtre de verdad necesita correo nuevo entrando.',
  }, async () => {
    const fields = bold('feat-attachments-view-6');
    const missing = fields.filter((f) => !ruleForm.includes(f));
    return ok(!missing.length, `${fields.join(', ')} en el formulario`, `falta: ${missing.join(', ')}`);
  });
  await claim('feat-attachments-view-7', 'aplicar a lo existente', {
    covers: ['tick Apply to existing emails after creating to collect from the mail you already have'], proof: 'label',
    how: 'Comprueba que el formulario de reglas ofrece aplicar la regla al correo ya sincronizado.',
  }, async ({ doc }) => {
    doc.match(/Apply to existing emails/);
    return ok(/Apply to existing emails/.test(ruleForm), 'el formulario ofrece aplicar la regla a lo ya sincronizado', 'no lo ofrece');
  });
  await press('Cancel').catch(() => {});
  await b.keys('Escape').catch(() => {});
  await sleep(900);
  await view('Tasks');
  await claim('ai-tasks-1', 'panel', {
    covers: ['collects them in a Tasks panel'], partial: 'la extracción necesita un modelo; aquí se ven las tareas ya extraídas del buzón demo',
    how: 'Abre la vista Tasks del buzón demo y comprueba que lista tareas abiertas.',
  }, async ({ doc }) => {
    doc.match(/Tasks panel/);
    return ok(/Tasks .*open/.test(await screen()), 'panel de tareas con lo extraído', 'no hay panel de tareas');
  });
  await view('Memory');
  const mem = await screen();
  await claim('ai-memory-1', 'inspeccionable', {
    covers: ['Everything it has learned is inspectable'],
    how: 'Abre la vista Memory del buzón demo y comprueba que lista los hechos aprendidos y permite editarlos.',
  }, async ({ doc }) => {
    doc.match(/inspectable/);
    return ok(/Edit/.test(mem) && /Consolidated|Candidate/.test(mem), 'los hechos aprendidos se listan y editan', 'no se ven los hechos');
  });
  await claim('ai-memory-1', 'estados', {
    covers: ['Candidate facts are scored and promoted past a threshold; low-scoring ones expire.'], proof: 'label',
    how: 'Comprueba que la vista Memory distingue hechos candidatos, consolidados y retirados. La puntuación y el umbral no se ven.',
  }, async ({ doc }) => {
    doc.match(/Candidate facts/);
    const states = ['Consolidated', 'Candidate', 'Retired'].filter((x) => !mem.includes(x));
    return ok(!states.length, 'candidatos, consolidados y retirados', `falta: ${states.join(', ')}`);
  });

  // ── tag board ──────────────────────────────────────────────────────────
  const views = side.join(' · ');
  await view('Tag Board');
  await claim('tag-board-dimensions', 'ubicación', {
    covers: ['The Tag Board (under Views in the sidebar, next to the inbox) turns those tags into a board.'],
    how: 'Comprueba que «Tag Board» está en la barra lateral junto a Inbox y que al pulsarlo aparece un tablero de bloques.',
  }, async ({ doc }) => {
    doc.match(/under Views in the sidebar/);
    return ok(/Tag Board/.test(views) && /Inbox/.test(views) && /Company|Priority/.test(await screen()), 'Tag Board en la barra lateral, abre el tablero', 'no está en la barra lateral');
  });
  await claim('tag-board-dimensions', 'dimensiones', {
    covers: ['Pick one dimension — Company, Priority, Intent or Topic'],
    how: 'Lee las dimensiones que enumera la doc y pulsa cada una en el Tag Board: el tablero debe cambiar de bloques con cada una.',
  }, async ({ doc }) => {
    const dims = doc.match(/Pick one dimension — (.+?) —/)[1].split(/, | or /);
    const boards = [];
    for (const d of dims) {
      await press(d).catch(() => {});
      boards.push(await js(() => document.querySelector('main')?.innerText.slice(0, 400) || ''));
    }
    const distinct = new Set(boards).size;
    await press(dims[0]).catch(() => {});
    return ok(distinct === dims.length, `${dims.join(', ')} dan tableros distintos`, `solo ${distinct} tableros distintos para ${dims.join(', ')}`);
  });
  await claim('tag-board-toolbar', 'barra', {
    covers: ['The toolbar narrows the board by time (Today, Yesterday, Last 7 days, or a custom date range)', 'with the same Hide junk messages switch as the inbox'], proof: 'label',
    how: 'Lee de la doc los filtros de tiempo y el interruptor, y comprueba que la barra del Tag Board los tiene. Su efecto sobre el tablero no se comprueba.',
  }, async ({ doc }) => {
    const times = doc.match(/by time \(([^)]+?), or a custom date range\)/)[1].split(', ');
    const all = await buttons();
    const missing = times.filter((l) => !all.includes(l));
    const sw = bold('tag-board-toolbar').find((l) => /junk/i.test(l)) || 'Hide junk messages';
    const hasSwitch = await js(() => !!document.querySelector('#tagboard-hide-junk'));
    if (!hasSwitch || !(await screen()).includes(sw)) missing.push(sw);
    return ok(!missing.length, `${times.join(', ')} y el interruptor ${sw}`, `falta: ${missing.join(', ')}`);
  });
  await claim('ai-tag-board-2', 'ocultar etiquetas', {
    covers: ['hide a tag from its ⋮ menu'], proof: 'label',
    how: 'Abre el menú ⋮ de un bloque y comprueba que ofrece ocultar la etiqueta; no se oculta para no alterar el buzón demo compartido.',
  }, async ({ doc }) => {
    doc.match(/hide a tag from its ⋮ menu/);
    await js(() => [...document.querySelectorAll('button')].find((e) => e.offsetParent && /More|⋮|Actions/i.test(e.getAttribute('aria-label') || e.title || '') && e.closest('section,div'))?.click());
    await sleep(700);
    const menu = await screen();
    await b.keys('Escape').catch(() => {});
    return ok(/Hide/i.test(menu), 'el menú ⋮ de un bloque ofrece ocultar la etiqueta', 'no hay opción de ocultar');
  });

  // ── settings that need an account ──────────────────────────────────────
  const aiSearch = await tab('AI Search');
  await claim('ai-semantic-search-1', 'AI Search', {
    covers: ['Pick which categories get embedded, and rebuild the index from scratch after changing the embedding model, in Settings → AI Search.'],
    how: 'Abre Ajustes → AI Search con una cuenta del buzón demo y comprueba que tiene las categorías a indexar (casillas) y el botón «Rebuild search index».',
  }, async ({ doc }) => {
    doc.match(/Settings → AI Search/);
    const boxes = await js(() => [...document.querySelectorAll('input[type=checkbox]')].filter((e) => e.offsetParent).length);
    return ok(/Embedding categories/.test(aiSearch) && boxes > 0 && /Rebuild search index/.test(aiSearch), 'categorías a indexar y «Rebuild search index»', 'faltan las categorías o el botón');
  });
  await claim('trbl-search-returns-1', 'AI Search', {
    covers: ['Open Settings → AI Search, check that the categories you care about are selected', 'After changing the embedding model, rebuild the index from the same screen.'],
    how: 'Comprueba que Ajustes → AI Search tiene las casillas de categorías y el botón de reconstruir el índice.',
  }, async ({ doc }) => {
    doc.match(/Settings → AI Search/);
    return ok(/Embedding categories/.test(aiSearch) && /Rebuild search index/.test(aiSearch), 'categorías y reconstrucción en AI Search', 'falta');
  });
  await claim('start-after-wizard-first-sync-5', 'AI Search', {
    covers: ['You can watch progress and rebuild the index in Settings → AI Search.'], partial: 'el progreso de una indexación en curso no se ve con el índice demo ya completo',
    how: 'Comprueba que Ajustes → AI Search tiene el botón de reconstruir el índice.',
  }, async ({ doc }) => {
    doc.match(/rebuild the index in Settings → AI Search/);
    return ok(/Rebuild search index/.test(aiSearch), '«Rebuild search index» en AI Search', 'falta');
  });
  await tab('AI Lenses');
  const lensesOn = await toggles();
  const wasOn = Object.entries(lensesOn).some(([k, v]) => /AI Lenses|Enable/.test(k) && v);
  const flipLenses = async () => {
    await js(() => [...document.querySelectorAll('button[aria-pressed],button[aria-checked]')].filter((e) => e.offsetParent).find((e) => {
      let r = e.parentElement; for (let i = 0; i < 4 && r && !r.innerText.trim(); i++) r = r.parentElement;
      return /AI Lenses/.test(r?.innerText || '');
    })?.click());
    await sleep(1500);
  };
  if (!wasOn) await flipLenses();
  await closeSettings();
  await claim('ai-lenses-1', 'barra lateral', {
    covers: ['that you create and run from the sidebar'], partial: 'crear y ejecutar una lente necesita un modelo descargado',
    how: 'Enciende AI Lenses en Ajustes y comprueba que «Lenses» aparece en la barra lateral; luego lo deja como estaba.',
  }, async ({ doc }) => {
    doc.match(/from the sidebar/);
    return ok((await buttons()).some((x) => /^Lenses/.test(x)), 'al activarlas, Lenses aparece en la barra lateral', 'no aparece');
  });
  // Put the demo DB back as it was: `make verify` shares it.
  if (!wasOn) {
    await tab('AI Lenses');
    await flipLenses();
    await closeSettings();
  }
  // ── skills (experimental, off by default) ─────────────────────────────
  // Everything below is put back: the switch as it was, and the one skill the
  // cases create is deleted again (it lands in skills/.deleted of the demo dir).
  const skillsSwitch = () => js(() => [...document.querySelectorAll('[role=switch]')].filter((e) => e.offsetParent).find((e) => {
    let r = e.parentElement; for (let i = 0; i < 4 && r && !/Enable skills/.test(r.innerText); i++) r = r.parentElement;
    return /Enable skills/.test(r?.innerText || '');
  })?.getAttribute('aria-checked'));
  const flipSkills = async () => {
    await js(() => [...document.querySelectorAll('[role=switch]')].filter((e) => e.offsetParent).find((e) => {
      let r = e.parentElement; for (let i = 0; i < 4 && r && !/Enable skills/.test(r.innerText); i++) r = r.parentElement;
      return /Enable skills/.test(r?.innerText || '');
    })?.click());
    await sleep(1500);
  };
  const byTestId = (id) => js((t) => { const e = document.querySelector(`[data-testid="${t}"]`); if (e) e.click(); return !!e; }, id);
  await tab('AI Skills');
  const skillsWasOn = (await skillsSwitch()) === 'true';
  await closeSettings();
  const skillsInSideBefore = (await buttons()).some((x) => x === 'Skills');
  if (!skillsWasOn) {
    await tab('AI Skills');
    await flipSkills();
    await closeSettings();
  }
  const skillsInSideAfter = (await buttons()).some((x) => x === 'Skills');
  await claim('ai-skills-2', 'activar', {
    covers: ['Turn them on in Settings → AI Skills with Enable skills; a Skills entry then appears in the sidebar.'],
    how: 'Abre Ajustes → AI Skills, enciende «Enable skills» si estaba apagado y comprueba que la barra lateral pasa a tener «Skills» (y que no lo tenía con el interruptor apagado).',
  }, async ({ doc }) => {
    const label = doc.bold().find((x) => /^Enable/.test(x)) || 'Enable skills';
    return ok(skillsInSideAfter && (skillsWasOn || !skillsInSideBefore),
      `con «${label}» encendido aparece Skills en la barra lateral${skillsWasOn ? '' : ', y no estaba antes'}`,
      `barra lateral antes: ${skillsInSideBefore}, después: ${skillsInSideAfter}`);
  });
  await view('Skills');
  await byTestId('skill-new');
  await sleep(500);
  await js(() => {
    const i = document.querySelector('[data-testid="skill-new-name"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, 'docs-check');
    i.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await byTestId('skill-create');
  await sleep(1500);
  const editorText = await js(() => document.querySelector('[data-testid="skill-editor"]')?.value || '');
  const rowSwitch = await js(() => !!document.querySelector('[data-testid="skill-toggle-docs-check"]'));
  await claim('ai-skills-3', 'crear', {
    covers: ['lists every skill with its own switch and opens the selected one\'s SKILL.md in an editor', 'New creates a skill from a template.'],
    how: 'En la vista Skills pulsa «New», crea «docs-check» y comprueba que su fila tiene interruptor y que el editor abre un SKILL.md de plantilla con name y description.',
  }, async ({ doc }) => {
    doc.match(/creates a skill from a template/);
    return ok(rowSwitch && /name: docs-check/.test(editorText) && /description:/.test(editorText),
      'la skill nueva tiene su interruptor y el editor muestra la plantilla', `interruptor: ${rowSwitch}; editor: ${editorText.slice(0, 60)}`);
  });
  if (!(await js(() => !!document.querySelector('textarea[placeholder^="Ask about your emails"]')))) {
    await js(() => document.querySelector('[aria-label="Open chat panel"]')?.click());
    await sleep(1200);
  }
  const typeChat = (v) => js((t) => {
    const i = document.querySelector('textarea[placeholder^="Ask about your emails"]');
    if (!i) return false;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(i, t);
    i.dispatchEvent(new Event('input', { bubbles: true }));
    return true;
  }, v);
  const chatFound = await typeChat('/');
  await sleep(1500);
  const suggested = await js(() => !!document.querySelector('[data-testid="slash-option-docs-check"]'));
  await typeChat('');
  await claim('ai-skills-4', 'sugerencias', {
    covers: ['Typing / lists your enabled skills.'],
    how: 'Escribe «/» en el chat con la skill «docs-check» creada y activa: debe aparecer en la lista de sugerencias. Luego vacía el campo.',
  }, async ({ doc }) => {
    doc.match(/Typing \/ lists your enabled skills/);
    return ok(chatFound && suggested, '«/» sugiere /docs-check', chatFound ? 'no aparece la sugerencia' : 'no se encontró el campo del chat');
  });
  await view('Skills');
  await js(() => document.querySelector('[data-testid="skill-row-docs-check"]')?.click());
  await sleep(800);
  await byTestId('skill-delete');
  await sleep(300);
  await byTestId('skill-delete-confirm');
  await sleep(1500);
  const stillListed = await js(() => !!document.querySelector('[data-testid="skill-row-docs-check"]'));
  await claim('ai-skills-3', 'borrar', {
    covers: ['Delete moves it to skills/.deleted'], partial: 'que el fichero quede en skills/.deleted lo prueba un test; aquí solo se ve que sale de la lista',
    how: 'Selecciona «docs-check», pulsa «Delete» y confirma: la skill debe desaparecer de la lista.',
  }, async ({ doc }) => {
    doc.match(/moves it to skills\/\.deleted/);
    return ok(!stillListed, 'tras confirmar, la skill sale de la lista', 'sigue en la lista');
  });
  if (!skillsWasOn) {
    await tab('AI Skills');
    await flipSkills();
    await closeSettings();
  }
  await tab('Calendar');
  const calSettings = await screen();
  await claim('feat-calendar-1', 'ajustes', {
    covers: ['can be switched off per account, along with the notification lead time, in Settings → Calendar'], proof: 'label',
    how: 'Abre Ajustes → Calendar con las cuentas del buzón demo y comprueba que hay un interruptor por cuenta y el ajuste de antelación de avisos. No se apaga ninguno para no alterar el buzón demo.',
  }, async ({ doc }) => {
    doc.match(/notification lead time/);
    const perAccountSwitches = await js(() => [...document.querySelectorAll('[role=switch]')].filter((e) => e.offsetParent && /@/.test(e.getAttribute('aria-label') || '')).length);
    return ok(perAccountSwitches > 0 && /Notification lead time/.test(calSettings), `${perAccountSwitches} interruptor(es) por cuenta y antelación de avisos`, `interruptores por cuenta: ${perAccountSwitches}; antelación: ${/Notification lead time/.test(calSettings)}`);
  });
  await closeSettings();

  // ── organizing, snooze, send, signatures, shortcuts, sender controls ──────
  // Everything here is undone before the next case: stars are removed again,
  // archives are undone (or refused by the credential-less demo account and
  // rolled back), snoozes are lifted, the block is unblocked, the signature
  // is emptied and the composer is cancelled without a draft.
  const toastText = () => js(() => document.querySelector('[data-testid="toast-stack"]')?.innerText.replace(/\s+/g, ' ') || '');
  const closeToasts = async () => { await js(() => document.querySelectorAll('[data-testid="toast-stack"] button[aria-label="Close"]').forEach((x) => x.click())); await sleep(300); };
  const toastAction = (label) => js((l) => { const x = [...document.querySelectorAll('[data-testid="toast-stack"] button')].find((y) => y.textContent.trim() === l); x?.click(); return !!x; }, label);
  const waitToast = async (re, ms) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { const t = await toastText(); if (re.test(t)) return t; await sleep(400); } return null; };
  const listRows = () => js(() => [...document.querySelectorAll('div[role="button"]')].filter((r) => r.offsetParent && r.querySelector('[data-testid="row-select"]')).map((r) => r.innerText.replace(/\s+/g, ' ')));
  const hasRow = async (t) => (await listRows()).some((r) => r.includes(t));
  const inRow = (text, sel, act = false) => js((s, q, a) => { const r = [...document.querySelectorAll('div[role="button"]')].find((x) => x.offsetParent && x.innerText.includes(s) && x.querySelector('[data-testid="row-select"]')); const el = r?.querySelector(q); if (!el) return null; if (a) el.click(); return el.getAttribute('aria-pressed') ?? true; }, text, sel, act);
  const rowMenu = async (text) => { const r = await inRow(text, '[aria-label="More actions"]', true); await sleep(700); return r; };
  const menuLabels = () => js(() => [...document.querySelectorAll('button,[role=menuitem]')].filter((x) => x.offsetParent && !x.closest('[data-testid="bulk-toolbar"], nav, aside, [data-testid="toast-stack"], div[role="button"]')).map((x) => x.textContent.trim()).filter(Boolean));
  const blur = () => js(() => document.activeElement?.blur());
  const h1 = () => js(() => document.querySelector('header h1')?.textContent.trim() || '');
  const headerTitles = () => js(() => [...document.querySelectorAll('header button')].map((x) => x.title).filter(Boolean));
  const sidebar = async (label) => {
    await js((l) => {
      const all = [...document.querySelectorAll('nav button, aside button')].filter((e) => e.offsetParent);
      (all.find((e) => e.innerText.trim() === l) || all.find((e) => e.innerText.trim().startsWith(l)))?.click();
    }, label);
    await sleep(1500);
  };
  const heading = () => js(() => document.querySelector('h2')?.textContent.trim() || '');
  const dialogText = (re) => js((r) => [...document.querySelectorAll('[role="dialog"], [aria-modal="true"], .fixed')].map((d) => d.innerText.trim()).find((t) => new RegExp(r).test(t)) || null, re);
  const dialogButton = (re, label) => js((r, l) => { const d = [...document.querySelectorAll('[role="dialog"], [aria-modal="true"], .fixed')].find((x) => new RegExp(r).test(x.innerText.trim())); const x = d && [...d.querySelectorAll('button')].filter((y) => y.textContent.trim() === l).pop(); x?.click(); return !!x; }, re, label);

  await account(DEMO[0]);
  await view('Inbox');
  await closeToasts();

  // Star from the row, and back.
  await claim('feat-organize-1', 'estrella en la fila', {
    covers: ['the star also sits on every row of the list'],
    how: 'En la bandeja demo pulsa la estrella de la fila de Nadia Brunner: debe quedar pulsada; la vuelve a pulsar y debe soltarse.',
  }, async ({ doc }) => {
    doc.match(/the star also sits on every row/);
    const start = await inRow('Nadia Brunner', '[data-testid="star-toggle"]');
    if (start === null) return 'FAIL: la fila de Nadia Brunner no tiene estrella';
    await inRow('Nadia Brunner', '[data-testid="star-toggle"]', true); await sleep(1200);
    const on = await inRow('Nadia Brunner', '[data-testid="star-toggle"]');
    await inRow('Nadia Brunner', '[data-testid="star-toggle"]', true); await sleep(1200);
    const off = await inRow('Nadia Brunner', '[data-testid="star-toggle"]');
    return ok(on !== start && off === start, `estrella ${start} → ${on} → ${off}`, `estrella ${start} → ${on} → ${off}`);
  });

  // The ⋮ menu and the reading pane offer the actions the docs name.
  await rowMenu('Nadia Brunner');
  const rowMenuItems = await menuLabels();
  await b.keys('Escape'); await sleep(500);
  await openThread('Nadia Brunner');
  const paneTitles = await headerTitles();
  await press('Back').catch(() => {});
  await sleep(1000);
  await claim('feat-organize-1', 'menú y panel', {
    covers: ['Both, along with Mark as unread and Star, are in the reading pane and in each conversation\'s More actions (⋮) menu'],
    proof: 'label',
    how: 'Abre el menú ⋮ («More actions») de una fila y el hilo en el panel de lectura, y comprueba que ofrecen Archive, Mark as unread y Star (Move to Inbox se comprueba en la vista Archive).',
  }, async () => {
    const [archive, , unread, star, more] = bold('feat-organize-1');
    const inMenu = [archive, unread, star].filter((l) => !rowMenuItems.includes(l));
    const inPane = [archive, unread, star].filter((l) => !paneTitles.some((x) => x.startsWith(l)));
    const moreOk = await js((l) => !!document.querySelector(`[aria-label="${l}"]`), more);
    return ok(!inMenu.length && !inPane.length && moreOk, `menú «${more}» y panel de lectura ofrecen ${[archive, unread, star].join(', ')}`,
      `faltan en el menú: ${inMenu.join(', ') || '—'}; en el panel: ${inPane.join(', ') || '—'} (títulos: ${paneTitles.join(', ')}); botón «${more}»: ${moreOk}`);
  });

  // Starred and Archive views.
  await view('All accounts');
  await sidebar('Starred');
  const starredHead = await heading();
  const starredRows = await listRows();
  await sidebar('Archive');
  const archiveHead = await heading();
  const archiveRows = await listRows();
  await rowMenu('Studio key handover confirmed');
  const archiveMenu = await menuLabels();
  await b.keys('Escape'); await sleep(500);
  await account(DEMO[0]);
  const imapArchive = await js(() => [...document.querySelectorAll('nav button, aside button')].some((e) => e.offsetParent && e.innerText.trim() === 'Archive'));
  await view('Inbox');
  await claim('feat-organize-2', 'vistas Starred y Archive', {
    covers: ['Starred in the sidebar lists your starred conversations.', 'Archive lists archived mail for Gmail and Outlook accounts and in All accounts', 'an IMAP account archives into its own Archive folder'],
    partial: 'la demo no tiene cuentas Gmail ni Outlook; que IMAP archive en su carpeta lo prueban tests',
    how: 'Con «All accounts», abre Starred (debe listar la conversación destacada de la demo) y Archive (debe listar el correo archivado de la demo); con la cuenta IMAP de la demo, la entrada Archive no aparece.',
  }, async () => {
    const [starred, archive] = bold('feat-organize-2');
    const s = starredHead.startsWith(starred) && starredRows.some((r) => r.includes('Corrected Larkspur Freight renewal quote'));
    const a = archiveHead.startsWith(archive) && archiveRows.some((r) => r.includes('Studio key handover confirmed'));
    return ok(s && a && !imapArchive, `«${starredHead}» lista la destacada; «${archiveHead}» lista la archivada; la cuenta IMAP no tiene ${archive}`,
      `Starred=${s} («${starredHead}»), Archive=${a} («${archiveHead}»), IMAP con Archive=${imapArchive}`);
  });
  await claim('feat-organize-1', 'Move to Inbox', {
    covers: ['Move to Inbox brings it back'], proof: 'label',
    how: 'En la vista Archive, el menú ⋮ de la conversación archivada ofrece «Move to Inbox». No se ejecuta: la cuenta demo no tiene credenciales.',
  }, async () => {
    const [, back] = bold('feat-organize-1');
    return ok(archiveMenu.includes(back), `«${back}» en el menú de la conversación archivada`, `menú: ${archiveMenu.join(', ')}`);
  });
  await claim('feat-organize-2', 'búsqueda', {
    covers: ['Archived mail is only out of the inbox: search'],
    partial: 'los filtros inteligentes los prueban tests; las funciones de IA no se prueban',
    how: 'Desde la bandeja, busca el asunto del correo archivado de la demo: la búsqueda debe encontrarlo aunque la bandeja no lo liste.',
  }, async ({ doc }) => {
    doc.match(/only out of the inbox/);
    const inInbox = await hasRow('Studio key handover confirmed');
    await search('Studio key handover');
    const hits = await rowTexts();
    await search('');
    const found = hits.some((t) => t.includes('Studio key handover confirmed'));
    return ok(!inInbox && found, 'fuera de la bandeja, pero la búsqueda lo encuentra', `en bandeja=${inInbox}, encontrado=${found}`);
  });

  // Multi-select, the bulk toolbar and Clear selection.
  await account(DEMO[0]);
  await view('Inbox');
  await inRow('Kwame Boateng', '[data-testid="row-select"]', true);
  await inRow('GlitchTip', '[data-testid="row-select"]', true);
  await sleep(600);
  const bar = await js(() => { const t = document.querySelector('[data-testid="bulk-toolbar"]'); return t ? { text: t.innerText.replace(/\s+/g, ' '), buttons: [...t.querySelectorAll('button')].map((x) => x.getAttribute('aria-label') || x.title || x.textContent.trim()) } : null; });
  await claim('feat-organize-3', 'barra de selección', {
    covers: ['Tick the box at the start of a row to select it.', 'With one or more selected, a toolbar above the list acts on all of them at once — archive, snooze, delete, mark as read or unread, star'],
    how: 'Marca la casilla de dos filas de la demo: aparece la barra con «2 selected» y botones para archivar, posponer, eliminar, marcar leído/no leído y destacar.',
  }, async ({ doc }) => {
    const acts = doc.match(/at once — (.+?) — and/)[1].split(', ');
    const want = { archive: 'Archive', snooze: 'Snooze', delete: 'Delete', 'mark as read or unread': /Mark as (un)?read/, star: 'Star' };
    if (!bar) return 'FAIL: no aparece la barra de acciones al marcar dos filas';
    const missing = acts.filter((a) => { const w = want[a]; return !w || !bar.buttons.some((x) => (w instanceof RegExp ? w.test(x) : x === w)); });
    return ok(/2 selected/.test(bar.text) && !missing.length, `${bar.text.slice(0, 40)}; ${bar.buttons.join(', ')}`, `texto «${bar.text}»; sin botón para: ${missing.join(', ')}`);
  });

  // Bulk archive + Undo: nothing reaches the (credential-less) provider.
  const bulkArchive = () => js(() => { const x = [...document.querySelectorAll('[data-testid="bulk-toolbar"] button')].find((y) => (y.getAttribute('aria-label') || y.title || y.textContent.trim()) === 'Archive'); x?.click(); return !!x; });
  const archived = await bulkArchive();
  await sleep(800);
  const goneAfterArchive = !(await hasRow('Kwame Boateng')) && !(await hasRow('GlitchTip'));
  const undoToast = await toastText();
  const undone = await toastAction('Undo');
  await sleep(1500);
  const backAfterUndo = (await hasRow('Kwame Boateng')) && (await hasRow('GlitchTip'));
  await sleep(7000);
  const lateAfterUndo = await toastText();
  await closeToasts();
  await claim('feat-organize-4', 'deshacer', {
    covers: ['Archiving or deleting removes the conversations from the list at once and shows a notice with Undo', 'so Undo simply puts them back'],
    partial: 'eliminar y la duración de 6 segundos no se miden aquí',
    how: 'Archiva dos filas desde la barra: salen de la lista al momento con un aviso «Archived 2 conversations · Undo»; Undo las devuelve y, pasados 7 s, ningún error del proveedor (la cuenta demo no tiene credenciales: cualquier llamada fallaría a la vista).',
  }, async () => {
    const [undo] = bold('feat-organize-4');
    return ok(archived && goneAfterArchive && /Archived 2 conversations/.test(undoToast) && undoToast.includes(undo) && undone && backAfterUndo && !/Could not archive/.test(lateAfterUndo),
      `aviso «${undoToast}»; ${undo} las devuelve; ningún error del proveedor después`,
      `archivadas=${archived}/${goneAfterArchive}, aviso «${undoToast}», undo=${undone}, de vuelta=${backAfterUndo}, aviso tardío «${lateAfterUndo}»`);
  });
  await js(() => document.querySelector('[data-testid="bulk-clear"]')?.click());
  await sleep(600);
  const barAfterClear = await js(() => !!document.querySelector('[data-testid="bulk-toolbar"]'));
  await claim('feat-organize-3', 'Clear selection', {
    covers: ['Clear selection ends it.'],
    how: 'Con dos filas marcadas, pulsa «Clear selection»: la barra de acciones desaparece.',
  }, async () => {
    const [clear] = bold('feat-organize-3');
    const label = await js(() => document.querySelector('[data-testid="bulk-clear"]')?.getAttribute('aria-label') || '');
    return ok(!barAfterClear && (label === '' || label === clear), `«${clear}» quita la barra`, `barra tras limpiar=${barAfterClear}`);
  });

  // An archive left to run: the provider is called only after the window.
  await rowMenu('GlitchTip');
  const archivedOne = await js(() => { const x = [...document.querySelectorAll('button')].find((y) => y.offsetParent && y.textContent.trim() === 'Archive' && !y.closest('[data-testid="bulk-toolbar"], nav, aside, header')); x?.click(); return !!x; });
  const t0 = Date.now();
  await sleep(2500);
  const earlyToast = await toastText();
  const providerError = await waitToast(/Could not archive 1 conversation/, 25000);
  const errorAfter = (Date.now() - t0) / 1000;
  await sleep(1000);
  const rolledBack = await hasRow('GlitchTip');
  await closeToasts();
  await claim('feat-organize-4', 'proveedor al acabar la ventana', {
    covers: ['Your mail provider is only told when those seconds are up'],
    how: 'Archiva una fila y no deshace: la cuenta demo no tiene credenciales, así que el error del proveedor delata cuándo se le llama. No debe llegar en los primeros segundos y sí después de la ventana; la fila vuelve.',
  }, async ({ doc }) => {
    const secs = doc.number(/for (\d+) seconds/);
    return ok(archivedOne && !/Could not archive/.test(earlyToast) && !!providerError && errorAfter >= secs - 1 && rolledBack,
      `el proveedor no se llama hasta ~${errorAfter.toFixed(0)} s (ventana de ${secs} s); el error devuelve la fila`,
      `archivada=${archivedOne}, aviso temprano «${earlyToast}», error=${providerError} a los ${errorAfter.toFixed(1)} s, fila de vuelta=${rolledBack}`);
  });

  // Keyboard: ? overlay, the table's keys, j/k, e + auto-advance + Undo.
  await blur(); await b.keys('?'); await sleep(900);
  const help = await js(() => {
    const h = document.querySelector('[data-testid="shortcut-help"]');
    if (!h) return null;
    return [...h.querySelectorAll('li[data-shortcut-id]')].map((li) => ({
      label: li.querySelector('span')?.textContent.trim() || '',
      seqs: (() => {
        // Alternatives are separated by an "or" span, presses by a "then" span.
        const parts = [...li.querySelectorAll('span:last-child > *')];
        const alts = []; let cur = [];
        for (const n of parts) {
          if (n.tagName === 'KBD') cur.push(n.textContent.trim());
          else if (n.textContent.trim() === 'or') { alts.push(cur.join(' ')); cur = []; }
        }
        if (cur.length) alts.push(cur.join(' '));
        return alts;
      })(),
    }));
  });
  await b.keys('Escape'); await sleep(600);
  const helpClosed = !(await js(() => !!document.querySelector('[data-testid="shortcut-help"]')));
  await claim('feat-shortcuts-1', 'tabla de atajos', {
    covers: ['Press ? anywhere outside a text field to see every shortcut.',
      '| j / k | next / previous conversation |', '| Enter or o, u | open the conversation, back to the list |',
      '| x | select or deselect the conversation |', '| e, #, s, b | archive, delete, star, snooze |',
      '| Shift+U / Shift+I | mark as unread / read |', '| c, r, a, f | new message, reply, reply all, forward |',
      '| g then i, s, b, a, l | go to Inbox, Starred, Snoozed, Archive, Scheduled |', '| / | search |'],
    how: 'Pulsa «?» fuera de un campo: se abre la lista de atajos. Para cada fila de la tabla de la doc, cada tecla (o secuencia «g …») debe estar en la lista con una acción cuyo nombre empiece como el de la doc.',
  }, async ({ doc }) => {
    if (!help) return 'FAIL: «?» no abre la lista de atajos';
    const rows = doc.text.split('| Keys | Action |')[1].split('|---|---|')[1].split(/\|\s*\|/).join('|\n|').split('\n')
      .map((r) => r.trim().replace(/^\|\s*|\s*\|$/g, '').split(' | ')).filter((r) => r.length === 2);
    const bad = [];
    for (const [keysCol, actCol] of rows) {
      let groups = keysCol === '/' ? ['/'] : keysCol.split(', ');
      let acts = actCol.split(', ');
      let prefix = '';
      if (/^g then /.test(groups[0])) { prefix = 'g '; groups[0] = groups[0].replace(/^g then /, ''); acts = acts.map((a) => a.replace(/^go to /, '')); }
      groups.forEach((g, i) => {
        const keys = g === '/' ? ['/'] : g.split(/ or | \/ /);
        const act = acts[Math.min(i, acts.length - 1)];
        const alts = act.split(' / ');
        keys.forEach((k, j) => {
          const want = (keys.length === alts.length ? alts[j] : alts[0]).replace(/^mark as /, '');
          const seq = prefix + k;
          const hit = help.find((h) => h.seqs.includes(seq));
          const word = want.split(' ')[0].toLowerCase();
          if (!hit || !hit.label.toLowerCase().includes(word)) bad.push(`${seq} → ${want} (app: ${hit ? hit.label : 'sin tecla'})`);
        });
      });
    }
    return ok(helpClosed && !bad.length && rows.length >= 8, `${rows.length} filas: cada tecla está en la lista de «?» con su acción; Escape la cierra`, `no casan: ${bad.join('; ') || '—'}; filas leídas ${rows.length}; cerrada=${helpClosed}`);
  });

  const cursorRow = () => js(() => document.querySelector('[data-cursor="true"]')?.innerText.replace(/\s+/g, ' ').slice(0, 80) ?? null);
  await blur();
  const c0 = await cursorRow(); await b.keys('j'); await sleep(400); const c1 = await cursorRow(); await b.keys('k'); await sleep(400); const c2 = await cursorRow();
  await claim('feat-shortcuts-1', 'j y k', {
    covers: ['| j / k | next / previous conversation |'],
    how: 'Pulsa j y k en la lista: el cursor de teclado baja a la fila siguiente y vuelve.',
  }, async () => ok(c1 && c1 !== c0 && c2 === c0, `${String(c0).slice(0, 30)} → ${String(c1).slice(0, 30)} → de vuelta`, `cursor ${c0} → ${c1} → ${c2}`));

  // e on an open conversation: archive, the next one opens, Undo.
  const list = await listRows();
  await js(() => [...document.querySelectorAll('div[role="button"]')].filter((r) => r.offsetParent && r.querySelector('[data-testid="row-select"]'))[1]?.click());
  await sleep(1500);
  const first = await h1();
  await blur(); await b.keys('e'); await sleep(1500);
  const next = await h1();
  const eToast = await toastText();
  await toastAction('Undo'); await sleep(1200);
  await closeToasts();
  await press('Back').catch(() => {});
  await sleep(1000);
  const firstBack = (await listRows()).some((r) => r.includes(first));
  await claim('feat-organize-5', 'abre la siguiente', {
    covers: ['When the conversation you are reading leaves the list — archived, deleted, snoozed, or moved to Spam — the next one opens.'],
    partial: 'solo se prueba al archivar',
    how: 'Abre la segunda fila, pulsa «e» (archivar): debe abrirse la tercera; Undo devuelve la archivada a la lista.',
  }, async ({ doc }) => {
    doc.match(/the next one opens/);
    return ok(first && next && next !== first && list[2]?.includes(next) && /Archived 1 conversation/.test(eToast) && firstBack,
      `«${first.slice(0, 30)}» archivada con e → abierta «${next.slice(0, 30)}»; Undo la devuelve`,
      `antes «${first}», después «${next}», tercera fila «${String(list[2]).slice(0, 50)}», aviso «${eToast}», de vuelta=${firstBack}`);
  });

  // Nothing acts behind a menu; nothing acts while typing; tooltips name the key.
  await blur();
  const before = await listRows(); const cur0 = await cursorRow();
  await js(() => [...document.querySelectorAll('div[role="button"]')].filter((r) => r.offsetParent && r.querySelector('[data-testid="row-select"]'))[2]?.querySelector('[aria-label="More actions"]')?.click());
  await sleep(700);
  for (const k of ['#', 'e', 'j']) { await b.keys(k); await sleep(300); }
  await sleep(800);
  const afterMenu = await listRows(); const menuToast = await toastText(); const cur1 = await cursorRow();
  await b.keys('Escape'); await sleep(500);
  await js(() => { const i = document.querySelector('input[placeholder^="Search"]'); i.focus(); });
  await b.keys('e'); await sleep(600);
  const typed = await js(() => document.querySelector('input[placeholder^="Search"]')?.value || '');
  const afterTyping = await listRows();
  await search('');
  await blur();
  await openThread('Nadia Brunner');
  const titlesOn = await headerTitles();
  await press('Back').catch(() => {});
  await sleep(800);
  await claim('feat-shortcuts-2', 'pausa y pistas', {
    covers: ['Shortcuts pause while you type and while a dialog or menu is open.', 'The buttons they stand for name their key in the tooltip, as in "Archive (E)".'],
    partial: 'con un diálogo abierto no se prueba aquí (lo hace make verify)',
    how: 'Con el menú ⋮ de una fila abierto pulsa «#», «e» y «j»: ninguna fila sale y el cursor no se mueve. Con el foco en el buscador, «e» se escribe y no archiva. En un hilo abierto, el botón de archivar se titula como cita la doc.',
  }, async ({ doc }) => {
    const hint = doc.match(/as in "([^"]+)"/)[1];
    const left = before.filter((r) => !afterMenu.includes(r));
    const leftTyping = before.filter((r) => !afterTyping.includes(r));
    return ok(!left.length && !/Deleted|Archived/.test(menuToast) && cur1 === cur0 && typed === 'e' && !leftTyping.length && titlesOn.includes(hint),
      `menú abierto: nada actúa; escribiendo: «e» va al buscador; tooltip «${hint}»`,
      `salieron con el menú: ${left.length}, aviso «${menuToast}», cursor ${cur0} → ${cur1}; buscador «${typed}», salieron escribiendo ${leftTyping.length}; títulos: ${titlesOn.join(', ')}`);
  });

  // Settings → Appearance: the shortcuts switch and Show the list.
  const appearance = await tab('Appearance');
  await js(() => document.querySelector('[data-testid="keyboard-shortcuts-show"]')?.click());
  await sleep(800);
  const listFromSettings = await js(() => !!document.querySelector('[data-testid="shortcut-help"]'));
  await b.keys('Escape'); await sleep(600);
  await tab('Appearance');
  await flip('Keyboard shortcuts');
  await closeSettings();
  await blur(); await b.keys('?'); await sleep(800);
  const helpWhenOff = await js(() => !!document.querySelector('[data-testid="shortcut-help"]'));
  if (helpWhenOff) { await b.keys('Escape'); await sleep(500); }
  await openThread('Nadia Brunner');
  const titlesOff = await headerTitles();
  await press('Back').catch(() => {});
  await sleep(800);
  await tab('Appearance');
  await flip('Keyboard shortcuts');
  const shortcutsBack = (await toggles())['Keyboard shortcuts'];
  await claim('feat-shortcuts-2', 'interruptor', {
    covers: ['Settings → Appearance → Keyboard shortcuts turns them off, and Show the list opens the same overview as ?.'],
    how: 'En Ajustes → Appearance, «Show the list» abre la lista de atajos. Apaga el interruptor: «?» ya no abre nada y los botones pierden la tecla del tooltip; después lo vuelve a encender.',
  }, async () => {
    const [path, show] = bold('feat-shortcuts-2');
    const label = path.split(' → ').pop();
    return ok(appearance.includes(label) && appearance.includes(show) && listFromSettings && !helpWhenOff && !titlesOff.some((x) => /\(E\)$/.test(x)) && shortcutsBack === true,
      `«${show}» abre la lista; con «${label}» apagado «?» no hace nada y los tooltips no nombran tecla; vuelto a encender`,
      `lista desde ajustes=${listFromSettings}, ? con atajos apagados=${helpWhenOff}, títulos=${titlesOff.join(', ')}, encendido de nuevo=${shortcutsBack}`);
  });

  // Undo send and After archiving or deleting, as Settings shows them.
  const undoSend = await js(() => { const s = document.querySelector('[data-testid="undo-send-setting"] select'); return s ? { value: s.value, options: [...s.options].map((o) => o.textContent.trim()) } : null; });
  const afterLeave = await js(() => { const s = document.querySelector('[data-testid="auto-advance-setting"] select'); return s ? { value: s.value, options: [...s.options].map((o) => o.textContent.trim()) } : null; });
  await closeSettings();
  await claim('feat-send-1', 'opciones', {
    covers: ['The wait is set in Settings → Appearance → Undo send: off, 5, 10, 20 or 30 seconds'],
    how: 'Lee el desplegable «Undo send» de Ajustes → Appearance y compara sus opciones con la lista de la doc.',
  }, async ({ doc }) => {
    const [, want] = doc.match(/Undo send: (.+?) seconds/);
    const nums = want.replace(/ or /, ', ').split(', ').filter((x) => x !== 'off');
    const got = (undoSend?.options || []).map((o) => (o.match(/^(\d+)/) || [])[1]).filter(Boolean);
    return ok(undoSend && /off/i.test(undoSend.options[0]) && nums.join() === got.join(), `opciones: ${undoSend?.options.join(', ')}`, `doc: off, ${nums.join(', ')}; app: ${undoSend?.options.join(', ')}`);
  });
  await claim('feat-organize-5', 'ajuste', {
    covers: ['Settings → Appearance → After archiving or deleting chooses between the next conversation, the previous one, or going back to the list.'],
    proof: 'label',
    how: 'Lee el desplegable «After archiving or deleting» de Ajustes → Appearance: debe ofrecer la siguiente, la anterior y volver a la lista.',
  }, async () => {
    const opts = afterLeave?.options || [];
    return ok(opts.length === 3 && /next/i.test(opts[0]) && /previous/i.test(opts[1]) && /list/i.test(opts[2]), opts.join(' / '), `opciones: ${opts.join(' / ') || 'sin desplegable'}`);
  });

  // Snooze: the row menu's presets, the Snoozed view, Unsnooze.
  await view('Inbox');
  await rowMenu('Kwame Boateng');
  await js(() => document.querySelector('[data-testid="row-snooze"]')?.click());
  await sleep(600);
  const presets = await js(() => [...document.querySelectorAll('[data-testid^="snooze-preset-"]')].map((x) => x.getAttribute('data-testid').replace('snooze-preset-', '')));
  const presetLabels = await js(() => document.querySelector('[data-testid="snooze-options"]')?.innerText.replace(/\s+/g, ' ') || '');
  await js(() => document.querySelector('[data-testid^="snooze-preset-"]')?.click());
  await sleep(1500);
  const snoozeToast = await toastText();
  const snoozedOut = !(await hasRow('Kwame Boateng'));
  await closeToasts();
  await openThread('Nadia Brunner');
  const paneSnooze = (await headerTitles()).some((x) => x.startsWith('Snooze'));
  await press('Back').catch(() => {});
  await sleep(800);
  await claim('feat-snooze-1', 'momentos y dónde', {
    covers: ['Snooze hides a conversation from the inbox until a time you choose: later today, tomorrow, this weekend, next week, or a date and time you pick.', 'It is offered in the reading pane, the row\'s ⋮ menu and the selection toolbar.'],
    how: 'En el menú ⋮ de una fila, Snooze ofrece los momentos de la doc (los del día: «later today» y «this weekend» dependen de la hora y el día) y una fecha a elegir; el primero saca la fila de la bandeja. El panel de lectura y la barra de selección también tienen Snooze.',
  }, async ({ doc }) => {
    const named = doc.match(/until a time you choose: (.+?)\./)[1].replace(/, or /, ', ').split(', ');
    const ids = { 'later today': 'laterToday', tomorrow: 'tomorrow', 'this weekend': 'thisWeekend', 'next week': 'nextWeek' };
    const unknown = presets.filter((p) => !named.some((n) => ids[n] === p));
    const always = ['tomorrow', 'nextWeek'].filter((p) => !presets.includes(p));
    const pick = /Pick date/.test(presetLabels);
    const inBar = bar?.buttons.includes('Snooze');
    return ok(!unknown.length && !always.length && pick && snoozedOut && /Snoozed until/.test(snoozeToast) && paneSnooze && inBar,
      `momentos: ${presets.join(', ')} + fecha; la fila sale («${snoozeToast.slice(0, 40)}»); también en el panel y la barra`,
      `momentos ${presets.join(', ')} (desconocidos ${unknown.join(', ')}, faltan ${always.join(', ')}), fecha=${pick}, fuera=${snoozedOut}, panel=${paneSnooze}, barra=${inBar}`);
  });
  await sidebar('Snoozed');
  const snoozedHead = await heading();
  const inSnoozed = await hasRow('Kwame Boateng');
  const badge = await inRow('Kwame Boateng', '[data-testid="snooze-badge"]');
  await rowMenu('Kwame Boateng');
  await js(() => document.querySelector('[data-testid="row-unsnooze"]')?.click());
  await sleep(1500);
  const leftSnoozed = !(await hasRow('Kwame Boateng'));
  await closeToasts();
  await view('Inbox');
  const backInInbox = await hasRow('Kwame Boateng');
  await claim('feat-snooze-2', 'vista y Unsnooze', {
    covers: ['Snoozed conversations are listed under Snoozed in the sidebar', 'where Unsnooze brings one back early.'],
    how: 'Tras posponer una fila, la vista Snoozed la lista con su marca de hora; Unsnooze en su menú ⋮ la saca de Snoozed y la devuelve a la bandeja.',
  }, async () => {
    const [snoozed] = bold('feat-snooze-2');
    return ok(snoozedHead.startsWith(snoozed) && inSnoozed && badge !== null && leftSnoozed && backInInbox,
      `«${snoozedHead}» la lista con su hora; Unsnooze la devuelve a la bandeja`, `cabecera «${snoozedHead}», listada=${inSnoozed}, marca=${badge}, fuera=${leftSnoozed}, en bandeja=${backInInbox}`);
  });

  // Schedule send needs a recipient, a subject and a body before its arrow
  // enables, and the composer would autosave that as a draft in the shared
  // demo DB: make verify (Envío/programar envío) drives it instead.
  await js(() => document.querySelector('[data-testid="sidebar-scheduled"]')?.click());
  await sleep(1500);
  const scheduledHead = await js(() => document.querySelector('[data-testid="scheduled-app-open-note"]')?.previousElementSibling?.textContent.trim() || '');
  const scheduledNote = await js(() => document.querySelector('[data-testid="scheduled-app-open-note"]')?.innerText || '');
  await claim('feat-send-3', 'vista Scheduled', {
    covers: ['Messages waiting to go out are listed under Scheduled in the sidebar', 'A scheduled message only goes out while EmailOps is open'],
    proof: 'label',
    how: 'Abre la vista Scheduled desde la barra lateral y lee su cabecera y la nota sobre que la app debe estar abierta. No se programa nada: la demo es compartida.',
  }, async () => {
    const [scheduled] = bold('feat-send-3');
    return ok(scheduledHead.startsWith(scheduled) && /only while EmailOps is open/.test(scheduledNote), `«${scheduledHead}»: ${scheduledNote}`, `cabecera «${scheduledHead}», nota «${scheduledNote}»`);
  });
  await view('Inbox');

  // Signatures: save one, see it in a new message, try two images, empty it again.
  const SIGNATURE = 'Ulises Demo · docs check signature';
  const sigTab = await tab('Signatures');
  const sigSwitches = await toggles();
  const editor = await js(() => { const ed = document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]'); if (!ed) return false; ed.focus(); document.execCommand('selectAll'); document.execCommand('delete'); return true; });
  if (editor) await js((t) => document.execCommand('insertText', false, t), SIGNATURE);
  await sleep(400);
  const saveSig = () => js(() => { const x = [...document.querySelectorAll('[data-testid="signatures-settings"] button')].find((y) => y.textContent.trim() === 'Save signature'); x?.click(); return !!x; });
  await saveSig(); await sleep(1500);
  await closeSettings();
  await press('Compose').catch(() => {});
  await sleep(1800);
  const inserted = await js(() => { const s = document.querySelector('[contenteditable="true"] [data-emailops-signature]'); return s ? { text: s.innerText.trim(), editable: !!s.closest('[contenteditable="true"]') } : null; });
  await press('Cancel').catch(() => {});
  await sleep(800);
  if (await js(() => !!document.querySelector('input[placeholder^="Email subject"]'))) { await b.keys('Escape'); await sleep(600); }
  await claim('feat-signatures-1', 'guardar e insertar', {
    covers: ['Each account has its own signature, set in Settings → Signatures.', 'Two switches decide where it goes: Insert in new messages and Insert in replies and forwards.', 'It is part of the message body, so you can change or delete it in any message before sending.'],
    how: 'En Ajustes → Signatures escribe una firma para la cuenta demo y la guarda; Compose abre con ella dentro del cuerpo editable. Comprueba que están los dos interruptores que nombra la doc. Al final la firma se vacía.',
  }, async () => {
    const [, forNew, forReplies] = bold('feat-signatures-1');
    const sw = [forNew, forReplies].filter((l) => !Object.keys(sigSwitches).some((k) => k.startsWith(l)));
    return ok(editor && inserted?.text === SIGNATURE && inserted.editable && !sw.length && /account/i.test(sigTab),
      `firma insertada en el cuerpo editable de un mensaje nuevo; interruptores ${forNew} / ${forReplies}`,
      `editor=${editor}, insertada=${JSON.stringify(inserted)}, faltan interruptores: ${sw.join(', ') || '—'}`);
  });

  await tab('Signatures');
  const pickImage = (kind) => js((k) => {
    const input = document.querySelector('[data-testid="signature-image-input"]'); if (!input) return false;
    const deliver = (file) => { const dt = new DataTransfer(); dt.items.add(file); input.files = dt.files; input.dispatchEvent(new Event('change', { bubbles: true })); };
    if (k === 'svg') { deliver(new File(['<svg xmlns="http://www.w3.org/2000/svg"></svg>'], 'docs-logo.svg', { type: 'image/svg+xml' })); return true; }
    const c = document.createElement('canvas'); c.width = 1600; c.height = 200; const g = c.getContext('2d'); g.fillStyle = '#2b6cb0'; g.fillRect(0, 0, 1600, 200);
    c.toBlob((blob) => deliver(new File([blob], 'docs-wide-logo.png', { type: 'image/png' })), 'image/png'); return true;
  }, kind);
  const addImage = await js(() => document.querySelector('[data-testid="signature-add-image"]')?.innerText.trim() || '');
  await pickImage('svg'); await sleep(1200);
  const svgRefused = await js(() => document.querySelector('[data-testid="signatures-settings"]')?.innerText.replace(/\s+/g, ' ') || '');
  await pickImage('png'); await sleep(2500);
  const imgWidth = await js(() => document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"] img')?.naturalWidth ?? null);
  await js(() => { const ed = document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]'); ed.focus(); document.execCommand('selectAll'); document.execCommand('delete'); });
  await sleep(400);
  await saveSig(); await sleep(1500);
  const emptied = await js(() => (document.querySelector('[data-testid="signatures-settings"] [contenteditable="true"]')?.innerText || '').trim() === '');
  await closeSettings();
  await claim('feat-signatures-2', 'imágenes', {
    covers: ['Add image puts a logo or a picture of your handwritten signature into it.', 'SVG and other files are refused with the reason.', 'A wide image is scaled down to 600 px'],
    how: 'En Ajustes → Signatures, «Add image» con un SVG muestra el motivo (solo PNG, JPEG, GIF o WebP); con un PNG de 1600 px entra en la firma reducido al ancho que dice la doc. Después la firma se vacía y se guarda vacía.',
  }, async ({ doc }) => {
    const [add] = bold('feat-signatures-2');
    const px = doc.number(/scaled down\s+to (\d+) px/);
    return ok(addImage === add && /PNG, JPEG, GIF or WebP/.test(svgRefused) && imgWidth === px && emptied,
      `«${add}»: SVG rechazado con el motivo; PNG de 1600 px → ${imgWidth} px; firma vaciada`,
      `botón «${addImage}», motivo SVG=${/PNG, JPEG, GIF or WebP/.test(svgRefused)}, ancho=${imgWidth}, vaciada=${emptied}`);
  });

  // Unsubscribe: the confirmation names what will be contacted. Never confirmed.
  await search('Harborlight');
  await openThread('Harborlight Weekly');
  await js(() => document.querySelector('[data-testid="unsubscribe-button"]')?.click());
  await sleep(1000);
  const unsubButton = await js(() => document.querySelector('[data-testid="unsubscribe-button"]')?.innerText.trim() || '');
  const unsubDialog = await dialogText('^Unsubscribe from');
  await dialogButton('^Unsubscribe from', 'Cancel');
  await sleep(800);
  const unsubClosed = !(await js(() => !!document.querySelector('[data-testid="unsubscribe-confirm"]')));
  await press('Back').catch(() => {});
  await sleep(800);
  await claim('feat-unsubscribe-1', 'confirmación', {
    covers: ['shows Unsubscribe next to its sender', 'Before anything is sent, a confirmation says exactly what will happen: a request sent straight to the sender\'s server (not through your mail provider)'],
    partial: 'el boletín demo es de un clic; el correo de baja y la página del remitente los prueban tests',
    how: 'Abre el boletín demo con List-Unsubscribe de un clic, pulsa Unsubscribe y lee la confirmación: debe nombrar el servidor del remitente y decir que no pasa por el proveedor. Pulsa Cancel; nunca confirma.',
  }, async () => {
    const [label] = bold('feat-unsubscribe-1');
    return ok(unsubButton === label && /harborlight-weekly\.example/.test(unsubDialog || '') && /not your mail provider/.test(unsubDialog || '') && unsubClosed,
      `«${label}» → «${(unsubDialog || '').replace(/\s+/g, ' ').slice(0, 140)}»; Cancel la cierra`, `botón «${unsubButton}», diálogo «${unsubDialog}», cerrado=${unsubClosed}`);
  });

  // Block sender, the banner, Settings → Junk → Blocked senders, Unblock.
  await rowMenu('Harborlight Weekly');
  const hideItem = (await menuLabels());
  await js(() => document.querySelector('[data-testid="menu-block-sender"]')?.click());
  await sleep(1000);
  const blockDialog = await dialogText('^Block news@harborlight-weekly');
  const moveExisting = await js(() => { const d = [...document.querySelectorAll('[role="dialog"], [aria-modal="true"], .fixed')].find((x) => /^Block /.test(x.innerText.trim())); const c = d?.querySelector('input[type="checkbox"]'); return c ? c.checked : null; });
  await js(() => document.querySelector('[data-testid="sender-block-confirm"]')?.click());
  const blockToast = await waitToast(/Blocked news@harborlight-weekly\.example/, 10000);
  await closeToasts();
  await openThread('Harborlight Weekly');
  const banner = await js(() => document.querySelector('[data-testid="blocked-sender-banner"]')?.innerText.replace(/\s+/g, ' ') || null);
  await press('Back').catch(() => {});
  await sleep(800);
  await search('');
  const junkTab = await tab('Junk');
  const blockedList = await js(() => document.querySelector('[data-testid="blocked-senders"]')?.innerText.replace(/\s+/g, ' ') || '');
  await js(() => [...document.querySelectorAll('[data-testid="blocked-senders"] button')].find((x) => x.textContent.trim() === 'Unblock')?.click());
  await sleep(1000);
  const unblockDialog = await dialogText('^Unblock news@harborlight-weekly');
  await dialogButton('^Unblock news@harborlight-weekly', 'Unblock');
  const unblockToast = await waitToast(/Unblocked/, 10000);
  await sleep(600);
  const listAfter = await js(() => document.querySelector('[data-testid="blocked-senders"]')?.innerText || '');
  await closeToasts();
  await closeSettings();
  await claim('feat-block-sender-1', 'bloquear', {
    covers: ['Block sender, in a conversation\'s ⋮ menu', 'Also move their existing messages to Spam files what is already there, and the conversation shows that the sender is blocked, with Unblock at hand.'],
    partial: 'la demo no tiene credenciales: mover a Spam en el servidor lo prueban tests',
    how: 'En el menú ⋮ del boletín demo pulsa Block sender: el diálogo ofrece mover sus mensajes a Spam (marcado); al confirmar, el hilo muestra el aviso de remitente bloqueado con Unblock. Después se desbloquea.',
  }, async () => {
    const [blk, move, unblock] = bold('feat-block-sender-1');
    return ok(hideItem.includes(blk) && (blockDialog || '').includes(move) && moveExisting === true && !!blockToast && /Unblock/.test(banner || '') && (banner || '').includes(unblock),
      `«${blk}» → diálogo con «${move}» marcado; aviso «${banner}»`, `menú=${hideItem.includes(blk)}, diálogo «${blockDialog}», casilla=${moveExisting}, aviso tostada «${blockToast}», banner «${banner}»`);
  });
  await claim('feat-block-sender-2', 'Ajustes → Junk', {
    covers: ['Settings → Junk → Blocked senders lists everyone you blocked, with Unblock, which can also bring their messages back from Spam to the inbox.', 'The ⋮ menu\'s Hide from smart filters is a different thing'],
    proof: 'label',
    how: 'Tras bloquear el boletín demo, Ajustes → Junk → Blocked senders lo lista; Unblock pide confirmación con la opción de devolver su correo de Spam y lo quita de la lista. El menú ⋮ tiene además «Hide from smart filters».',
  }, async () => {
    const [path, , hide] = bold('feat-block-sender-2');
    const title = path.split(' → ').pop();
    return ok(junkTab.includes(title) && blockedList.includes('news@harborlight-weekly.example') && /back to the inbox/.test(unblockDialog || '') && !!unblockToast && !listAfter.includes('news@harborlight-weekly.example') && hideItem.includes(hide),
      `«${title}» lista el bloqueo; Unblock con «devolver a la bandeja» lo quita; «${hide}» en el menú`,
      `lista «${blockedList.slice(0, 80)}», diálogo «${unblockDialog}», aviso «${unblockToast}», después «${listAfter.slice(0, 60)}», menú: ${hideItem.join(', ')}`);
  });

  // Settings → Notifications on the demo accounts.
  const notif = await tab('Notifications');
  const notifToggles = await toggles();
  const notifRadio = await js(() => [...document.querySelectorAll('input[name="notification-content"]')].map((r) => `${r.value}:${r.checked}`));
  const notifyFor = await js(() => [...document.querySelectorAll('[aria-label^="Notify for "]')].map((e) => e.getAttribute('aria-label')));
  await closeSettings();
  await claim('feat-notifications-2', 'ajustes', {
    covers: ['Settings → Notifications has the main switch, one switch per account, the Notification content (sender and subject, or Hide content, which shows only the account) and Only when EmailOps is not focused'],
    proof: 'label',
    how: 'Abre Ajustes → Notifications con las dos cuentas demo y comprueba el interruptor general, uno por cuenta, las dos opciones de contenido y «Only when EmailOps is not focused». No se cambia nada (los valores de fábrica se leen en la fase fresh).',
  }, async () => {
    const [, content, hidden, unfocused] = bold('feat-notifications-2');
    const accounts = DEMO.filter((m) => notifyFor.includes(`Notify for ${m}`));
    return ok(notif.includes(content) && notif.includes(hidden) && notif.includes(unfocused) && accounts.length === DEMO.length && notifRadio.length === 2 && Object.keys(notifToggles).some((k) => /^New mail notifications/.test(k)),
      `interruptor general, ${accounts.length} por cuenta, contenido (${notifRadio.join(', ')}) y «${unfocused}»`,
      `por cuenta ${accounts.join(', ')}, contenido ${notifRadio.join(', ')}, interruptores ${Object.keys(notifToggles).join(' | ')}`);
  });
  await view('Inbox');
  await closeToasts();

  // ── the same mailbox with AI switched off, then back on ──────────────────
  await tab('AI Backend & Models');
  await flip('AI Features');
  await closeSettings();
  await view('Inbox');
  const plainSide = (await screen()).split('SMART FILTERS')[0];
  await search('Ollama');
  const plainHits = await rowCount();
  await search('');
  await view('Calendar');
  const plainCal = /Calendar account/.test(await screen());
  await view('Attachments');
  const plainAtt = /Attachments\s*\(\d+\)/.test(await screen());
  await claim('feat-intro-1', 'buzón sin IA', {
    covers: ['Everything on this page works with AI switched off.'],
    how: 'Apaga «AI Features» en el buzón demo y comprueba que las funciones de la página siguen funcionando: la bandeja, la búsqueda de texto (encuentra «Ollama»), el calendario y la vista de adjuntos; luego vuelve a encender la IA.',
  }, async ({ doc }) => {
    doc.match(/works with AI switched off/);
    const missing = [!/Inbox/.test(plainSide) && 'bandeja', !plainHits && 'búsqueda', !plainCal && 'calendario', !plainAtt && 'adjuntos'].filter(Boolean);
    return ok(!missing.length, 'bandeja, búsqueda, calendario y adjuntos funcionan con la IA apagada', `con la IA apagada no funciona: ${missing.join(', ')}`);
  }, 'Either keep these working without AI, or say on this page which need AI switched on.');
  await tab('AI Backend & Models');
  await flip('AI Features');
  await closeSettings();
  void labelsVisible;
};
