// The unified ("All accounts") scope chip: shows which single account a
// per-account view is bound to, with the explanatory hint as its tooltip and
// the same deterministic account color the inbox rows use. UnifiedScopeBar is
// the self-gating wrapper views mount unconditionally.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { initI18n } from '@/i18n';
import { accountColorClass } from '@/lib/colors';
import { ALL_ACCOUNTS_ID, useAccountStore } from '@/stores/accountStore';
import type { Account } from '@/types';
import { AccountScopeChip } from './AccountScopeChip';
import { UnifiedScopeBar } from './UnifiedScopeBar';

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

describe('AccountScopeChip', () => {
  it('renders the scoped account email with the hint as tooltip', () => {
    act(() => {
      root.render(
        <AccountScopeChip accountId="acc-1" email="work@example.com" hint="This view shows work@example.com only" />,
      );
    });
    expect(container.textContent).toContain('work@example.com');
    const chip = container.querySelector('[title]');
    expect(chip?.getAttribute('title')).toBe('This view shows work@example.com only');
  });

  it('uses the deterministic per-account color dot (same palette as inbox rows)', () => {
    act(() => {
      root.render(<AccountScopeChip accountId="acc-1" email="work@example.com" hint="hint" />);
    });
    const dot = container.querySelector('span[aria-hidden="true"]');
    expect(dot?.className).toContain(accountColorClass('acc-1'));
  });
});

describe('UnifiedScopeBar', () => {
  beforeAll(async () => {
    await initI18n('en');
  });

  const account = { id: 'acc-1', provider: 'gmail', email: 'work@example.com', enabled: true } as Account;

  it('renders the chip while All accounts is selected', () => {
    useAccountStore.setState({ accounts: [account], activeAccountId: ALL_ACCOUNTS_ID });
    act(() => {
      root.render(<UnifiedScopeBar accountId="acc-1" />);
    });
    expect(container.textContent).toContain('work@example.com');
  });

  it('renders nothing when a single account is selected', () => {
    useAccountStore.setState({ accounts: [account], activeAccountId: 'acc-1' });
    act(() => {
      root.render(<UnifiedScopeBar accountId="acc-1" />);
    });
    expect(container.textContent).toBe('');
  });

  it('renders nothing without a concrete account', () => {
    useAccountStore.setState({ accounts: [], activeAccountId: ALL_ACCOUNTS_ID });
    act(() => {
      root.render(<UnifiedScopeBar accountId={null} />);
    });
    expect(container.textContent).toBe('');
  });
});
