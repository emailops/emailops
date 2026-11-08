// The inbox loads the sender names of the accounts it shows, once, and again
// when one of them finishes a sync — the moment new senders may have arrived.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const getSenderNames = vi.fn<(accountId: string) => Promise<Record<string, string>>>();

vi.mock('@/lib/api', () => ({
  getSenderNames: (accountId: string) => getSenderNames(accountId),
}));

import { useAccountStore } from '@/stores/accountStore';
import { INITIAL_SENDER_NAMES, useSenderNameStore } from '@/stores/senderNameStore';
import type { Email } from '@/types';
import { useSenderNames } from './useSenderNames';

const row = (id: string, accountId: string) => ({ id, accountId }) as Email;

let nameOf: ReturnType<typeof useSenderNames> = () => undefined;

function Probe({ emails }: { emails: Email[] }) {
  nameOf = useSenderNames(emails);
  return null;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  getSenderNames.mockReset();
  getSenderNames.mockResolvedValue({ 'ana@example.com': 'Ana Ruiz' });
  useSenderNameStore.setState(INITIAL_SENDER_NAMES);
  useAccountStore.setState({ syncingAccountIds: new Set<string>() });
  container = document.createElement('div');
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
});

async function render(emails: Email[]) {
  await act(async () => {
    root.render(<Probe emails={emails} />);
  });
}

describe('useSenderNames', () => {
  it('loads each account on screen once and resolves names from it', async () => {
    await render([row('e1', 'acc-1'), row('e2', 'acc-1'), row('e3', 'acc-2')]);
    await render([row('e1', 'acc-1'), row('e3', 'acc-2'), row('e4', 'acc-2')]);

    expect(getSenderNames.mock.calls.map(([id]) => id).sort()).toEqual(['acc-1', 'acc-2']);
    expect(nameOf('acc-1', 'ana@example.com')).toBe('Ana Ruiz');
    expect(nameOf('acc-1', 'luis@example.org')).toBeUndefined();
  });

  it('reloads an account when its sync ends', async () => {
    await render([row('e1', 'acc-1')]);
    await act(async () => useAccountStore.setState({ syncingAccountIds: new Set(['acc-1']) }));
    expect(getSenderNames).toHaveBeenCalledTimes(1);

    getSenderNames.mockResolvedValue({ 'ana@example.com': 'Ana R.' });
    await act(async () => useAccountStore.setState({ syncingAccountIds: new Set<string>() }));

    expect(getSenderNames).toHaveBeenCalledTimes(2);
    expect(nameOf('acc-1', 'ana@example.com')).toBe('Ana R.');
  });
});
