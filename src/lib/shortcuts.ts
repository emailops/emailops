// Keyboard shortcuts: one declarative table and a pure matcher.
//
// The table (`SHORTCUTS`) is the single source of truth — the global key
// handler (`useGlobalShortcuts`), the composers' send key and the `?` help
// modal all read it, so a binding cannot be documented in one place and
// implemented differently in another.
//
// Bindings are strings: `j`, `Shift+U`, `Mod+K` (Cmd on macOS, Ctrl
// elsewhere), and two-key sequences separated by a space (`g i`). Everything
// here is pure and takes the platform and the clock as inputs, so the rules
// are tested for every platform from any host.

import { formatShortcut } from './platform';

export type ShortcutId =
  | 'nav.next'
  | 'nav.prev'
  | 'nav.open'
  | 'nav.back'
  | 'nav.escape'
  | 'select.toggle'
  | 'select.all'
  | 'select.none'
  | 'thread.archive'
  | 'thread.delete'
  | 'thread.star'
  | 'thread.markUnread'
  | 'thread.markRead'
  | 'thread.snooze'
  | 'compose.new'
  | 'compose.reply'
  | 'compose.replyAll'
  | 'compose.forward'
  | 'compose.send'
  | 'go.inbox'
  | 'go.starred'
  | 'go.sent'
  | 'go.drafts'
  | 'go.snoozed'
  | 'go.archive'
  | 'go.scheduled'
  | 'go.contacts'
  | 'app.search'
  | 'app.searchPalette'
  | 'app.help';

/** `global`: the app-wide handler. `composer`: only inside a message editor. */
export type ShortcutScope = 'global' | 'composer';

/** Sections of the help modal, in display order. */
export const SHORTCUT_GROUPS = ['navigation', 'actions', 'compose', 'goto', 'application'] as const;
export type ShortcutGroup = (typeof SHORTCUT_GROUPS)[number];

export interface ShortcutDef {
  id: ShortcutId;
  /** Alternative bindings; any of them triggers the shortcut. */
  keys: readonly string[];
  scope: ShortcutScope;
  group: ShortcutGroup;
  /** i18n key of the help-modal label. */
  labelKey: `shortcuts:${string}`;
}

const def = (
  id: ShortcutId,
  keys: readonly string[],
  group: ShortcutGroup,
  labelKey: `shortcuts:${string}`,
  scope: ShortcutScope = 'global',
): ShortcutDef => ({ id, keys, scope, group, labelKey });

/** Gmail's bindings where Gmail has one. */
export const SHORTCUTS: readonly ShortcutDef[] = [
  def('nav.next', ['j'], 'navigation', 'shortcuts:items.next'),
  def('nav.prev', ['k'], 'navigation', 'shortcuts:items.prev'),
  def('nav.open', ['Enter', 'o'], 'navigation', 'shortcuts:items.open'),
  def('nav.back', ['u'], 'navigation', 'shortcuts:items.back'),
  def('nav.escape', ['Escape'], 'navigation', 'shortcuts:items.escape'),
  def('select.toggle', ['x'], 'navigation', 'shortcuts:items.selectToggle'),
  def('select.all', ['* a'], 'navigation', 'shortcuts:items.selectAll'),
  def('select.none', ['* n'], 'navigation', 'shortcuts:items.selectNone'),
  def('thread.archive', ['e'], 'actions', 'shortcuts:items.archive'),
  def('thread.delete', ['#', 'Delete'], 'actions', 'shortcuts:items.delete'),
  def('thread.star', ['s'], 'actions', 'shortcuts:items.star'),
  def('thread.markUnread', ['Shift+U'], 'actions', 'shortcuts:items.markUnread'),
  def('thread.markRead', ['Shift+I'], 'actions', 'shortcuts:items.markRead'),
  def('thread.snooze', ['b'], 'actions', 'shortcuts:items.snooze'),
  def('compose.new', ['c'], 'compose', 'shortcuts:items.compose'),
  def('compose.reply', ['r'], 'compose', 'shortcuts:items.reply'),
  def('compose.replyAll', ['a'], 'compose', 'shortcuts:items.replyAll'),
  def('compose.forward', ['f'], 'compose', 'shortcuts:items.forward'),
  def('compose.send', ['Mod+Enter'], 'compose', 'shortcuts:items.send', 'composer'),
  def('go.inbox', ['g i'], 'goto', 'shortcuts:items.goInbox'),
  def('go.starred', ['g s'], 'goto', 'shortcuts:items.goStarred'),
  def('go.sent', ['g t'], 'goto', 'shortcuts:items.goSent'),
  def('go.drafts', ['g d'], 'goto', 'shortcuts:items.goDrafts'),
  def('go.snoozed', ['g b'], 'goto', 'shortcuts:items.goSnoozed'),
  def('go.archive', ['g a'], 'goto', 'shortcuts:items.goArchive'),
  def('go.scheduled', ['g l'], 'goto', 'shortcuts:items.goScheduled'),
  def('go.contacts', ['g c'], 'goto', 'shortcuts:items.goContacts'),
  def('app.search', ['/'], 'application', 'shortcuts:items.search'),
  def('app.searchPalette', ['Mod+K'], 'application', 'shortcuts:items.searchPalette'),
  def('app.help', ['?'], 'application', 'shortcuts:items.help'),
];

