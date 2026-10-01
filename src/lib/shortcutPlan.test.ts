import { describe, expect, it } from 'vitest';
import { planShortcut, type ShortcutPlanContext } from './shortcutPlan';

const base = (over: Partial<ShortcutPlanContext> = {}): ShortcutPlanContext => ({
  listView: true,
  layout: 'split',
  listIds: ['e1', 'e2', 'e3'],
  selectionCount: 0,
  openConversation: false,
  openEmailId: null,
  cursorId: null,
  ...over,
});

const fullList = (over: Partial<ShortcutPlanContext> = {}) => base({ layout: 'full-width', ...over });
const split = (openEmailId: string, over: Partial<ShortcutPlanContext> = {}) =>
  base({ openConversation: true, openEmailId, ...over });

describe('planShortcut — j / k', () => {
  it('full-width list: moves the cursor, starting at the first row', () => {
    expect(planShortcut('nav.next', fullList())).toEqual({ type: 'cursor', id: 'e1' });
    expect(planShortcut('nav.next', fullList({ cursorId: 'e1' }))).toEqual({ type: 'cursor', id: 'e2' });
    expect(planShortcut('nav.prev', fullList({ cursorId: 'e2' }))).toEqual({ type: 'cursor', id: 'e1' });
  });

  it('stops at either end of the list', () => {
    expect(planShortcut('nav.next', fullList({ cursorId: 'e3' }))).toEqual({ type: 'none' });
    expect(planShortcut('nav.prev', fullList({ cursorId: 'e1' }))).toEqual({ type: 'none' });
  });

  it('a cursor on a row that left the list restarts at the top', () => {
    expect(planShortcut('nav.next', fullList({ cursorId: 'gone' }))).toEqual({ type: 'cursor', id: 'e1' });
  });

  it('split layout: opens the next / previous conversation in the pane', () => {
    expect(planShortcut('nav.next', split('e1'))).toEqual({ type: 'open', id: 'e2' });
    expect(planShortcut('nav.prev', split('e3'))).toEqual({ type: 'open', id: 'e2' });
    expect(planShortcut('nav.next', base())).toEqual({ type: 'open', id: 'e1' });
  });

  it('full-width reading a conversation: opens the neighbour, like Gmail', () => {
    expect(planShortcut('nav.next', fullList({ openConversation: true, openEmailId: 'e2' }))).toEqual({
      type: 'open',
      id: 'e3',
    });
  });

  it('a conversation opened from elsewhere (not in the list) has no neighbour', () => {
    expect(planShortcut('nav.next', split('elsewhere'))).toEqual({ type: 'none' });
  });

  it('does nothing outside a list view or on an empty list', () => {
    expect(planShortcut('nav.next', base({ listView: false }))).toEqual({ type: 'none' });
    expect(planShortcut('nav.next', base({ listIds: [] }))).toEqual({ type: 'none' });
  });
});

describe('planShortcut — open, back, escape', () => {
  it('Enter / o opens the cursor row in the full-width list', () => {
    expect(planShortcut('nav.open', fullList({ cursorId: 'e2' }))).toEqual({ type: 'open', id: 'e2' });
    expect(planShortcut('nav.open', fullList())).toEqual({ type: 'none' });
  });

  it('Enter does nothing when the cursor row is already open', () => {
    expect(planShortcut('nav.open', split('e2', { cursorId: 'e2' }))).toEqual({ type: 'none' });
  });

  it('u returns to the list in full-width only', () => {
    expect(planShortcut('nav.back', fullList({ openConversation: true, openEmailId: 'e1' }))).toEqual({
      type: 'closeConversation',
    });
    expect(planShortcut('nav.back', split('e1'))).toEqual({ type: 'none' });
    expect(planShortcut('nav.back', fullList())).toEqual({ type: 'none' });
  });

  it('Escape clears a selection first, then leaves a full-width conversation', () => {
    expect(planShortcut('nav.escape', base({ selectionCount: 2 }))).toEqual({ type: 'clearSelection' });
    expect(planShortcut('nav.escape', fullList({ openConversation: true, openEmailId: 'e1' }))).toEqual({
      type: 'closeConversation',
    });
    expect(planShortcut('nav.escape', split('e1'))).toEqual({ type: 'none' });
  });
});

