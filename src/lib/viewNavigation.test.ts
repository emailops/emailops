import { describe, expect, it } from 'vitest';
import { baseViewToken, isEmailListView, planAccountSwitchView, planViewChange } from './viewNavigation';

describe('isEmailListView', () => {
  it('recognises every mailbox-backed view', () => {
    expect(isEmailListView('inbox')).toBe(true);
    expect(isEmailListView('sent')).toBe(true);
    expect(isEmailListView('spam')).toBe(true);
    expect(isEmailListView('deleted')).toBe(true);
    expect(isEmailListView('folder:Projects/2026')).toBe(true);
  });

  it('rejects views that do not render the email list', () => {
    expect(isEmailListView('contacts')).toBe(false);
    expect(isEmailListView('chat')).toBe(false);
    expect(isEmailListView('dashboard')).toBe(false);
    expect(isEmailListView('drafts')).toBe(false);
  });
});

describe('planViewChange', () => {
  it('resets inbox filters when switching to any mailbox-backed view', () => {
    // Smart filters and search always query `mailbox IN ('inbox','sent')` and
    // ignore the selected mailbox, so carrying one into Sent/Spam/Trash/a
    // custom folder would leave the sidebar pointing at a view the list
    // never actually shows.
    expect(planViewChange('inbox', 'split').resetInboxFilters).toBe(true);
    expect(planViewChange('sent', 'split').resetInboxFilters).toBe(true);
    expect(planViewChange('spam', 'split').resetInboxFilters).toBe(true);
    expect(planViewChange('deleted', 'split').resetInboxFilters).toBe(true);
    expect(planViewChange('folder:Projects', 'split').resetInboxFilters).toBe(true);
  });

  it('leaves inbox filters alone for views that do not show the email list', () => {
    expect(planViewChange('contacts', 'split').resetInboxFilters).toBe(false);
    expect(planViewChange('chat', 'split').resetInboxFilters).toBe(false);
    expect(planViewChange('dashboard', 'split').resetInboxFilters).toBe(false);
  });

  it('closes the open email when switching views in full-width layout', () => {
    expect(planViewChange('sent', 'full-width').closeOpenEmail).toBe(true);
    expect(planViewChange('dashboard', 'full-width').closeOpenEmail).toBe(true);
    expect(planViewChange('inbox', 'full-width').closeOpenEmail).toBe(true);
  });

  it('keeps the open email when switching views in split layout', () => {
    expect(planViewChange('sent', 'split').closeOpenEmail).toBe(false);
    expect(planViewChange('inbox', 'split').closeOpenEmail).toBe(false);
  });
});

describe('planAccountSwitchView', () => {
  it('keeps the tag board when the account scope changes', () => {
    // The board is scope-aware end to end — both queries behind it take the
    // unified scope — so flipping between one account and "All accounts" is a
    // thing you do *to* the board, not a reason to leave it.
    expect(planAccountSwitchView('tagboard')).toBe('tagboard');
  });

  it('returns to the inbox from every per-account view', () => {
    // These views are hard-scoped to a single account and reset their own
    // state on a switch; landing back on the inbox is the established
    // behaviour and this change must not widen past the board.
    expect(planAccountSwitchView('contacts')).toBe('inbox');
    expect(planAccountSwitchView('tasks')).toBe('inbox');
    expect(planAccountSwitchView('memory')).toBe('inbox');
    expect(planAccountSwitchView('chat')).toBe('inbox');
    expect(planAccountSwitchView('drafts')).toBe('inbox');
    expect(planAccountSwitchView('dashboard')).toBe('inbox');
  });

  it('returns to the inbox from a mailbox-backed view', () => {
    // A folder belongs to one account, so its id is meaningless after a switch.
    expect(planAccountSwitchView('sent')).toBe('inbox');
    expect(planAccountSwitchView('folder:Projects/2026')).toBe('inbox');
    expect(planAccountSwitchView('inbox')).toBe('inbox');
  });
});

describe('baseViewToken', () => {
  it('names the open Lens so the chat can resolve "this lens"', () => {
    expect(baseViewToken(null, 'lenses', 'lens-1')).toBe('lens/lens-1');
  });

  it('falls back to the view without a Lens, and elsewhere ignores the Lens', () => {
    expect(baseViewToken(null, 'lenses', null)).toBe('view/lenses');
    expect(baseViewToken(null, 'inbox', 'lens-1')).toBe('view/inbox');
  });

  it('lets an open Settings tab win', () => {
    expect(baseViewToken('ai', 'lenses', 'lens-1')).toBe('settings/ai');
  });
});