/** How long the second key of a sequence (`g` then `i`) may take. */
export const SEQUENCE_TIMEOUT_MS = 1000;

/** The parts of a `KeyboardEvent` the matcher reads. */
export interface KeyEventLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
  keyCode?: number;
}

export interface MatchContext {
  /** Tauri platform code (`macos`, `windows`, `linux`); decides what `Mod` is. */
  platform: string;
  /** Focus is in a text field, select or rich-text editor. */
  editable: boolean;
  /** A modal dialog is open; it handles its own keys. */
  modalOpen: boolean;
  scope: ShortcutScope;
}

interface Binding {
  mod: boolean;
  shift: boolean;
  /** The key as written in the table (`j`, `U`, `Enter`, `#`). */
  key: string;
}

function parseBinding(raw: string): Binding {
  const parts = raw.split('+');
  const key = parts[parts.length - 1];
  return { mod: parts.includes('Mod'), shift: parts.includes('Shift'), key };
}

const isLetter = (k: string) => k.length === 1 && /[a-z]/i.test(k);
const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'OS']);

/** Whether one key press matches one (non-sequence) binding. */
function pressMatches(event: KeyEventLike, binding: Binding, platform: string): boolean {
  const isMac = platform === 'macos';
  const modDown = isMac ? event.metaKey : event.ctrlKey;
  const otherMod = isMac ? event.ctrlKey : event.metaKey;
  if (binding.mod !== modDown || otherMod || event.altKey) return false;
  if (isLetter(binding.key)) {
    // Letters compare case-insensitively and read Shift explicitly, so caps
    // lock does not turn `j` into nothing and `U` needs a real Shift.
    return event.key.toLowerCase() === binding.key.toLowerCase() && event.shiftKey === binding.shift;
  }
  // Symbols (`#`, `?`, `*`) need Shift on most layouts; what counts is the
  // character produced, so Shift is ignored for them.
  return event.key === binding.key;
}

interface ParsedDef {
  def: ShortcutDef;
  /** Each alternative as one or two key presses. */
  alternatives: Binding[][];
}

const PARSED: readonly ParsedDef[] = SHORTCUTS.map((d) => ({
  def: d,
  alternatives: d.keys.map((k) => k.split(' ').map(parseBinding)),
}));

export interface MatchResult {
  id: ShortcutId | null;
  /** The first key of a sequence waiting for its second, if any. */
  pending: string | null;
}

const NONE: MatchResult = { id: null, pending: null };

/**
 * Pure: the shortcut one key press triggers, given the first key of a
 * sequence that may be pending (`g`, `*`). Callers own the timeout — see
 * `createShortcutMatcher`.
 */
