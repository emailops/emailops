/**
 * Capture helpers for teasers, on top of record-emailops-demo's emailops-ui.mjs.
 *
 *   import { openTeaser } from '<skill>/scripts/teaser-capture.mjs';
 *   const ui = await openTeaser({ dir: 'frames10' });
 *   await ui.closeReleaseNotes();
 *   await ui.goInbox();
 *   await ui.openChat();
 *   await ui.selectAllAccounts();          // real click + verified, after openChat
 *   await ui.shot('u00-empty-chat');
 *   await ui.typeFrames('textarea', 'What is …?', 'u01-type');   // letter by letter, a frame every 2 chars
 *   await ui.sendChat(); await ui.chatFrames('u02-wait'); await ui.shot('u03-answer');
 *   await ui.finish();
 *
 * Every helper here exists because the obvious way failed during the first
 * teaser; see reference/gotchas.md.
 */
import { openApp } from '../../record-emailops-demo/scripts/emailops-ui.mjs';

export async function openTeaser(opts) {
  const ui = await openApp(opts);
  const b = ui.browser;

  /** The "What's new in …" dialog appears after every version bump. */
  async function closeReleaseNotes() {
    await ui.clickText('Got it');
    await ui.pause(500);
  }

  /** "All accounts" must be really selected (highlighted) for a unified-inbox shot.
   *  A JS .click() changes the title to "Inbox — All accounts" but leaves the
   *  work account highlighted; opening the chat afterwards re-selects it too.
   *  So: open the chat first, then call this. Throws if the highlight is wrong. */
  async function selectAllAccounts() {
    await (await b.$('button=All accounts')).click();
    await ui.pause(2500);
    await ui.dismissBanner();
    const fresh = await b.$('aria/New chat');
    if (await fresh.isExisting()) { await fresh.click(); await ui.pause(1200); }
    const ok = await b.execute(() => {
      const bt = [...document.querySelectorAll('button')].find((e) => e.innerText.trim() === 'All accounts');
      return !!bt && bt.parentElement.className.includes('bg-primary');
    });
    if (!ok) throw new Error('All accounts is not highlighted in the sidebar');
  }

  /** Type into an input/textarea letter by letter, one screenshot every `every`
   *  characters. Uses the native value setter + an input event: WebDriver's
   *  synthetic Space key never reaches this WKWebView, so b.keys() drops spaces. */
  async function typeFrames(selector, text, prefix, every = 2) {
    let n = 0;
    for (let i = 1; i <= text.length; i++) {
      await b.execute((sel, v) => {
        const el = document.querySelector(sel);
        el.focus();
        const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
        Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }, selector, text.slice(0, i));
      if (i % every === 0 || i === text.length) await ui.shot(`${prefix}-${String(++n).padStart(3, '0')}`);
    }
    return n;
  }

  const sendChat = () => b.execute(() =>
    document.querySelector('textarea').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));

  const chatBusy = () => b.execute(() =>
    /Waiting for reply/.test(document.querySelector('textarea')?.placeholder || '')
    || [...document.querySelectorAll('button')].some((e) => e.innerText.trim() === 'Cancel'));

  /** One frame every ~0.7 s while the chat answers (routing, streaming), then stop. */
  async function chatFrames(prefix, timeoutS = 240) {
    let k = 0;
    for (let i = 0; i < timeoutS / 0.7; i++) {
      await ui.pause(700);
      await ui.dismissBanner();
      await ui.shot(`${prefix}-${String(++k).padStart(3, '0')}`);
      if (!(await chatBusy()) && i > 2) break;
    }
    await ui.pause(1500);
    await ui.dismissBanner();
    return k;
  }

  /** Text of the last answer (to check it before building the story around it). */
  const lastAnswer = () => b.execute(() => [...document.querySelectorAll('div')]
    .filter((d) => /qwen|gemma/.test(d.innerText) && d.innerText.length < 4000)
    .sort((a, c) => a.innerText.length - c.innerText.length)[0]?.innerText);

  /** Scroll the chat so the last question sits at the top (long answers end scrolled down). */
  const chatToTop = () => b.execute(() => {
    const qs = [...document.querySelectorAll('div')].filter((d) => d.getBoundingClientRect().x > 1400
      && /bg-blue|bg-primary/.test((d.className || '').toString()) && d.innerText.length > 10);
    const q = qs[qs.length - 1];
    if (!q) return false;
    let el = q.parentElement;
    while (el && !(el.scrollHeight > el.clientHeight + 5 && /auto|scroll/.test(getComputedStyle(el).overflowY))) el = el.parentElement;
    if (!el) return false;
    el.scrollTop += q.getBoundingClientRect().y - el.getBoundingClientRect().y - 12;
    return true;
  });

  /** Wait for an AI draft: "Generating draft…" gone and a body in the editor. */
  async function draftFrames(prefix, timeoutS = 180) {
    let k = 0;
    for (let i = 0; i < timeoutS * 2; i++) {
      await ui.pause(500);
      await ui.shot(`${prefix}-${String(++k).padStart(3, '0')}`);
      const len = await b.execute(() => (document.querySelector('[contenteditable=true]')?.innerText || '').length);
      if (len > 40 && !(await ui.bodyHas('Generating draft'))) break;
    }
    await ui.pause(1000);
    return k;
  }

  /** Screenshots of the sidebar at several scroll offsets, for stitch_strip.py.
   *  The sidebar is a scroll container; low sections (SMART FILTERS: companies,
   *  intent, topic) are only reachable by scrolling it. Returns the offsets the
   *  container really took (it clamps at the bottom). */
  async function sidebarScrolls(prefix, tops = [700, 950, 1200, 1450]) {
    const meta = [];
    for (const top of tops) {
      const r = await b.execute((top) => {
        const sb = [...document.querySelectorAll('*')].find((e) => {
          const s = getComputedStyle(e);
          return /auto|scroll/.test(s.overflowY) && e.scrollHeight > e.clientHeight + 20
            && e.getBoundingClientRect().x < 50 && e.getBoundingClientRect().width < 300;
        });
        sb.scrollTop = top;
        const rr = sb.getBoundingClientRect();
        return { top: sb.scrollTop, y0: rr.y, y1: rr.bottom };
      }, top);
      await ui.pause(600);
      await ui.dismissBanner();
      await ui.shot(`${prefix}-${r.top}`);
      meta.push(r);
    }
    return meta;
  }

  /** Paste rich content (headings, lists) into a contenteditable such as an EO Doc.
   *  execCommand('insertParagraph') is lost half the time in this editor;
   *  insertHTML behaves like a paste and persists (reopen the doc to check). */
  const pasteHtml = (html) => b.execute((html) => {
    const ed = document.querySelector('[contenteditable=true]');
    ed.focus();
    document.execCommand('selectAll');
    document.execCommand('delete');
    document.execCommand('insertHTML', false, html);
  }, html);

  return { ...ui, closeReleaseNotes, selectAllAccounts, typeFrames, sendChat, chatFrames, lastAnswer, chatToTop,
           draftFrames, sidebarScrolls, pasteHtml };
}
