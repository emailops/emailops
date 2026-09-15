// The account name is the sender name on outgoing mail. The dialog edits it:
// prefilled when the account already has a real name, empty when the name is
// just the address (accounts added without a display name store it that way).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Account } from '@/types';
import { AccountSettingsDialog } from './AccountSettingsDialog';

const api = vi.hoisted(() => ({
  getAccountSettings: vi.fn(),
  setAccountSettings: vi.fn(),
  updateAccountName: vi.fn(),
  updateAccountSyncFrom: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

const account: Account = {
  id: 'acct-1',
  provider: 'gmail',
  email: 'ada@example.com',
  name: 'Ada Example',
  createdAt: 0,
  sortOrder: 0,
  enabled: true,
  syncFromTimestamp: null,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  api.getAccountSettings.mockReset().mockResolvedValue({ gmailCategories: ['primary'] });
  api.setAccountSettings.mockReset().mockResolvedValue(undefined);
  api.updateAccountName.mockReset().mockResolvedValue(account);
  api.updateAccountSyncFrom.mockReset().mockResolvedValue(account);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderDialog(acc: Account) {
  // Async act flushes the Gmail settings load, so the form is rendered.
  await act(async () => {
    root.render(
      <AccountSettingsDialog
        account={acc}
        onClose={() => {}}
        onSaved={() => {}}
        onToggleEnabled={async () => {}}
        onDelete={async () => {}}
      />,
    );
  });
  const input = container.querySelector<HTMLInputElement>('#account-sender-name');
  if (!input) throw new Error('sender name input not rendered');
  return input;
}

function typeInto(input: HTMLInputElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  act(() => {
    setValue?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function save() {
  const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Save & Sync');
  if (!button) throw new Error('save button not rendered');
  await act(async () => {
    button.click();
  });
}

describe('AccountSettingsDialog sender name', () => {
  it('prefills the sender name from the account name', async () => {
    expect((await renderDialog(account)).value).toBe('Ada Example');
  });

  it('leaves the sender name empty when the account name is its address', async () => {
    expect((await renderDialog({ ...account, name: 'ada@example.com' })).value).toBe('');
  });

  it('saves a changed sender name', async () => {
    typeInto(await renderDialog({ ...account, name: 'ada@example.com' }), 'Ada Example');

    await save();

    expect(api.updateAccountName).toHaveBeenCalledWith('acct-1', 'Ada Example');
  });

  it('does not rename the account when the sender name is unchanged', async () => {
    await renderDialog(account);

    await save();

    expect(api.updateAccountName).not.toHaveBeenCalled();
    expect(api.updateAccountSyncFrom).toHaveBeenCalled();
  });
});
