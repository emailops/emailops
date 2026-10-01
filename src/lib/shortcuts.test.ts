import { describe, expect, it } from 'vitest';
import {
  createShortcutMatcher,
  isSendShortcut,
  type KeyEventLike,
  type MatchContext,
  matchKey,
  SEQUENCE_TIMEOUT_MS,
  SHORTCUTS,
  type ShortcutId,
  shortcutKeyLabels,
} from './shortcuts';

const key = (k: string, mods: Partial<KeyEventLike> = {}): KeyEventLike => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

const ctx = (over: Partial<MatchContext> = {}): MatchContext => ({
  platform: 'macos',
  editable: false,
  modalOpen: false,
  scope: 'global',
  ...over,
});

/** Match one key with no pending sequence. */
const one = (event: KeyEventLike, c: MatchContext = ctx()) => matchKey(event, c, null).id;

describe('matchKey — single keys', () => {
  const table: [string, KeyEventLike, ShortcutId][] = [
    ['j next', key('j'), 'nav.next'],
    ['k previous', key('k'), 'nav.prev'],
    ['Enter opens', key('Enter'), 'nav.open'],
    ['o opens', key('o'), 'nav.open'],
    ['u back to list', key('u'), 'nav.back'],
    ['Escape', key('Escape'), 'nav.escape'],
    ['x selects', key('x'), 'select.toggle'],
    ['e archives', key('e'), 'thread.archive'],
    ['# deletes', key('#', { shiftKey: true }), 'thread.delete'],
    ['Delete deletes', key('Delete'), 'thread.delete'],
    ['s stars', key('s'), 'thread.star'],
    ['Shift+U marks unread', key('U', { shiftKey: true }), 'thread.markUnread'],
    ['Shift+I marks read', key('I', { shiftKey: true }), 'thread.markRead'],
    ['b snoozes', key('b'), 'thread.snooze'],
    ['c composes', key('c'), 'compose.new'],
    ['r replies', key('r'), 'compose.reply'],
    ['a replies all', key('a'), 'compose.replyAll'],
    ['f forwards', key('f'), 'compose.forward'],
    ['/ focuses search', key('/'), 'app.search'],
    ['? opens help', key('?', { shiftKey: true }), 'app.help'],
    ['⌘K opens search on macOS', key('k', { metaKey: true }), 'app.searchPalette'],
  ];
  it.each(table)('%s', (_name, event, id) => {
    expect(one(event)).toBe(id);
  });

  it('a plain letter does not fire its Shift variant, nor the reverse', () => {
    expect(one(key('u'))).toBe('nav.back');
    expect(one(key('U', { shiftKey: true }))).toBe('thread.markUnread');
    expect(one(key('J', { shiftKey: true }))).toBeNull();
  });

  it('caps lock (uppercase without Shift) still reads as the plain key', () => {
    expect(one(key('J'))).toBe('nav.next');
  });

  it('single keys ignore any held Cmd, Ctrl or Alt', () => {
    expect(one(key('e', { metaKey: true }))).toBeNull();
    expect(one(key('e', { ctrlKey: true }))).toBeNull();
    expect(one(key('e', { altKey: true }))).toBeNull();
  });

  it('unknown keys match nothing', () => {
    expect(one(key('q'))).toBeNull();
    expect(one(key('Shift', { shiftKey: true }))).toBeNull();
  });
});

describe('matchKey — platform modifier', () => {
  it('Mod is Cmd on macOS, not Ctrl', () => {
    expect(one(key('k', { metaKey: true }), ctx({ platform: 'macos' }))).toBe('app.searchPalette');
    expect(one(key('k', { ctrlKey: true }), ctx({ platform: 'macos' }))).toBeNull();
  });

  it.each(['windows', 'linux', ''])('Mod is Ctrl on %j, not the Windows/Super key', (platform) => {
    expect(one(key('k', { ctrlKey: true }), ctx({ platform }))).toBe('app.searchPalette');
    expect(one(key('k', { metaKey: true }), ctx({ platform }))).toBeNull();
  });

  it('a modifier shortcut still fires from inside an editable field', () => {
    expect(one(key('k', { metaKey: true }), ctx({ editable: true }))).toBe('app.searchPalette');
  });
});

describe('matchKey — where shortcuts must not fire', () => {
  it.each(['j', 'e', 'c', '/', '?', 'Enter', 'Escape', 'Delete'])('%s is ignored while typing', (k) => {
    expect(one(key(k), ctx({ editable: true }))).toBeNull();
  });

  it('nothing fires while a modal is open (modals handle their own keys)', () => {
    expect(one(key('j'), ctx({ modalOpen: true }))).toBeNull();
    expect(one(key('Escape'), ctx({ modalOpen: true }))).toBeNull();
    expect(one(key('k', { metaKey: true }), ctx({ modalOpen: true }))).toBeNull();
  });

  it('nothing fires while an IME is composing', () => {
    expect(one(key('j', { isComposing: true }))).toBeNull();
    expect(one(key('Process', { keyCode: 229 }))).toBeNull();
  });

  it('composer-only shortcuts never match globally', () => {
    expect(one(key('Enter', { metaKey: true }))).toBeNull();
  });
});

describe('matchKey — composer scope', () => {
  const composer = (over: Partial<MatchContext> = {}) => ctx({ scope: 'composer', editable: true, ...over });

  it('Cmd+Enter sends on macOS, Ctrl+Enter elsewhere', () => {
    expect(one(key('Enter', { metaKey: true }), composer())).toBe('compose.send');
    expect(one(key('Enter', { ctrlKey: true }), composer({ platform: 'windows' }))).toBe('compose.send');
    expect(one(key('Enter', { ctrlKey: true }), composer())).toBeNull();
  });

  it('works inside a modal composer and ignores the global bindings', () => {
    expect(one(key('Enter', { metaKey: true }), composer({ modalOpen: true }))).toBe('compose.send');
    expect(one(key('Enter'), composer({ editable: false }))).toBeNull();
    expect(one(key('j'), composer({ editable: false }))).toBeNull();
  });

  it('plain Enter in a composer is a newline, not a send', () => {
    expect(one(key('Enter'), composer())).toBeNull();
  });
});

