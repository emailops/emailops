/**
 * Primitives for driving the EmailOps demo instance while recording.
 *
 *   import { openApp } from './emailops-ui.mjs';
 *   const ui = await openApp({ dir: framesDir });
 *   await ui.goInbox();
 *   const row = await ui.visibleRow('Hetzner');   // measured, not guessed
 *   await ui.shot('a01-before-click', row);
 *   await ui.clickRow();
 *   await ui.finish();                            // writes rects.json + frames.json
 *
 * The rule the video depends on: anything the viewer must see clicked is
 * measured here, at capture time, and saved to rects.json. The composer draws
 * the pointer from those coordinates, so a click can never land off-screen.
 */
import { writeFileSync } from 'node:fs';
import { remote } from 'webdriverio';

export const WINDOW = { width: 1800, height: 1150 };

export async function openApp({ dir, port = 4445, window = WINDOW } = {}) {
  const b = await remote({ hostname: '127.0.0.1', port, path: '/', capabilities: {}, logLevel: 'error' });
  const rects = {};
  const frames = [];
  const pause = (ms) => b.pause(ms);

  await b.setWindowSize(window.width, window.height);
  await pause(1000);
  // One scale for the whole video: mixing window sizes makes the picture jump
  // between shots of the same screen.
  await b.execute(() => { document.documentElement.style.zoom = '1.0'; });
  await pause(600);

  async function shot(name, note = '') {
    await b.saveScreenshot(`${dir}/${name}.png`);
    frames.push({ name, note: note && JSON.stringify(note).slice(0, 160) });
    console.log('frame', name, note ? JSON.stringify(note).slice(0, 100) : '');
  }

  const clickText = (label) => b.execute((label) => {
    const bt = [...document.querySelectorAll('button')].find((e) => (e.innerText || '').trim() === label);
    if (bt) bt.click();
    return !!bt;
  }, label);

  const bodyHas = (text) => b.execute((t) => document.body.innerText.includes(t), text);

  /** The demo accounts have no credentials, so a sync attempt raises a banner
   *  that would sit in the header of every shot. Dismiss it as a user would. */
  async function dismissBanner() {
    const r = await b.execute(() => {
      const banner = [...document.querySelectorAll('div,section')].find((e) =>
        /sign in again|Authentication required/i.test(e.innerText || '') &&
        e.getBoundingClientRect().height < 200 && e.getBoundingClientRect().width > 400);
      const close = banner && [...banner.querySelectorAll('button')].pop();
      if (close) { close.click(); return 'dismissed'; }
      return 'none';
    });
    await pause(700);
    return r;
  }

  /** Centre of the smallest control whose visible text matches, recorded under
   *  `key`. Match on what the button actually says: "Show translation" does not
   *  contain "translate". */
  async function rectOf(key, pattern, maxWidth = 320) {
    const r = await b.execute((pattern, maxWidth) => {
      const re = new RegExp(pattern, 'i');
      const el = [...document.querySelectorAll('button,a,[role=button],label')]
        .filter((e) => {
          const box = e.getBoundingClientRect();
          return re.test((e.innerText || '').trim()) && box.width > 0 && box.width < maxWidth && box.height > 0;
        })
        .sort((a, c) => {
          const ra = a.getBoundingClientRect(), rc = c.getBoundingClientRect();
          return ra.width * ra.height - rc.width * rc.height;
        })[0];
      if (!el) return null;
      const box = el.getBoundingClientRect();
      el.setAttribute('data-target', '1');
      return { label: (el.innerText || '').trim().slice(0, 30),
               x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) };
    }, pattern, maxWidth);
    rects[key] = r;
    return r;
  }

  /** An email row that is fully on screen. A row scrolled out of view still
   *  answers to .click(), but its coordinates fall outside the frame. */
  async function visibleRow(needle, key = 'row') {
    const r = await b.execute((needle) => {
      const rows = [...document.querySelectorAll('button,[role=button]')].filter((e) => {
        const box = e.getBoundingClientRect();
        return (e.innerText || '').includes(needle) && box.height > 40 && box.height < 170
               && box.width > 500 && box.top > 80 && box.bottom < window.innerHeight - 80;
      });
      const row = rows[Math.floor(rows.length / 2)] || rows[0];
      if (!row) return null;
      row.setAttribute('data-target', '1');
      const box = row.getBoundingClientRect();
      return { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2),
               text: (row.innerText || '').replace(/\s+/g, ' ').slice(0, 56) };
    }, needle);
    rects[key] = r;
    return r;
  }

  /** Click whatever rectOf/visibleRow last measured, so the marker and the
   *  action are the same element. */
  const clickTarget = () => b.execute(() => {
    const el = document.querySelector('[data-target="1"]');
    if (!el) return false;
    (el.closest('button,a,[role=button]') || el).click();
    el.removeAttribute('data-target');
    return true;
  });

  const setScroll = (top) => b.execute((top) => {
    const list = [...document.querySelectorAll('div')]
      .find((e) => (e.className || '').toString().trim() === 'flex-1 overflow-y-auto');
    if (list) list.scrollTop = top;
    return list ? list.scrollTop : -1;
  }, top);

  async function goInbox() {
    const back = await b.$('aria/Back');
    if (await back.isExisting()) { await back.click(); await pause(700); }
    await clickText('Inbox');
    await pause(2000);
    await dismissBanner();
    // Virtualised rows arrive late; shooting too early gives an empty list.
    for (let i = 0; i < 40; i++) {
      const n = await b.execute(() => {
        const list = [...document.querySelectorAll('div')]
          .find((e) => (e.className || '').toString().trim() === 'flex-1 overflow-y-auto');
        return list ? list.querySelectorAll('[role=button],button').length : 0;
      });
      if (n > 10) return n;
      await pause(1000);
    }
    return 0;
  }

  async function openChat() {
    const open = await b.$('aria/Open chat panel');
    if (await open.isExisting()) { await open.click(); await pause(1500); }
    const fresh = await b.$('aria/New chat');
    if (await fresh.isExisting()) { await fresh.click(); await pause(1400); }
  }

  const typeQuestion = (text) => b.execute((text) => {
    const el = document.querySelector('textarea');
    el.focus();
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(el, text);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }, text);

  /** Wait for the local model to stop producing. `done` is text that appears
   *  when it has finished, e.g. 'Discard' for a draft. */
  async function waitForModel(done, timeoutMs = 180000) {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      if (await bodyHas(done)) return true;
      await pause(1000);
    }
    return false;
  }

  async function finish() {
    writeFileSync(`${dir}/rects.json`, JSON.stringify(rects, null, 1));
    writeFileSync(`${dir}/frames.json`, JSON.stringify(frames, null, 1));
    await b.deleteSession();
  }

  return { browser: b, rects, pause, shot, clickText, bodyHas, dismissBanner, rectOf,
           visibleRow, clickTarget, setScroll, goInbox, openChat, typeQuestion,
           waitForModel, finish };
}
