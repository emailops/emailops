import { afterEach, describe, expect, it, vi } from 'vitest';

import tauriConf from '../../src-tauri/tauri.conf.json';
import {
  allowEmailDragOver,
  buildDragPreview,
  EMAIL_DRAG_MIME,
  emailIdsToDrag,
  ignoreEmailDrop,
  isEmailDrag,
  planEmailDrop,
  readEmailDragPayload,
  setEmailDragImage,
  writeEmailDragPayload,
} from './emailDrag';

describe('tauri window config', () => {
  // Tauri's native drag-drop handler (dragDropEnabled, default true) swallows
  // HTML5 dragstart/dragover/drop inside the WKWebView, which silently breaks
  // both the email→folder drag-and-drop and the rich-text editor's image
  // drop. Nothing listens to the native tauri://drag-drop events, so the
  // handler must stay disabled. tauri.intel.conf.json does not override
  // `app.windows`, so it inherits this setting from the base config.
  it('disables the native drag-drop interceptor on every window', () => {
    const windows = tauriConf.app.windows;
    expect(windows.length).toBeGreaterThan(0);
    for (const win of windows) {
      expect(win, `window "${win.title}" must set dragDropEnabled: false`).toHaveProperty('dragDropEnabled', false);
    }
  });
});

/** Minimal DataTransfer stand-in (jsdom lacks a constructor). */
function fakeDataTransfer(): DataTransfer {
  const store = new Map<string, string>();
  return {
    setData: (type: string, value: string) => store.set(type, value),
    getData: (type: string) => store.get(type) ?? '',
    get types() {
      return Array.from(store.keys());
    },
    effectAllowed: 'none',
  } as unknown as DataTransfer;
}

describe('email drag payload', () => {
  it('round-trips through write and read', () => {
    const dt = fakeDataTransfer();
    writeEmailDragPayload(dt, { emailIds: ['acc-1::10'], accountId: 'acc-1', mailbox: 'inbox' });

    expect(isEmailDrag(dt)).toBe(true);
    expect(readEmailDragPayload(dt)).toEqual({
      emailIds: ['acc-1::10'],
      accountId: 'acc-1',
      mailbox: 'inbox',
    });
  });

  it('carries several emails, without duplicates', () => {
    const dt = fakeDataTransfer();
    writeEmailDragPayload(dt, { emailIds: ['a', 'b', 'a'], accountId: 'acc-1', mailbox: 'inbox' });
    expect(readEmailDragPayload(dt)?.emailIds).toEqual(['a', 'b']);
  });

  it('returns null for foreign drags', () => {
    const dt = fakeDataTransfer();
    dt.setData('text/plain', 'not an email');

    expect(isEmailDrag(dt)).toBe(false);
    expect(readEmailDragPayload(dt)).toBeNull();
  });

  it('returns null for malformed or incomplete payloads', () => {
    for (const raw of [
      'not json',
      '42',
      '{}',
      '{"emailIds":["x"]}',
      '{"emailIds":[],"accountId":"a","mailbox":"inbox"}',
      '{"emailIds":[""],"accountId":"a","mailbox":"inbox"}',
      '{"emailIds":[42],"accountId":"a","mailbox":"inbox"}',
      '{"emailIds":"x","accountId":"a","mailbox":"inbox"}',
      // The old single-email shape is no longer accepted.
      '{"emailId":"x","accountId":"a","mailbox":"inbox"}',
    ]) {
      const dt = fakeDataTransfer();
      dt.setData(EMAIL_DRAG_MIME, raw);
      expect(readEmailDragPayload(dt), raw).toBeNull();
    }
  });
});

describe('dropping emails on a folder', () => {
  const payload = (emailIds: string[]) => ({ emailIds, accountId: 'acc-1', mailbox: 'inbox' });

  it('moves a single dragged email with the single move', () => {
    expect(planEmailDrop(payload(['e1']), 'acc-1', 'folder:Work')).toEqual({ kind: 'single', emailId: 'e1' });
  });

  it('moves several dragged emails together with the bulk move', () => {
    expect(planEmailDrop(payload(['e1', 'e2', 'e3']), 'acc-1', 'folder:Work')).toEqual({
      kind: 'bulk',
      emailIds: ['e1', 'e2', 'e3'],
    });
  });

  it('ignores foreign drags, another account, and the emails’ own mailbox', () => {
    expect(planEmailDrop(null, 'acc-1', 'folder:Work')).toEqual({ kind: 'ignore' });
    expect(planEmailDrop(payload(['e1', 'e2']), 'acc-2', 'folder:Work')).toEqual({ kind: 'ignore' });
    expect(planEmailDrop(payload(['e1', 'e2']), 'acc-1', 'inbox')).toEqual({ kind: 'ignore' });
  });
});

describe('emails carried by a drag', () => {
  it('is the dragged email alone when nothing is checked', () => {
    expect(emailIdsToDrag('e1', new Set())).toEqual(['e1']);
  });

  it('is the dragged email alone when it is not part of the selection', () => {
    expect(emailIdsToDrag('e1', new Set(['e2', 'e3']))).toEqual(['e1']);
  });

  it('is the whole selection, dragged email first, when the dragged email is checked', () => {
    expect(emailIdsToDrag('e3', new Set(['e1', 'e2', 'e3']))).toEqual(['e3', 'e1', 'e2']);
  });
});