describe('planShortcut — selection', () => {
  it('x toggles the cursor row (full-width) or the open row (split)', () => {
    expect(planShortcut('select.toggle', fullList({ cursorId: 'e3' }))).toEqual({ type: 'toggleSelect', id: 'e3' });
    expect(planShortcut('select.toggle', split('e2'))).toEqual({ type: 'toggleSelect', id: 'e2' });
    expect(planShortcut('select.toggle', fullList())).toEqual({ type: 'none' });
  });

  it('* a selects every row, * n clears', () => {
    expect(planShortcut('select.all', base())).toEqual({ type: 'selectAll' });
    expect(planShortcut('select.all', base({ listIds: [] }))).toEqual({ type: 'none' });
    expect(planShortcut('select.none', base({ selectionCount: 1 }))).toEqual({ type: 'clearSelection' });
  });
});

describe('planShortcut — conversation actions', () => {
  it.each(['archive', 'delete', 'star', 'markUnread', 'markRead', 'snooze'] as const)(
    '%s acts on the selection when there is one',
    (action) => {
      const id = `thread.${action}` as const;
      expect(planShortcut(id, split('e1', { selectionCount: 2 }))).toEqual({ type: 'bulk', action });
    },
  );

  it('without a selection, acts on the open conversation through the reading pane', () => {
    expect(planShortcut('thread.archive', split('e1'))).toEqual({ type: 'pane', command: 'archive' });
    expect(planShortcut('thread.star', fullList({ openConversation: true }))).toEqual({
      type: 'pane',
      command: 'star',
    });
  });

  it('in the full-width list, acts on the cursor row', () => {
    expect(planShortcut('thread.delete', fullList({ cursorId: 'e2' }))).toEqual({
      type: 'row',
      action: 'delete',
      id: 'e2',
    });
    expect(planShortcut('thread.delete', fullList())).toEqual({ type: 'none' });
  });

  it('a conversation shown outside a list view still takes pane actions', () => {
    expect(planShortcut('thread.archive', base({ listView: false, openConversation: true }))).toEqual({
      type: 'pane',
      command: 'archive',
    });
  });
});

describe('planShortcut — compose, go-to, app', () => {
  it('reply / reply all / forward need an open conversation', () => {
    expect(planShortcut('compose.reply', split('e1'))).toEqual({ type: 'pane', command: 'reply' });
    expect(planShortcut('compose.replyAll', split('e1'))).toEqual({ type: 'pane', command: 'replyAll' });
    expect(planShortcut('compose.forward', split('e1'))).toEqual({ type: 'pane', command: 'forward' });
    expect(planShortcut('compose.reply', fullList())).toEqual({ type: 'none' });
  });

  it('c, /, ⌘K and ? work from any view', () => {
    const elsewhere = base({ listView: false });
    expect(planShortcut('compose.new', elsewhere)).toEqual({ type: 'compose' });
    expect(planShortcut('app.search', elsewhere)).toEqual({ type: 'search' });
    expect(planShortcut('app.searchPalette', elsewhere)).toEqual({ type: 'searchPalette' });
    expect(planShortcut('app.help', elsewhere)).toEqual({ type: 'help' });
  });

  it.each([
    ['go.inbox', 'inbox'],
    ['go.starred', 'starred'],
    ['go.sent', 'sent'],
    ['go.drafts', 'drafts'],
    ['go.snoozed', 'snoozed'],
    ['go.archive', 'archive'],
    ['go.scheduled', 'scheduled'],
    ['go.contacts', 'contacts'],
  ] as const)('%s goes to %s', (id, view) => {
    expect(planShortcut(id, base({ listView: false }))).toEqual({ type: 'goTo', view });
  });

  it('the composer send key is not a global action', () => {
    expect(planShortcut('compose.send', split('e1'))).toEqual({ type: 'none' });
  });
});
