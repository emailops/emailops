import { describe, expect, it } from 'vitest';
import { isEmailListView, planViewChange } from './viewNavigation';

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
