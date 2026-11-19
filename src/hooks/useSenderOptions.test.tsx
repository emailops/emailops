// The From list of every composer: enabled accounts, the one with the most
// sent mail first, counted in the local DB when the composer opens.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({ getSentCounts: vi.fn() }));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';
import { useSenderOptions } from './useSenderOptions';

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

const accounts = [account('home'), account('work'), account('old', false)];
let latest: { value: string; label: string }[] = [];

function Probe() {
  latest = useSenderOptions(accounts);
  return null;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.mocked(api.getSentCounts).mockReset();
  container = document.createElement('div');
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
});

async function render() {
  await act(async () => {
    root.render(<Probe />);
  });
}

describe('useSenderOptions', () => {
  it('lists the enabled accounts with the most used sender first', async () => {
    vi.mocked(api.getSentCounts).mockResolvedValue([
      { accountId: 'home', sentCount: 3 },
      { accountId: 'work', sentCount: 40 },
    ]);
    await render();
    expect(latest).toEqual([
      { value: 'work', label: 'work@mail.example' },
      { value: 'home', label: 'home@mail.example' },
    ]);
  });

  it('keeps the account order and logs when the counts cannot be read', async () => {
    vi.mocked(api.getSentCounts).mockRejectedValue(new Error('db locked'));
    await render();
    expect(latest.map((o) => o.value)).toEqual(['home', 'work']);
    expect(useLogStore.getState().entries.some((l) => l.message.includes('db locked'))).toBe(true);
  });
});