describe('email drag preview', () => {
  afterEach(() => {
    document.body.innerHTML = '';
    vi.useRealTimers();
  });

  it('shows the sender and subject on a card attached to the page', () => {
    const card = buildDragPreview(document, { sender: 'Alice Martin', subject: 'Invoice 42' });
    expect(document.body.contains(card)).toBe(true);
    expect(card.textContent).toContain('Alice Martin');
    expect(card.textContent).toContain('Invoice 42');
    expect(card.getAttribute('aria-hidden')).toBe('true');
    expect(card.style.pointerEvents).toBe('none');
  });

  it('never interprets sender or subject as markup', () => {
    const card = buildDragPreview(document, { sender: '<img src=x onerror=alert(1)>', subject: '<b>hi</b>' });
    expect(card.querySelector('img, b')).toBeNull();
    expect(card.textContent).toContain('<img src=x onerror=alert(1)>');
  });

  it('shortens long subjects and shows a count badge for several emails', () => {
    const card = buildDragPreview(document, { sender: 'A', subject: 'x'.repeat(200), count: 3 });
    expect(card.textContent?.length).toBeLessThan(60);
    expect(card.querySelector('[data-role="count"]')?.textContent).toBe('3');
    expect(
      buildDragPreview(document, { sender: 'A', subject: 's', count: 1 }).querySelector('[data-role="count"]'),
    ).toBeNull();
  });

  it('keeps the sender whole and lets a long subject give way', () => {
    const card = buildDragPreview(document, { sender: 'PayPal', subject: 'Notification de paiement reçu' });
    const [sender, subject] = Array.from(card.children) as HTMLElement[];
    expect(sender.textContent).toBe('PayPal');
    expect(sender.style.flexShrink).toBe('0');
    expect(subject.style.flexShrink).toBe('1');
  });

  it('stays compact: one line, small text, at most 200px wide', () => {
    const card = buildDragPreview(document, { sender: 'Alice Martin', subject: 'Invoice 42', count: 3 });
    expect(card.style.maxWidth).toBe('200px');
    expect(card.style.whiteSpace).toBe('nowrap');
    expect(card.style.font).toContain('10px');
    // Count badge first, then sender, then subject — all on the same line.
    expect(Array.from(card.children).map((c) => c.textContent)).toEqual(['3', 'Alice Martin', 'Invoice 42']);
  });

  it('keeps a single preview on the page', () => {
    buildDragPreview(document, { sender: 'A', subject: '1' });
    buildDragPreview(document, { sender: 'B', subject: '2' });
    expect(document.querySelectorAll('#emailops-drag-preview')).toHaveLength(1);
  });

  it('hands the card to setDragImage and removes it right after', () => {
    vi.useFakeTimers();
    const setDragImage = vi.fn();
    setEmailDragImage({ dataTransfer: { setDragImage } as unknown as DataTransfer }, { sender: 'A', subject: 'S' });
    expect(setDragImage).toHaveBeenCalledTimes(1);
    const [card, x, y] = setDragImage.mock.calls[0];
    expect((card as HTMLElement).textContent).toContain('S');
    expect([x, y]).toEqual([8, 8]);
    vi.runAllTimers();
    expect(document.getElementById('emailops-drag-preview')).toBeNull();
  });

  it('does nothing where setDragImage is unavailable', () => {
    expect(() => setEmailDragImage({ dataTransfer: {} as DataTransfer }, { sender: 'A', subject: 'S' })).not.toThrow();
    expect(() => setEmailDragImage({ dataTransfer: null }, { sender: 'A', subject: 'S' })).not.toThrow();
    expect(document.getElementById('emailops-drag-preview')).toBeNull();
  });
});

describe('email list drag zone', () => {
  function dragEvent(types: string[]) {
    const dataTransfer = { types, dropEffect: 'none' } as unknown as DataTransfer;
    return { dataTransfer, preventDefault: vi.fn() };
  }

  it('accepts an email drag over the list so the "no drop" cursor is not shown', () => {
    const e = dragEvent([EMAIL_DRAG_MIME]);
    allowEmailDragOver(e);
    expect(e.preventDefault).toHaveBeenCalled();
    // 'move', not 'none': on WebKitGTK 'none' brings the "no drop" cursor back.
    expect(e.dataTransfer.dropEffect).toBe('move');
  });

  it('leaves foreign drags (files, text) alone', () => {
    const e = dragEvent(['Files']);
    allowEmailDragOver(e);
    ignoreEmailDrop(e);
    expect(e.preventDefault).not.toHaveBeenCalled();
    expect(e.dataTransfer.dropEffect).toBe('none');
  });

  it('swallows an email dropped back on the list', () => {
    const e = dragEvent([EMAIL_DRAG_MIME]);
    ignoreEmailDrop(e);
    expect(e.preventDefault).toHaveBeenCalled();
  });
});
