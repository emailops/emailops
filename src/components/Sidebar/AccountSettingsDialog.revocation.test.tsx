// Removing an account revokes EmailOps' access at Google automatically, but
// Microsoft offers no per-app revocation. Before an Outlook account is
// deleted, the confirmation tells the user where to remove that access.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Account } from '@/types';
import { AccountSettingsDialog } from './AccountSettingsDialog';

const api = vi.hoisted(() => ({
  getAccountSettings: vi.fn(),
  getImapSettings: vi.fn(),
  setAccountSettings: vi.fn(),
  updateAccountName: vi.fn(),
  updateAccountSyncFrom: vi.fn(),
  getFullSignature: vi.fn(async () => ({ text: '', image: null })),
  setSignature: vi.fn(),
  setSignatureImage: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

function accountFor(provider: Account['provider']): Account {
  return {
    id: 'acct-1',
    provider,
    email: 'ada@example.com',
    name: 'Ada Example',
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  api.getAccountSettings.mockReset().mockResolvedValue({ gmailCategories: ['primary'] });
  api.getImapSettings.mockReset().mockResolvedValue(null);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function openDeleteConfirmation(provider: Account['provider']) {
  await act(async () => {
    root.render(
      <AccountSettingsDialog
        account={accountFor(provider)}
        onClose={() => {}}
        onSaved={() => {}}
        onToggleEnabled={async () => {}}
        onDelete={async () => {}}
      />,
    );
  });
  // The test i18n setup does not load the `modal` namespace, so the label may
  // render as its key.
  const button = Array.from(container.querySelectorAll('button')).find((b) =>
    ['Delete account', 'modal:accountSettings.deleteAccount'].includes(b.textContent ?? ''),
  );
  if (!button) throw new Error('delete button not rendered');
  act(() => button.click());
}

describe('AccountSettingsDialog delete confirmation', () => {
  it('tells an Outlook user where to remove EmailOps at Microsoft', async () => {
    await openDeleteConfirmation('outlook');
    expect(container.textContent).toContain('https://account.live.com/consent/Manage');
    expect(container.textContent).toContain('https://myapps.microsoft.com');
  });

  it('adds no Microsoft notice for other providers', async () => {
    for (const provider of ['gmail', 'imap'] as const) {
      await openDeleteConfirmation(provider);
      expect(container.textContent).not.toContain('account.live.com');
    }
  });
});
