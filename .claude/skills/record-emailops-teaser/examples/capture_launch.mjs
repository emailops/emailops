/**
 * Captures every frame folder examples/launch_teaser.py reads, in one run, on a
 * scratch copy of the demo made with scripts/scratch_demo.py + emails.example.json.
 *
 *   node .claude/skills/record-emailops-teaser/examples/capture_launch.mjs <work_dir>
 *
 * Takes ~6 minutes (three local-model answers). Check each answer it prints:
 * if one is wrong or leaks internals, re-run that part (see SKILL.md, step 3).
 */
import { mkdirSync } from 'node:fs';
import { openTeaser } from '../scripts/teaser-capture.mjs';

const W = process.argv[2];
const dirs = ['frames-app', 'frames4', 'frames5', 'frames6', 'frames7', 'frames10'];
for (const d of dirs) mkdirSync(`${W}/${d}`, { recursive: true });

async function section(dir, fn) {
  const ui = await openTeaser({ dir: `${W}/${dir}` });
  try { await fn(ui, ui.browser); } finally { await ui.finish(); }
}

// 1. Unified inbox with an empty chat; the question typed; the streamed answer.
await section('frames10', async (ui) => {
  await ui.closeReleaseNotes();
  await ui.goInbox();
  await ui.openChat();
  await ui.selectAllAccounts();
  await ui.shot('u00-empty-chat');
  await ui.typeFrames('textarea', "What's the status of the bug with orders stuck in processing?", 'u01-type');
  await ui.sendChat();
  await ui.chatFrames('u02-wait');
  await ui.shot('u03-answer');
  console.log('CHAT ANSWER:\n', await ui.lastAnswer());
});

