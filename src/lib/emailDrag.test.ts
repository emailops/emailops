import { describe, expect, it } from 'vitest';

import tauriConf from '../../src-tauri/tauri.conf.json';
import { EMAIL_DRAG_MIME, isEmailDrag, readEmailDragPayload, writeEmailDragPayload } from './emailDrag';

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
    writeEmailDragPayload(dt, { emailId: 'acc-1::10', accountId: 'acc-1', mailbox: 'inbox' });

    expect(isEmailDrag(dt)).toBe(true);
    expect(readEmailDragPayload(dt)).toEqual({
      emailId: 'acc-1::10',
      accountId: 'acc-1',
      mailbox: 'inbox',
    });
  });

  it('returns null for foreign drags', () => {
    const dt = fakeDataTransfer();
    dt.setData('text/plain', 'not an email');

    expect(isEmailDrag(dt)).toBe(false);
    expect(readEmailDragPayload(dt)).toBeNull();
  });

  it('returns null for malformed or incomplete payloads', () => {
    for (const raw of ['not json', '42', '{}', '{"emailId":"x"}', '{"emailId":"","accountId":"a","mailbox":"inbox"}']) {
      const dt = fakeDataTransfer();
      dt.setData(EMAIL_DRAG_MIME, raw);
      expect(readEmailDragPayload(dt)).toBeNull();
    }
  });
});
