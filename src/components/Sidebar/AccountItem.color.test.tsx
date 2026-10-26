// Each sidebar account row carries the account's colour, the same one its
// mail rows and calendar events show in the "All accounts" views.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { initI18n } from '@/i18n';
import type { Account } from '@/types';
import { AccountItem } from './Sidebar';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const account: Account = {
  id: 'acct-1',
  provider: 'gmail',
  email: 'ada@example.com',
  name: '',
  createdAt: 0,
  sortOrder: 0,
  enabled: true,
  syncFromTimestamp: null,
  color: 'emerald',
};

describe('AccountItem', () => {
  it("shows a dot in the account's colour", () => {
    act(() => {
      root.render(
        <ul>
          <AccountItem
            account={account}
            isActive={false}
            isFirst
            isLast
            onSelect={() => {}}
            onMoveUp={() => {}}
            onMoveDown={() => {}}
            onToggleEnabled={() => {}}
            onOpenSettings={() => {}}
          />
        </ul>,
      );
    });
    const dot = container.querySelector('[data-testid="account-color-dot"]');
    expect(dot?.className).toContain('bg-emerald-500');
  });
});