export function matchKey(event: KeyEventLike, context: MatchContext, pending: string | null): MatchResult {
  if (event.isComposing || event.keyCode === 229) return NONE;
  if (MODIFIER_KEYS.has(event.key)) return { id: null, pending };
  const inScope = PARSED.filter((p) => p.def.scope === context.scope);

  if (context.scope === 'composer') {
    for (const p of inScope) {
      for (const alt of p.alternatives) {
        if (alt.length === 1 && alt[0].mod && pressMatches(event, alt[0], context.platform)) {
          return { id: p.def.id, pending: null };
        }
      }
    }
    return NONE;
  }

  if (context.modalOpen) return NONE;

  // A modifier combination works from a text field too (⌘K from the
  // composer); single keys and sequences would be typing.
  if (context.editable) {
    for (const p of inScope) {
      for (const alt of p.alternatives) {
        if (alt.length === 1 && alt[0].mod && pressMatches(event, alt[0], context.platform)) {
          return { id: p.def.id, pending: null };
        }
      }
    }
    return NONE;
  }

  if (pending !== null) {
    for (const p of inScope) {
      for (const alt of p.alternatives) {
        if (alt.length === 2 && alt[0].key === pending && pressMatches(event, alt[1], context.platform)) {
          return { id: p.def.id, pending: null };
        }
      }
    }
    // Not a completion: drop the prefix and read this key on its own.
  }

  for (const p of inScope) {
    for (const alt of p.alternatives) {
      if (alt.length === 1 && pressMatches(event, alt[0], context.platform)) return { id: p.def.id, pending: null };
    }
  }
  for (const p of inScope) {
    for (const alt of p.alternatives) {
      if (alt.length === 2 && pressMatches(event, alt[0], context.platform)) return { id: null, pending: alt[0].key };
    }
  }
  return NONE;
}

export interface ShortcutMatcher {
  match: (event: KeyEventLike, context: MatchContext) => ShortcutId | null;
  reset: () => void;
}

/** A matcher that remembers a pending sequence prefix for `SEQUENCE_TIMEOUT_MS`
 *  of the injected clock. */
export function createShortcutMatcher(now: () => number = Date.now): ShortcutMatcher {
  let pending: string | null = null;
  let at = 0;
  return {
    match(event, context) {
      if (pending !== null && now() - at > SEQUENCE_TIMEOUT_MS) pending = null;
      const before = pending;
      const result = matchKey(event, context, pending);
      pending = result.pending;
      if (pending !== null && pending !== before) at = now();
      return result.id;
    },
    reset() {
      pending = null;
    },
  };
}

/** How a single key reads in the help modal. */
function keyLabel(binding: Binding, platform: string): string {
  const key = binding.key.length === 1 && binding.mod ? binding.key.toUpperCase() : binding.key;
  if (binding.mod) return formatShortcut(platform, key);
  if (binding.shift) return `Shift+${key}`;
  return key;
}

/**
 * The bindings of a shortcut as display labels: one array per alternative,
 * one label per key press (`[['g', 'i']]`, `[['Enter'], ['o']]`, `[['⌘K']]`).
 */
export function shortcutKeyLabels(shortcut: ShortcutDef, platform: string): string[][] {
  return shortcut.keys.map((k) =>
    k
      .split(' ')
      .map(parseBinding)
      .map((b) => keyLabel(b, platform)),
  );
}

/** The table entry for an id. */
export function shortcutById(id: ShortcutId): ShortcutDef | undefined {
  return SHORTCUTS.find((s) => s.id === id);
}

/**
 * A shortcut's first binding as a compact tooltip label: `E`, `#`, `Shift+U`,
 * `⌘K` / `Ctrl+K`, `G I`. Read from `SHORTCUTS`, so a tooltip cannot name a
 * key the handler does not bind.
 */
export function shortcutHintKeys(id: ShortcutId, platform: string): string | null {
  const shortcut = shortcutById(id);
  if (!shortcut) return null;
  const [first] = shortcutKeyLabels(shortcut, platform);
  if (!first) return null;
  return first.map((k) => (k.length === 1 ? k.toUpperCase() : k)).join(' ');
}

export interface ShortcutHintOptions {
  /** Keyboard shortcuts are on (Settings → Appearance). */
  enabled: boolean;
  platform: string;
  /** The locale's "label (keys)" pattern. */
  format: (label: string, keys: string) => string;
}

/** Pure: a button label with its shortcut, or the bare label when shortcuts
 *  are off (a hint for a key that does nothing would mislead). */
export function shortcutHint(label: string, id: ShortcutId, options: ShortcutHintOptions): string {
  if (!options.enabled) return label;
  const keys = shortcutHintKeys(id, options.platform);
  return keys ? options.format(label, keys) : label;
}

/** The composers' send key (Cmd+Enter on macOS, Ctrl+Enter elsewhere). */
export function isSendShortcut(event: KeyEventLike, platform: string): boolean {
  return matchKey(event, { platform, editable: true, modalOpen: false, scope: 'composer' }, null).id === 'compose.send';
}
