// Settings → Junk → Blocked senders: the list, and Unblock as the inverse of
// every block.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  listBlockedSenders: vi.fn(),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useAccountStore } from '@/stores/accountStore';
import { useSenderStore } from '@/stores/senderStore';
import type { Account } from '@/types';
import { BlockedSendersSettings } from './BlockedSendersSettings';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useAccountStore.setState({ accounts: [{ id: 'acc', email: 'ana@example.com', provider: 'gmail' } as Account] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
  useSenderStore.setState({ statusByEmail: {}, blocked: [], dialog: null });
});

async function render() {
  await act(async () => {
    root.render(<BlockedSendersSettings />);
  });
}

describe('BlockedSendersSettings', () => {
  it('says so when nobody is blocked', async () => {
    vi.mocked(api.listBlockedSenders).mockResolvedValue([]);
    await render();
    expect(container.textContent).toContain('You haven’t blocked anyone');
  });

  it('lists the blocked senders and unblocks through the confirmation', async () => {
    vi.mocked(api.listBlockedSenders).mockResolvedValue([
      { accountId: 'acc', address: 'deals@shop.example', createdAt: 1_700_000_000 },
    ]);
    await render();

    expect(api.listBlockedSenders).toHaveBeenCalledWith(null);
    expect(container.textContent).toContain('deals@shop.example');
    const unblock = [...container.querySelectorAll('button')].find((b) => b.textContent === 'Unblock');
    await act(async () => unblock?.click());

    expect(useSenderStore.getState().dialog).toEqual({
      type: 'unblock',
      accountId: 'acc',
      address: 'deals@shop.example',
    });
  });
});
