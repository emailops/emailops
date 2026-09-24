// In the unified ("All accounts") view the search dropdown lists hits from
// every account, so each hit carries a chip naming its account. A search
// scoped to one account needs no chip.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/lib/api', () => ({
  searchEmails: vi.fn(),
  autocompleteSenders: vi.fn(),
}));

import type { SearchResult } from '@/lib/api';
import * as api from '@/lib/api';
import { useAccountStore } from '@/stores/accountStore';
import type { Account, Email } from '@/types';
import { SearchBar } from './SearchBar';

function account(id: string, email: string): Account {
  return {
    id,
    provider: 'gmail',
    email,
    name: email,
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
  };
}

function hit(id: string, accountId: string) {
  return {
    id,
    accountId,
    threadId: `t-${id}`,
    messageId: null,
    subject: 'Invoice',
    sender: 'Billing',
    senderEmail: 'billing@example.com',
    recipients: [],
    cc: [],
    body: '',
    snippet: 'snippet',
    timestamp: 1_700_000_000,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
    relevanceScore: null,
    matchReason: null,
  } as Email & { relevanceScore: null; matchReason: null };
}

const RESULT = {
  emails: [hit('e1', 'acct-work'), hit('e2', 'acct-home')],
  query: 'invoice',
  parsedQuery: null,
  aiAvailable: false,
  searchMethod: 'keyword_search',
} as unknown as SearchResult;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(api.searchEmails).mockClear();
  vi.mocked(api.searchEmails).mockResolvedValue(RESULT);
  useAccountStore.setState({
    accounts: [account('acct-work', 'work@example.com'), account('acct-home', 'home@example.com')],
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function searchFor(accountId: string | null, query: string) {
  act(() => {
    root.render(
      <SearchBar
        accountId={accountId}
        onSelectEmail={() => {}}
        onApplySearch={() => {}}
        onApplySearchWithResults={() => {}}
        onClose={() => {}}
      />,
    );
  });
  const input = container.querySelector('input') as HTMLInputElement;
  act(() => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setValue?.call(input, query);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await act(async () => {
    await vi.runAllTimersAsync();
  });
}

function chips() {
  return Array.from(container.querySelectorAll('[data-testid="account-chip"]')).map((el) => el.textContent);
}

describe('SearchBar account chip', () => {
  it('names each hit’s account in the unified view', async () => {
    await searchFor(null, 'invoice');
    expect(chips()).toEqual(['work@example.com', 'home@example.com']);
  });

  it('searches every category, not only the selected inbox tab', async () => {
    await searchFor(null, 'invoice');
    expect(vi.mocked(api.searchEmails).mock.calls[0]).toEqual([null, 'invoice', true]);
  });

  it('shows no chip when the search is scoped to one account', async () => {
    await searchFor('acct-work', 'invoice');
    expect(container.textContent).toContain('Invoice');
    expect(chips()).toEqual([]);
  });
});
