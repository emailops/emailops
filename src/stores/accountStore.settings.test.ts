// accountStore account-settings actions: sidebar reordering, enabling an
// account, and moving its sync start date. Each writes to the backend first and
// only then changes what the sidebar shows; a refused write leaves the list as
// it was and reports an error that is not pinned to one account.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Account } from '@/types';

const api = vi.hoisted(() => ({
  listAccounts: vi.fn(),
  reorderAccounts: vi.fn(),
  setAccountEnabled: vi.fn(),
  updateAccountSyncFrom: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

vi.mock('@/lib/api', () => api);

import { useAccountStore } from './accountStore';

function account(id: string, extra: Partial<Account> = {}): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@example.com`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
    ...extra,
  };
}

const ids = () => useAccountStore.getState().accounts.map((a) => a.id);

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.reorderAccounts.mockResolvedValue(undefined);
  api.setAccountEnabled.mockResolvedValue(undefined);
  api.listAccounts.mockImplementation(() => Promise.resolve(useAccountStore.getState().accounts));
  useAccountStore.setState({
    accounts: [account('a'), account('b'), account('c')],
    activeAccountId: 'a',
    error: null,
    errorAccountId: null,
  });
});

describe('accountStore reordering', () => {
  it('moves an account one place up and reloads the list', async () => {
    await useAccountStore.getState().moveAccountUp('c');

    expect(api.reorderAccounts).toHaveBeenCalledWith(['a', 'c', 'b']);
    expect(api.listAccounts).toHaveBeenCalled();
  });

  it('moves an account one place down', async () => {
    await useAccountStore.getState().moveAccountDown('a');

    expect(api.reorderAccounts).toHaveBeenCalledWith(['b', 'a', 'c']);
  });

  it('does nothing past either end or for an unknown account', async () => {
    const s = useAccountStore.getState();
    await s.moveAccountUp('a');
    await s.moveAccountDown('c');
    await s.moveAccountUp('zzz');
    await s.moveAccountDown('zzz');

    expect(api.reorderAccounts).not.toHaveBeenCalled();
  });

  it('reports a refused reorder without pinning it to an account', async () => {
    api.reorderAccounts.mockRejectedValue(new Error('locked'));

    await useAccountStore.getState().moveAccountDown('b');
    await useAccountStore.getState().moveAccountUp('b');

    expect(api.reorderAccounts).toHaveBeenCalledTimes(2);
    expect(useAccountStore.getState().error).toContain('locked');
    expect(useAccountStore.getState().errorAccountId).toBeNull();
    expect(ids()).toEqual(['a', 'b', 'c']);
  });
});

describe('accountStore.setAccountEnabled', () => {
  it('changes only the account the backend accepted', async () => {
    await useAccountStore.getState().setAccountEnabled('b', false);

    expect(api.setAccountEnabled).toHaveBeenCalledWith('b', false);
    expect(useAccountStore.getState().accounts.map((a) => a.enabled)).toEqual([true, false, true]);
  });

  it('leaves the account as it was when the change is refused', async () => {
    api.setAccountEnabled.mockRejectedValue(new Error('refused'));

    await useAccountStore.getState().setAccountEnabled('b', false);

    expect(useAccountStore.getState().accounts[1].enabled).toBe(true);
    expect(useAccountStore.getState().error).toContain('refused');
  });
});

describe('accountStore.updateAccountSyncFrom', () => {
  it('replaces the account with the one the backend returns', async () => {
    api.updateAccountSyncFrom.mockResolvedValue(account('b', { syncFromTimestamp: 1_700_000_000 }));

    const updated = await useAccountStore.getState().updateAccountSyncFrom('b', 1_700_000_000);

    expect(updated.syncFromTimestamp).toBe(1_700_000_000);
    expect(useAccountStore.getState().accounts.map((a) => a.syncFromTimestamp)).toEqual([null, 1_700_000_000, null]);
  });

  it('reports and rethrows a refused change', async () => {
    api.updateAccountSyncFrom.mockRejectedValue(new Error('date in the future'));

    await expect(useAccountStore.getState().updateAccountSyncFrom('b', 1)).rejects.toThrow('date in the future');

    expect(useAccountStore.getState().error).toContain('date in the future');
    expect(useAccountStore.getState().accounts[1].syncFromTimestamp).toBeNull();
  });
});
