// Opening a draft has to show what the provider has. The Drafts list renders
// rows read from SQLite, which only learn about an edit made in Gmail when a
// pull runs — so clicking a draft asks for one first, then re-reads the row.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Draft } from '@/types';

const refreshDrafts = vi.fn((_accountId: string) => Promise.resolve(0));
const getDraft = vi.fn((_draftId: string) => Promise.resolve(null as Draft | null));

vi.mock('@/lib/api', () => ({
  refreshDrafts: (accountId: string) => refreshDrafts(accountId),
  getDraft: (draftId: string) => getDraft(draftId),
}));

const { freshDraftToOpen } = await import('./draftOpen');

function draft(subject: string): Draft {
  return {
    id: 'd-1',
    accountId: 'acc-1',
    subject,
    body: 'body',
    toAddresses: [],
    ccAddresses: [],
    updatedAt: 1,
  } as unknown as Draft;
}

beforeEach(() => {
  refreshDrafts.mockClear();
  getDraft.mockClear();
  refreshDrafts.mockImplementation(() => Promise.resolve(0));
  getDraft.mockImplementation(() => Promise.resolve(null));
});

describe('freshDraftToOpen', () => {
  it('asks the provider for changes before reading the draft back', async () => {
    getDraft.mockImplementation(() => Promise.resolve(draft('Edited in Gmail')));

    const opened = await freshDraftToOpen(draft('Stale'));

    expect(refreshDrafts).toHaveBeenCalledWith('acc-1');
    expect(getDraft).toHaveBeenCalledWith('d-1');
    expect(opened.subject).toBe('Edited in Gmail');
  });

  it('falls back to the listed draft when the provider is unreachable', async () => {
    refreshDrafts.mockImplementation(() => Promise.reject(new Error('offline')));

    const opened = await freshDraftToOpen(draft('Stale'));

    expect(opened.subject).toBe('Stale');
    expect(getDraft).not.toHaveBeenCalled();
  });

  it('falls back to the listed draft when the row is gone after the pull', async () => {
    // e.g. the draft was sent or discarded elsewhere and the pull pruned it —
    // still better to open what the user clicked than nothing.
    const opened = await freshDraftToOpen(draft('Stale'));
    expect(opened.subject).toBe('Stale');
  });
});
