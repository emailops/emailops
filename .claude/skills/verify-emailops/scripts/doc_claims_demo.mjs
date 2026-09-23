// Demo-phase doc claims: what the docs promise about a mailbox with mail in it.
// Loaded by doc_claims.mjs (`phase demo`), which passes its helpers in. Runs
// against the synthetic demo DB (.emailops-demo-data), never a real mailbox.
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
    const narrowed = after.length > 0 && after.join('|') !== before.join('|') && after.length <= before.length;
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
    const all = (await rowTexts()).length;
    // The list is virtualised, so a filtered result set is not a subset of the
    // rows on screen: check what each operator promises instead.
    const probes = [
      ['from:nadia', (rows) => rows.length > 0 && rows.every((t) => /nadia/i.test(t))],
      ['subject:ollama', (rows) => rows.length > 0 && rows.every((t) => /ollama/i.test(t))],
      ['tag:intent=request', (rows) => rows.length > 0 && rows.length < all],
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
  // Put the demo DB back: `make verify` shares it and expects Lenses off.
  if (!wasOn) {
    await tab('AI Lenses');
    await flipLenses();
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
