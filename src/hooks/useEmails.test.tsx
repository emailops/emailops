// Regression: deleting the last account left the inbox showing the
// previously active account's stale email list. useEmails only refetched
// when activeAccountId was truthy, so the transition to null (no accounts
// left) never cleared the emailStore.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_LIST_SORT } from '@/lib/listColumns';
import { useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import type { Email } from '@/types';
import { useEmails } from './useEmails';

vi.mock('@/lib/api', () => ({
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  currentPlatform: vi.fn(() => ''),
}));

let container: HTMLDivElement;
let root: Root;

// Stable reference, matching real callers (App.tsx memoizes this list) —
// an inline `[]` default would churn identity every render and starve out
// the effect's dependency comparison.
const NO_CATEGORIES: never[] = [];

function Harness() {
  const { emails } = useEmails(NO_CATEGORIES);
  return <div data-testid="count">{emails.length}</div>;
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useAccountStore.setState({ accounts: [], activeAccountId: null });
  useEmailStore.getState().reset();
});

describe('useEmails', () => {
  it('clears the stale email list when the last account is removed', async () => {
    useAccountStore.setState({
      accounts: [{ id: 'acc-1', provider: 'gmail', email: 'work@example.com', enabled: true } as never],
      activeAccountId: 'acc-1',
    });
    useEmailStore.setState({ emails: [{ id: 'e1' } as Email] });

    act(() => {
      root.render(<Harness />);
    });
    expect(container.querySelector('[data-testid="count"]')?.textContent).toBe('1');

    // Simulate accountStore.removeAccount deleting the last remaining account.
    act(() => {
      useAccountStore.setState({ accounts: [], activeAccountId: null });
    });

    expect(container.querySelector('[data-testid="count"]')?.textContent).toBe('0');
  });

  it('reloads the list when the Unread toggle changes', async () => {
    const { getEmails } = await import('@/lib/api');
    useAccountStore.setState({
      accounts: [{ id: 'acc-1', provider: 'gmail', email: 'work@example.com', enabled: true } as never],
      activeAccountId: 'acc-1',
    });
    await act(async () => {
      root.render(<Harness />);
    });
    vi.mocked(getEmails).mockClear();

    await act(async () => {
      useEmailStore.getState().setUnreadOnly(true);
    });

    expect(getEmails).toHaveBeenCalledWith('acc-1', 50, 0, 'inbox', true, DEFAULT_LIST_SORT);
    useEmailStore.getState().setUnreadOnly(false);
  });

  it('reloads the list in the order a column header picks', async () => {
    const { getEmails } = await import('@/lib/api');
    useAccountStore.setState({
      accounts: [{ id: 'acc-1', provider: 'gmail', email: 'work@example.com', enabled: true } as never],
      activeAccountId: 'acc-1',
    });
    await act(async () => {
      root.render(<Harness />);
    });
    vi.mocked(getEmails).mockClear();

    await act(async () => {
      useEmailStore.getState().setListSort({ key: 'sender', dir: 'asc' });
    });

    expect(getEmails).toHaveBeenCalledWith('acc-1', 50, 0, 'inbox', false, { key: 'sender', dir: 'asc' });
    useEmailStore.getState().setListSort(DEFAULT_LIST_SORT);
  });
});