// 2. ⌘K palette, empty then searching "invoice".
await section('frames-app', async (ui, b) => {
  const close = await b.$('aria/Close chat panel');
  if (await close.isExisting()) { await close.click(); await ui.pause(800); }
  await ui.goInbox();
  await b.keys(['Meta', 'k']); await ui.pause(1200);
  await ui.shot('k01-palette');
  await b.execute(() => {
    const el = document.activeElement;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, 'invoice');
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await ui.pause(2500);
  await ui.shot('k02-palette-invoice');
  await b.keys(['Escape']);
});

// 3. AI Draft from a typed idea: empty reply, idea typed, Generating…, the draft.
await section('frames4', async (ui, b) => {
  await ui.goInbox();
  await (await b.$('button=All accounts')).click(); await ui.pause(2000);
  await ui.visibleRow('Kwame Boateng', 'kwame'); await ui.clickTarget(); await ui.pause(2500);
  if (await ui.bodyHas('Discard') || await ui.bodyHas('Regenerate')) { await ui.clickText('Cancel'); await ui.pause(1200); }
  await ui.clickText('Reply'); await ui.pause(1500); await ui.dismissBanner();
  await ui.shot('i00-empty');
  await ui.typeFrames('input[placeholder^="Tell the AI"]', 'Yes, Ollama works. Explain where to set it up and offer help.', 'i01-type');
  await ui.rectOf('gen', '^Generate with AI$'); await ui.clickTarget();
  await ui.draftFrames('i03-gen');
  console.log('DRAFT:\n', await b.execute(() => document.querySelector('[contenteditable=true]')?.innerText));
});

// 4. The finished draft, the send row, the classified inbox and the views.
await section('frames5', async (ui, b) => {
  await ui.shot('r-draft');
  await b.execute(() => {
    const btn = [...document.querySelectorAll('button')].find((e) => /^Send Reply$/.test(e.innerText.trim()));
    let el = btn?.parentElement;
    while (el) { if (el.scrollHeight > el.clientHeight + 5 && /auto|scroll/.test(getComputedStyle(el).overflowY)) el.scrollTop += 400; el = el.parentElement; }
  });
  await ui.pause(600); await ui.shot('s01-send-visible');
  await ui.goInbox(); await (await b.$('button=All accounts')).click(); await ui.pause(2500); await ui.dismissBanner();
  await ui.shot('v00-inbox');
  await ui.reveal('Tag Board'); await ui.clickText('Tag Board'); await ui.pause(3000); await ui.dismissBanner();
  await ui.shot('v01-tag-company');
  for (const g of ['Intent', 'Topic']) { await ui.clickText(g); await ui.pause(2500); await ui.shot(`v02-tag-${g.toLowerCase()}`); }
  await ui.reveal('Attachments'); await ui.clickText('Attachments'); await ui.pause(3000); await ui.shot('v03-attachments');
  await ui.reveal('Calendar'); await ui.clickText('Calendar'); await ui.pause(3000); await ui.shot('v04-calendar');
});

// 5. Translation of the synthetic German email, and the sidebar tag filters.
await section('frames6', async (ui, b) => {
  await ui.goInbox(); await (await b.$('button=All accounts')).click(); await ui.pause(2500); await ui.dismissBanner();
  const meta = await ui.sidebarScrolls('t12-sb');
  console.log('SIDEBAR', JSON.stringify(meta));
  await ui.goInbox(); await ui.dismissBanner();
  await ui.visibleRow('Jonas Weber', 'jonas'); await ui.clickTarget(); await ui.pause(2000);
  for (let i = 0; i < 90 && !(await ui.bodyHas('This email is in')); i++) await ui.pause(1000);
  await ui.dismissBanner(); await ui.shot('t21-detected');
  await ui.rectOf('translate', '^Translate$'); await ui.clickTarget();
  for (let i = 0; i < 180 && !(await ui.bodyHas('Show original')); i++) await ui.pause(1000);
  await ui.pause(1000); await ui.dismissBanner(); await ui.shot('t25-translated');
});

// 6. A long EO Doc with headings, and the share dialog filled (never sent).
await section('frames7', async (ui, b) => {
  await ui.reveal('EO Docs'); await ui.clickText('EO Docs'); await ui.pause(2500);
  await ui.clickText('New'); await ui.pause(1200);
  await b.execute(() => {
    const el = document.querySelector('input[placeholder="Title"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, 'Proposal: Kanzlei Weber setup');
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await ui.clickText('Create'); await ui.pause(2500);
  await ui.pasteHtml('<h2>Kanzlei Weber: local AI setup</h2><p>Proposal for a six-person tax office in Leipzig. All AI runs on the office computers; no client email leaves the building.</p>'
    + '<h2>Scope</h2><ul><li>6 Windows workstations with EmailOps and the built-in AI model.</li><li>Two shared mailboxes and one personal account per person.</li></ul>'
    + '<h2>Day 1: Installation</h2><ul><li>Install EmailOps, connect the accounts and index the mailboxes.</li><li>Check GPU drivers so the local model runs at full speed.</li></ul>'
    + '<h2>Day 2: Workflows and training</h2><ul><li>Classification rules for clients, invoices and tax notices.</li><li>Drafts, translation and search: a two-hour team session.</li></ul>'
    + '<h2>Price</h2><p>Fixed price for two days of consulting, travel included.</p>');
  await ui.pause(2500); await ui.dismissBanner();
  await ui.shot('e01b-before-share');
  await ui.rectOf('share', '^Share$'); await ui.clickTarget(); await ui.pause(1500);
  await b.execute(() => {
    const ta = [...document.querySelectorAll('textarea')].find((e) => e.getBoundingClientRect().width > 0);
    ta.focus();
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(ta, 'jonas@kanzlei-weber.example');
    ta.dispatchEvent(new Event('input', { bubbles: true }));
    const cb = [...document.querySelectorAll('input[type=checkbox]')].find((e) => e.getBoundingClientRect().width > 0 && e.getBoundingClientRect().x > 500);
    if (cb && !cb.checked) cb.click();
  });
  await ui.pause(800); await ui.shot('e02-share-filled');
  await ui.clickText('Cancel');   // never press Share: the demo accounts cannot send
});

console.log(`done. Stitch the sidebar strip next:
  uv run --no-project --with pillow python .claude/skills/record-emailops-teaser/scripts/stitch_strip.py \\
    ${W}/frames6 t12-sb <y0> <y1> ${W}/frames6/sidebar-strip.png <tops from SIDEBAR above>`);
