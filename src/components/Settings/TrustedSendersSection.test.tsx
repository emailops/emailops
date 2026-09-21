// Regression: trusting a sender was a one-way door. `addTrustedSender` is
// wired to the blocked-images banner, but `listTrustedSenders` and
// `removeTrustedSender` — both implemented, registered and wrapped in api.ts —
// had no caller anywhere in the UI. One click, next to "Show images", granted
// permanent permission to load remote content (tracking pixels included) from
// that sender, and there was no screen listing the grants and no way to revoke
// one.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/lib/api', () => ({
  listTrustedSenders: vi.fn(),
  removeTrustedSender: vi.fn(),
}));

// The store slice must be referentially stable across renders — Zustand returns
// the same array, and a fresh one per call would re-fire the load effect
// forever.
vi.mock('@/stores/accountStore', () => {
  const state = {
    accounts: [
      { id: 'acc-1', email: 'work@example.test' },
      { id: 'acc-2', email: 'personal@example.test' },
    ],
  };
  return { useAccountStore: (selector: (s: typeof state) => unknown) => selector(state) };
});

vi.mock('@/stores/logStore', () => {
  const state = { addLog: () => {} };
  return { useLogStore: (selector: (s: typeof state) => unknown) => selector(state) };
});

import * as api from '@/lib/api';
import { TrustedSendersSection } from './TrustedSendersSection';

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
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => {
    root.render(<TrustedSendersSection />);
  });
  await act(async () => {
    await Promise.resolve();
  });
}

function text(): string {
  return container.textContent ?? '';
}

function removeButtons(): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll('button[data-sender]'));
}

describe('TrustedSendersSection', () => {
  it('lists the grants of every account, not just one', async () => {
    vi.mocked(api.listTrustedSenders).mockImplementation(async (accountId: string) =>
      accountId === 'acc-1' ? ['news@shop.test'] : ['alerts@bank.test'],
    );

    await mount();

    expect(api.listTrustedSenders).toHaveBeenCalledWith('acc-1');
    expect(api.listTrustedSenders).toHaveBeenCalledWith('acc-2');
    expect(text()).toContain('news@shop.test');
    expect(text()).toContain('alerts@bank.test');
  });

  it('revokes a grant and drops it from the list', async () => {
    vi.mocked(api.listTrustedSenders).mockImplementation(async (accountId: string) =>
      accountId === 'acc-1' ? ['news@shop.test'] : [],
    );
    vi.mocked(api.removeTrustedSender).mockResolvedValue(undefined);

    await mount();
    const button = removeButtons().find((b) => b.dataset.sender === 'news@shop.test');
    expect(button, 'every listed grant needs a revoke control').toBeDefined();

    await act(async () => {
      button?.click();
    });

    expect(api.removeTrustedSender).toHaveBeenCalledWith('acc-1', 'news@shop.test');
    expect(text()).not.toContain('news@shop.test');
  });

  it('keeps the grant visible when revoking fails', async () => {
    // Only one account holds the grant — otherwise the other account's copy
    // keeps the address on screen and the assertion passes for the wrong
    // reason, which is exactly what an earlier draft of this test did.
    vi.mocked(api.listTrustedSenders).mockImplementation(async (accountId: string) =>
      accountId === 'acc-1' ? ['news@shop.test'] : [],
    );
    vi.mocked(api.removeTrustedSender).mockRejectedValue(new Error('db locked'));

    await mount();
    await act(async () => {
      removeButtons()[0]?.click();
    });

    // Removing it from the list on a failed revoke would tell the user the
    // grant is gone while it is still loading their images.
    expect(text()).toContain('news@shop.test');
  });

  it('says so when an account has granted nothing', async () => {
    vi.mocked(api.listTrustedSenders).mockResolvedValue([]);

    await mount();

    expect(removeButtons()).toHaveLength(0);
    expect(text()).toContain('settings:privacy.trustedSenders.none');
  });
});
