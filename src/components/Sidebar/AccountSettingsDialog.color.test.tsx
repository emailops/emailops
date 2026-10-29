// The account colour marks the account in the sidebar, the unified inbox and
// the "All accounts" calendar. The dialog lets the user pick another one; it
// is saved with the rest of the settings.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import type { Account } from '@/types';
import { AccountSettingsDialog } from './AccountSettingsDialog';

const api = vi.hoisted(() => ({
  getAccountSettings: vi.fn(),
  setAccountSettings: vi.fn(),
  updateAccountName: vi.fn(),
  updateAccountSyncFrom: vi.fn(),
  setAccountColor: vi.fn(),
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
  color: 'blue',
};

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  api.getAccountSettings.mockReset().mockResolvedValue({ gmailCategories: ['primary'] });
  api.setAccountSettings.mockReset().mockResolvedValue(undefined);
  api.updateAccountName.mockReset().mockResolvedValue(account);
  api.updateAccountSyncFrom.mockReset().mockResolvedValue(account);
  api.setAccountColor.mockReset().mockResolvedValue(account);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderDialog() {
  await act(async () => {
    root.render(
      <AccountSettingsDialog
        account={account}
        onClose={() => {}}
        onSaved={() => {}}
        onToggleEnabled={async () => {}}
        onDelete={async () => {}}
      />,
    );
  });
}

function swatch(label: string): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>(`[role="radio"][aria-label="${label}"]`);
  if (!button) throw new Error(`swatch ${label} not rendered`);
  return button;
}

async function save() {
  const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Save & Sync');
  if (!button) throw new Error('save button not rendered');
  await act(async () => {
    button.click();
  });
}

describe('AccountSettingsDialog colour', () => {
  it("marks the account's current colour as selected", async () => {
    await renderDialog();
    expect(swatch('Blue').getAttribute('aria-checked')).toBe('true');
    expect(swatch('Rose').getAttribute('aria-checked')).toBe('false');
  });

  it('saves a newly picked colour', async () => {
    await renderDialog();
    act(() => swatch('Rose').click());

    await save();

    expect(api.setAccountColor).toHaveBeenCalledWith('acct-1', 'rose');
  });

  it('does not save the colour when it is unchanged', async () => {
    await renderDialog();

    await save();

    expect(api.setAccountColor).not.toHaveBeenCalled();
  });
});
