// Regression: editing a draft in Gmail did not show up in EmailOps.
//
// The backend was fine — the sync's draft pull writes the upstream edit into
// SQLite and `pull_provider_drafts` updates the local row in place. But
// `DraftsView` fetched the list exactly once, in an effect keyed only on
// `accountId`. Every other surface (mail list, open thread, filters) refreshes
// when a sync reports `complete`; the drafts list did not. So the pulled edit
// sat in the database while the open view kept rendering the snapshot it took
// on mount, and only switching accounts or leaving and re-entering the view
// showed it.
//
// These tests pin that a completed sync for the shown account re-reads the
// list, and that a sync for a different account does not.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

interface TestDraft {
  id: string;
  accountId: string;
  subject: string;
  body: string;
  toAddresses: string[];
  ccAddresses: string[];
  updatedAt: number;
}

function draft(subject: string): TestDraft {
  return {
    id: 'd-1',
    accountId: 'acc-1',
    subject,
    body: 'body',
    toAddresses: ['dest@example.com'],
    ccAddresses: [],
    updatedAt: 1_700_000_000,
  };
}

const listDrafts = vi.fn((_accountId: string) => Promise.resolve([draft('Before')] as unknown[]));
const refreshDrafts = vi.fn((_accountId: string) => Promise.resolve(0));

vi.mock('@/lib/api', () => ({
  listDrafts: (accountId: string) => listDrafts(accountId),
  refreshDrafts: (accountId: string) => refreshDrafts(accountId),
  deleteDraft: vi.fn(() => Promise.resolve()),
}));

const { DraftsView } = await import('./DraftsView');

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  listDrafts.mockClear();
  refreshDrafts.mockClear();
  refreshDrafts.mockImplementation(() => Promise.resolve(0));
  listDrafts.mockImplementation(() => Promise.resolve([draft('Before')] as unknown[]));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

type SyncProgress = { accountId: string; status: string; current: number; total: number; message: string };

function complete(accountId: string): SyncProgress {
  return { accountId, status: 'complete', current: 0, total: 0, message: '' };
}

async function render(syncProgress: SyncProgress | null) {
  await act(async () => {
    root.render(
      <DraftsView
        accountId="acc-1"
        accounts={[{ id: 'acc-1', email: 'me@example.com' } as never]}
        syncProgress={syncProgress}
        onOpenComposeTab={() => {}}
      />,
    );
  });
}

describe('DraftsView', () => {
  it('shows the drafts it loaded on mount', async () => {
    await render(null);
    expect(listDrafts).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain('Before');
  });

  it('re-reads the list when a sync completes for the shown account', async () => {
    await render(null);
    listDrafts.mockImplementation(() => Promise.resolve([draft('Edited in Gmail')] as unknown[]));

    await render(complete('acc-1'));

    expect(listDrafts).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain('Edited in Gmail');
    expect(container.textContent).not.toContain('Before');
  });

  it('ignores a sync that completed for a different account', async () => {
    await render(null);
    await render(complete('acc-2'));
    expect(listDrafts).toHaveBeenCalledTimes(1);
  });

  it('does not re-read while a sync is still running', async () => {
    await render(null);
    await render({ ...complete('acc-1'), status: 'fetching' });
    expect(listDrafts).toHaveBeenCalledTimes(1);
  });

  // Opening the screen must not wait for the next account sync (up to minutes
  // away) to show a draft edited in the provider's own client.
  it('asks the provider for draft changes when the screen opens', async () => {
    refreshDrafts.mockImplementation(() => {
      listDrafts.mockImplementation(() => Promise.resolve([draft('Edited in Gmail')] as unknown[]));
      return Promise.resolve(1);
    });

    await render(null);

    expect(refreshDrafts).toHaveBeenCalledWith('acc-1');
    expect(container.textContent).toContain('Edited in Gmail');
  });

  it('keeps the local list when the provider refresh finds nothing new', async () => {
    await render(null);
    expect(refreshDrafts).toHaveBeenCalledTimes(1);
    // One read on mount; the refresh reported no changes so no second read.
    expect(listDrafts).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain('Before');
  });

  it('still shows the local drafts when the provider refresh fails', async () => {
    refreshDrafts.mockImplementation(() => Promise.reject(new Error('offline')));

    await render(null);

    expect(container.textContent).toContain('Before');
  });
});
