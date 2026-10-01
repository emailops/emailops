import { describe, expect, it } from 'vitest';
import { type DeleteShortcutContext, isDeleteShortcut, type KeyLike } from './deleteShortcut';

const OPEN: DeleteShortcutContext = { hasThread: true, isReplyOpen: false, isDeleting: false };

function key(k: string, overrides: Partial<KeyLike> = {}): KeyLike {
  return {
    key: k,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    target: document.body,
    ...overrides,
  };
}

function inside(html: string, selector: string): Element {
  const host = document.createElement('div');
  host.innerHTML = html;
  document.body.appendChild(host);
  const el = host.querySelector(selector);
  if (!el) throw new Error(`no ${selector}`);
  return el;
}

describe('isDeleteShortcut', () => {
  it('fires on Delete and Backspace with an open thread', () => {
    expect(isDeleteShortcut(key('Delete'), OPEN)).toBe(true);
    expect(isDeleteShortcut(key('Backspace'), OPEN)).toBe(true);
  });

  it('ignores other keys and modifier combinations', () => {
    expect(isDeleteShortcut(key('d'), OPEN)).toBe(false);
    for (const mod of ['metaKey', 'ctrlKey', 'altKey', 'shiftKey'] as const) {
      expect(isDeleteShortcut(key('Delete', { [mod]: true }), OPEN)).toBe(false);
    }
  });

  it('does nothing without a thread, with a reply open, or while deleting', () => {
    expect(isDeleteShortcut(key('Delete'), { ...OPEN, hasThread: false })).toBe(false);
    expect(isDeleteShortcut(key('Delete'), { ...OPEN, isReplyOpen: true })).toBe(false);
    expect(isDeleteShortcut(key('Delete'), { ...OPEN, isDeleting: true })).toBe(false);
  });

  it('ignores a held-down key and an event already handled', () => {
    expect(isDeleteShortcut(key('Delete', { repeat: true }), OPEN)).toBe(false);
    expect(isDeleteShortcut(key('Delete', { defaultPrevented: true }), OPEN)).toBe(false);
  });

  it('never fires while typing', () => {
    const targets = [
      inside('<input type="text">', 'input'),
      inside('<textarea></textarea>', 'textarea'),
      inside('<div contenteditable="true"><p>hi</p></div>', 'p'),
      inside('<div role="textbox"><span>x</span></div>', 'span'),
      inside('<select><option>a</option></select>', 'select'),
    ];
    for (const target of targets) {
      expect(isDeleteShortcut(key('Backspace', { target }), OPEN)).toBe(false);
      expect(isDeleteShortcut(key('Delete', { target }), OPEN)).toBe(false);
    }
  });

  it('never fires inside an open dialog or menu', () => {
    expect(
      isDeleteShortcut(
        key('Delete', { target: inside('<div role="dialog"><button>x</button></div>', 'button') }),
        OPEN,
      ),
    ).toBe(false);
    expect(
      isDeleteShortcut(key('Delete', { target: inside('<div role="menu"><button>x</button></div>', 'button') }), OPEN),
    ).toBe(false);
  });

  it('fires from a focused email row or button outside fields', () => {
    const row = inside('<div role="button" tabindex="0">Email</div>', 'div');
    expect(isDeleteShortcut(key('Delete', { target: row }), OPEN)).toBe(true);
  });
});