describe('matchKey — two-key sequences', () => {
  it('g arms a sequence and fires nothing', () => {
    const r = matchKey(key('g'), ctx(), null);
    expect(r.id).toBeNull();
    expect(r.pending).toBe('g');
  });

  it.each([
    ['i', 'go.inbox'],
    ['s', 'go.starred'],
    ['t', 'go.sent'],
    ['d', 'go.drafts'],
    ['b', 'go.snoozed'],
    ['a', 'go.archive'],
    ['l', 'go.scheduled'],
    ['c', 'go.contacts'],
  ] as [string, ShortcutId][])('g %s', (second, id) => {
    const r = matchKey(key(second), ctx(), 'g');
    expect(r.id).toBe(id);
    expect(r.pending).toBeNull();
  });

  it('* a selects all, * n selects none', () => {
    expect(matchKey(key('*', { shiftKey: true }), ctx(), null).pending).toBe('*');
    expect(matchKey(key('a'), ctx(), '*').id).toBe('select.all');
    expect(matchKey(key('n'), ctx(), '*').id).toBe('select.none');
  });

  it('a second key that completes no sequence drops the prefix and is read on its own', () => {
    const r = matchKey(key('j'), ctx(), 'g');
    expect(r.id).toBe('nav.next');
    expect(r.pending).toBeNull();
  });

  it('typing into a field cancels a pending sequence', () => {
    const r = matchKey(key('i'), ctx({ editable: true }), 'g');
    expect(r.id).toBeNull();
    expect(r.pending).toBeNull();
  });
});

describe('createShortcutMatcher — sequence timeout', () => {
  it('completes a sequence within the timeout', () => {
    let now = 1000;
    const m = createShortcutMatcher(() => now);
    expect(m.match(key('g'), ctx())).toBeNull();
    now += SEQUENCE_TIMEOUT_MS - 1;
    expect(m.match(key('i'), ctx())).toBe('go.inbox');
  });

  it('forgets the prefix after the timeout', () => {
    let now = 1000;
    const m = createShortcutMatcher(() => now);
    m.match(key('g'), ctx());
    now += SEQUENCE_TIMEOUT_MS + 1;
    // `i` alone is nothing; `s` alone is star, not "go to starred".
    expect(m.match(key('i'), ctx())).toBeNull();
    m.match(key('g'), ctx());
    now += SEQUENCE_TIMEOUT_MS + 1;
    expect(m.match(key('s'), ctx())).toBe('thread.star');
  });

  it('a pure modifier key press does not cancel a pending sequence', () => {
    const now = 1000;
    const m = createShortcutMatcher(() => now);
    m.match(key('*'), ctx());
    expect(m.match(key('Shift', { shiftKey: true }), ctx())).toBeNull();
    expect(m.match(key('a'), ctx())).toBe('select.all');
  });

  it('reset() drops a pending prefix', () => {
    const m = createShortcutMatcher(() => 0);
    m.match(key('g'), ctx());
    m.reset();
    expect(m.match(key('i'), ctx())).toBeNull();
  });
});

describe('registry', () => {
  it('ids are unique', () => {
    const ids = SHORTCUTS.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('no binding is claimed by two shortcuts', () => {
    const seen = new Map<string, string>();
    for (const s of SHORTCUTS) {
      for (const k of s.keys) {
        const slot = `${s.scope}:${k}`;
        expect(seen.get(slot), `${k} bound twice`).toBeUndefined();
        seen.set(slot, s.id);
      }
    }
  });

  it('every label key lives in the shortcuts namespace', () => {
    for (const s of SHORTCUTS) expect(s.labelKey.startsWith('shortcuts:')).toBe(true);
  });
});

describe('shortcutKeyLabels', () => {
  const def = (id: ShortcutId) => SHORTCUTS.find((s) => s.id === id)!;

  it('formats the platform modifier like the rest of the app', () => {
    expect(shortcutKeyLabels(def('app.searchPalette'), 'macos')).toEqual([['⌘K']]);
    expect(shortcutKeyLabels(def('app.searchPalette'), 'windows')).toEqual([['Ctrl+K']]);
    expect(shortcutKeyLabels(def('compose.send'), 'macos')).toEqual([['⌘Enter']]);
  });

  it('splits sequences into their keys and lists alternatives', () => {
    expect(shortcutKeyLabels(def('go.inbox'), 'macos')).toEqual([['g', 'i']]);
    expect(shortcutKeyLabels(def('nav.open'), 'macos')).toEqual([['Enter'], ['o']]);
    expect(shortcutKeyLabels(def('thread.markUnread'), 'linux')).toEqual([['Shift+U']]);
  });
});

describe('isSendShortcut', () => {
  it('is Cmd+Enter on macOS and Ctrl+Enter elsewhere', () => {
    expect(isSendShortcut(key('Enter', { metaKey: true }), 'macos')).toBe(true);
    expect(isSendShortcut(key('Enter', { ctrlKey: true }), 'windows')).toBe(true);
    expect(isSendShortcut(key('Enter', { ctrlKey: true }), 'macos')).toBe(false);
    expect(isSendShortcut(key('Enter'), 'macos')).toBe(false);
    expect(isSendShortcut(key('Enter', { metaKey: true, isComposing: true }), 'macos')).toBe(false);
  });
});
