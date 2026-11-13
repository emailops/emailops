// Settings → Appearance: the account new blank messages start from, stored in
// the SQLite preferences.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  currentPlatform: () => 'macos',
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { DEFAULT_SENDER_PREF } from '@/lib/composeSender';
import { useAccountStore } from '@/stores/accountStore';
import { useDefaultSenderStore } from '@/stores/defaultSenderStore';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';
import { DefaultSenderSetting } from './DefaultSenderSetting';

function account(id: string, enabled = true): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@mail.example`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled,
    syncFromTimestamp: null,
    color: 'blue',
  };
}

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useAccountStore.setState({ accounts: [account('home'), account('work'), account('old', false)] });
  useDefaultSenderStore.setState({ accountId: null });
  vi.mocked(api.getPref).mockReset();
  vi.mocked(api.getPref).mockResolvedValue(null);
  vi.mocked(api.setPref).mockReset();
  vi.mocked(api.setPref).mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(<DefaultSenderSetting />);
  });
}

const select = () => container.querySelector('select') as HTMLSelectElement;

async function choose(value: string) {
  await act(async () => {
    select().value = value;
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('DefaultSenderSetting', () => {
  it("offers the selected view's account and each enabled account, following the view by default", async () => {
    await render();
    expect(container.textContent).toContain('Send new messages from');
    expect(Array.from(select().options).map((o) => o.textContent)).toEqual([
      'Account of the selected view',
      'home@mail.example',
      'work@mail.example',
    ]);
    expect(select().value).toBe('');
  });

  it('stores the chosen account', async () => {
    await render();
    await choose('work');
    expect(api.setPref).toHaveBeenCalledWith(DEFAULT_SENDER_PREF, 'work');
    expect(useDefaultSenderStore.getState().accountId).toBe('work');
  });

  it("stores going back to the selected view's account as an empty value", async () => {
    useDefaultSenderStore.setState({ accountId: 'work' });
    await render();
    await choose('');
    expect(api.setPref).toHaveBeenCalledWith(DEFAULT_SENDER_PREF, '');
    expect(useDefaultSenderStore.getState().accountId).toBeNull();
  });

  it("shows the selected view's account when the stored default is disabled", async () => {
    useDefaultSenderStore.setState({ accountId: 'old' });
    await render();
    expect(select().value).toBe('');
  });

  it('shows and logs a failed save, keeping the old choice', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await render();
    await choose('home');
    expect(container.textContent).toContain('disk full');
    expect(useDefaultSenderStore.getState().accountId).toBeNull();
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('disk full'))).toBe(
      true,
    );
  });

  it('loads the stored default', async () => {
    vi.mocked(api.getPref).mockResolvedValue('work');
    await useDefaultSenderStore.getState().load();
    expect(api.getPref).toHaveBeenCalledWith(DEFAULT_SENDER_PREF);
    expect(useDefaultSenderStore.getState().accountId).toBe('work');
  });
});
