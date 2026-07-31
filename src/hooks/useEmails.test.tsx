// Regression: deleting the last account left the inbox showing the
// previously active account's stale email list. useEmails only refetched
// when activeAccountId was truthy, so the transition to null (no accounts
// left) never cleared the emailStore.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
});
