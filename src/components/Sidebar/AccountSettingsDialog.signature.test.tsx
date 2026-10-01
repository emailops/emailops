// The account dialog edits the account's email signature next to its sender
// name. It is saved only when it changed, and never when it could not be
// loaded (an empty field there would otherwise erase the stored signature).

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
  getFullSignature: vi.fn(),
  setSignature: vi.fn(),
  setSignatureImage: vi.fn(),
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
  api.getFullSignature.mockReset().mockResolvedValue({ text: 'Ada\nExample Inc.', image: null });
  api.setSignature.mockReset().mockResolvedValue(undefined);
  api.setSignatureImage.mockReset().mockResolvedValue(undefined);
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
  const textarea = container.querySelector<HTMLTextAreaElement>('#account-signature');
  if (!textarea) throw new Error('signature field not rendered');
  return textarea;
}

function typeInto(textarea: HTMLTextAreaElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  act(() => {
    setValue?.call(textarea, value);
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function save() {
  const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Save & Sync');
  if (!button) throw new Error('save button not rendered');
  await act(async () => {
    button.click();
  });
}

describe('AccountSettingsDialog signature', () => {
  it('prefills the stored signature', async () => {
    const field = await renderDialog();
    expect(api.getFullSignature).toHaveBeenCalledWith('acct-1');
    expect(field.value).toBe('Ada\nExample Inc.');
    expect(field.disabled).toBe(false);
  });

  it('saves a changed signature', async () => {
    typeInto(await renderDialog(), 'Ada\nNew Co.');
    await save();
    expect(api.setSignature).toHaveBeenCalledWith('acct-1', 'Ada\nNew Co.');
  });

  it('saves an emptied signature (removing it)', async () => {
    typeInto(await renderDialog(), '');
    await save();
    expect(api.setSignature).toHaveBeenCalledWith('acct-1', '');
  });

  it('does not write the signature when it is unchanged', async () => {
    await renderDialog();
    await save();
    expect(api.setSignature).not.toHaveBeenCalled();
    expect(api.updateAccountSyncFrom).toHaveBeenCalled();
  });

  it('never overwrites a signature it could not load', async () => {
    api.getFullSignature.mockRejectedValueOnce(new Error('db locked'));
    const field = await renderDialog();
    expect(field.disabled).toBe(true);
    await save();
    expect(api.setSignature).not.toHaveBeenCalled();
    expect(api.setSignatureImage).not.toHaveBeenCalled();
  });
});

const PNG = 'data:image/png;base64,iVBORw0KGgo=';

function setRange(input: HTMLInputElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  act(() => {
    setValue?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('AccountSettingsDialog signature image', () => {
  it('shows the stored image with its width and saves a new width', async () => {
    api.getFullSignature.mockResolvedValue({ text: 'Ada', image: { src: PNG, width: 160 } });
    await renderDialog();
    const range = container.querySelector<HTMLInputElement>('#account-signature-image-width');
    if (!range) throw new Error('width slider not rendered');
    expect(range.value).toBe('160');
    expect(container.querySelector(`img[src="${PNG}"]`)?.getAttribute('width')).toBe('160');

    setRange(range, '240');
    await save();
    expect(api.setSignatureImage).toHaveBeenCalledWith('acct-1', { src: PNG, width: 240 });
    expect(api.setSignature).not.toHaveBeenCalled();
  });

  it('removes the image', async () => {
    api.getFullSignature.mockResolvedValue({ text: 'Ada', image: { src: PNG, width: 160 } });
    await renderDialog();
    const remove = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent === 'Remove' || b.textContent === 'modal:accountSettings.signatureImageRemove',
    );
    if (!remove) throw new Error('remove button not rendered');
    act(() => remove.click());
    await save();
    expect(api.setSignatureImage).toHaveBeenCalledWith('acct-1', null);
  });

  it('does not rewrite an unchanged image', async () => {
    api.getFullSignature.mockResolvedValue({ text: 'Ada', image: { src: PNG, width: 160 } });
    await renderDialog();
    await save();
    expect(api.setSignatureImage).not.toHaveBeenCalled();
  });
});
