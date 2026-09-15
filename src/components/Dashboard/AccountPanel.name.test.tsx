// An account without a name stores an empty one; the panel header falls back
// to the address instead of rendering a blank title.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AccountDashboard } from '@/types';
import { AccountPanel } from './AccountPanel';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/lib/api', () => ({ refreshServerTotal: vi.fn() }));

function dashboard(name: string): AccountDashboard {
  return {
    account: {
      id: 'acct-1',
      provider: 'imap',
      email: 'ada@example.com',
      name,
      createdAt: 0,
      sortOrder: 0,
      enabled: true,
      syncFromTimestamp: null,
    },
    sync: { accountId: 'acct-1', status: 'idle', lastSyncAt: null, error: null },
    syncedSince: null,
    syncedCount: 0,
    serverTotal: null,
    serverTotalFetchedAt: null,
    categoryCounts: [],
    sentCount: 0,
    classifiedCount: 0,
    classifiedEligible: 0,
    memoryAnalyzedCount: 0,
    memoryEligible: 0,
    taskAnalyzedCount: 0,
    taskEligible: 0,
    embeddedCount: 0,
    embeddedEligible: 0,
    junkScoredCount: 0,
    junkPhishingCount: 0,
    junkSpamCount: 0,
    junkGraymailCount: 0,
  };
}

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function headerTitle(name: string): string | null | undefined {
  act(() => {
    root.render(<AccountPanel data={dashboard(name)} onRefreshed={() => {}} onOpenSettings={() => {}} />);
  });
  return container.querySelector('div.font-semibold.truncate')?.textContent;
}

describe('AccountPanel header', () => {
  it('shows the account name', () => {
    expect(headerTitle('Ada Example')).toBe('Ada Example');
  });

  it('falls back to the address when the account has no name', () => {
    expect(headerTitle('')).toBe('ada@example.com');
  });
});
