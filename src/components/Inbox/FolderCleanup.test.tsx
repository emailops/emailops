// Above the Trash and Spam lists: "Empty Trash / Spam" for accounts that can
// delete forever, an explanation for Gmail, and the dialog either opens.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/api', () => ({
  getEmailCount: vi.fn(async () => 4),
  deleteForever: vi.fn(async () => undefined),
  cancelDeleteForever: vi.fn(async () => undefined),
  currentPlatform: () => 'macos',
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { ALL_ACCOUNTS_ID, useAccountStore } from '@/stores/accountStore';
import { useDeleteForeverStore } from '@/stores/deleteForeverStore';
import { useEmailStore } from '@/stores/emailStore';
import type { Account } from '@/types';
import { FolderCleanup } from './FolderCleanup';

let container: HTMLDivElement;
let root: Root;

const account = (id: string, provider: Account['provider']) => ({ id, provider, enabled: true }) as Account;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
  useDeleteForeverStore.setState({ request: null });
  useAccountStore.setState({
    accounts: [account('imap-1', 'imap'), account('g-1', 'gmail')],
    activeAccountId: 'imap-1',
  });
  useEmailStore.setState({ listScope: 'deleted' });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const q = (id: string) => document.querySelector<HTMLElement>(`[data-testid="${id}"]`);

async function render() {
  await act(async () => root.render(<FolderCleanup />));
}

describe('FolderCleanup', () => {
  it('offers Empty Trash on an IMAP account and opens the dialog with the folder count', async () => {
    await render();

    expect(q('empty-folder')?.textContent).toBe('Empty Trash');
    await act(async () => q('empty-folder')?.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    expect(useDeleteForeverStore.getState().request).toEqual({ accountId: 'imap-1', folder: 'trash', emailIds: null });
    expect(api.getEmailCount).toHaveBeenCalledWith('imap-1', 'deleted');
  });

  it('offers Empty Spam in the Spam view', async () => {
    useEmailStore.setState({ listScope: 'spam' });
    await render();

    expect(q('empty-folder')?.textContent).toBe('Empty Spam');
  });

  it('explains why a Gmail account has no Empty button', async () => {
    useAccountStore.setState({ activeAccountId: 'g-1' });
    await render();

    expect(q('empty-folder')).toBeNull();
    expect(q('empty-folder-gmail-note')?.textContent).toContain('30 days');
  });

  it('shows nothing outside Trash and Spam', async () => {
    useEmailStore.setState({ listScope: 'inbox' });
    await render();

    expect(container.textContent).toBe('');
  });

  it('shows no Empty button across all accounts, but still hosts a bulk request', async () => {
    useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
    await render();
    expect(q('empty-folder')).toBeNull();

    await act(async () =>
      useDeleteForeverStore.getState().open({ accountId: 'imap-1', folder: 'trash', emailIds: ['m-1'] }),
    );

    expect(q('delete-forever-confirm-text')?.textContent).toContain('1 message in Trash');
  });
});
