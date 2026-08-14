// Unit tests for accountStore unified-mode helpers (pure functions) and the
// store actions that must respect the "All accounts" sentinel (api mocked).

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Account } from '@/types';
import {
  ALL_ACCOUNTS_ID,
  isUnifiedMode,
  planChatAccountChange,
  reduceSyncProgress,
  type SyncProgress,
  selectAccountById,
  selectEffectiveAccountId,
  selectStartupAccountId,
  toQueryAccountId,
  useAccountStore,
} from './accountStore';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  listAccounts: vi.fn(async () => []),
  removeAccount: vi.fn(async () => {}),
  syncAccount: vi.fn(async () => {}),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';

function makeAccount(id: string, enabled = true): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@example.com`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled,
  } as Account;
}

function makeProgress(accountId: string, status: string): SyncProgress {
  return { accountId, status, current: 0, total: 0, message: `${status} for ${accountId}` };
}

beforeEach(() => {
  vi.clearAllMocks();
  useAccountStore.setState({
    accounts: [],
    activeAccountId: null,
    isLoading: false,
    isSyncing: false,
    syncProgress: null,
    error: null,
    errorAccountId: null,
    currentSyncId: 0,
    setupPendingAccountId: null,
    pendingSyncAccountIds: new Set<string>(),
  });
});

// ── pure helpers ──────────────────────────────────────────────────────────────

describe('isUnifiedMode', () => {
  it('is true only for the sentinel', () => {
    expect(isUnifiedMode(ALL_ACCOUNTS_ID)).toBe(true);
    expect(isUnifiedMode('acc-1')).toBe(false);
    expect(isUnifiedMode(null)).toBe(false);
  });
});

describe('toQueryAccountId', () => {
  it('maps the sentinel to null (backend "all enabled accounts")', () => {
    expect(toQueryAccountId(ALL_ACCOUNTS_ID)).toBeNull();
  });

  it('passes real ids and null through unchanged', () => {
    expect(toQueryAccountId('acc-1')).toBe('acc-1');
    expect(toQueryAccountId(null)).toBeNull();
  });
});

describe('selectEffectiveAccountId', () => {
  const accounts = [makeAccount('a', false), makeAccount('b'), makeAccount('c')];

  it('returns the active id unchanged when not unified', () => {
    expect(selectEffectiveAccountId(accounts, 'c')).toBe('c');
    expect(selectEffectiveAccountId(accounts, null)).toBeNull();
  });

  it('returns the first ENABLED account in unified mode', () => {
    expect(selectEffectiveAccountId(accounts, ALL_ACCOUNTS_ID)).toBe('b');
  });

  it('falls back to the first account when none are enabled', () => {
    const allDisabled = [makeAccount('a', false), makeAccount('b', false)];
    expect(selectEffectiveAccountId(allDisabled, ALL_ACCOUNTS_ID)).toBe('a');
  });

  it('returns null in unified mode with no accounts', () => {
    expect(selectEffectiveAccountId([], ALL_ACCOUNTS_ID)).toBeNull();
  });
});

describe('selectAccountById', () => {
  const accounts = [makeAccount('a'), makeAccount('b')];

  it('returns the matching account', () => {
    expect(selectAccountById(accounts, 'b')).toEqual(makeAccount('b'));
  });

  it('returns null for a null id', () => {
    expect(selectAccountById(accounts, null)).toBeNull();
  });

  it('returns null instead of undefined when the id has no match (regression)', () => {
    // AccountSettingsDialog used to receive `accounts.find(...)!` directly —
    // when the account was deleted out from under a still-open dialog, that
    // resolved to `undefined` at runtime (the `!` is compile-time only) and
    // crashed the whole app via the root ErrorBoundary. `null` lets the
    // caller unmount the dialog instead of rendering it with no account.
    expect(selectAccountById(accounts, 'deleted-id')).toBeNull();
  });
});

// ── reduceSyncProgress ────────────────────────────────────────────────────────

describe('reduceSyncProgress', () => {
  const base = { error: null, errorAccountId: null, pendingSyncAccountIds: new Set<string>() };

  it('null progress clears sync state and pending set', () => {
    const s = reduceSyncProgress({ ...base, pendingSyncAccountIds: new Set(['a']) }, null);
    expect(s.syncProgress).toBeNull();
    expect(s.isSyncing).toBe(false);
    expect(s.pendingSyncAccountIds.size).toBe(0);
  });

  it('non-terminal progress keeps syncing', () => {
    const s = reduceSyncProgress(base, makeProgress('a', 'fetching'));
    expect(s.isSyncing).toBe(true);
  });

  it('terminal progress with empty pending stops syncing', () => {
    const s = reduceSyncProgress(base, makeProgress('a', 'complete'));
    expect(s.isSyncing).toBe(false);
  });

  it('terminal progress removes the account from pending and keeps syncing while others remain', () => {
    const s = reduceSyncProgress(
      { ...base, pendingSyncAccountIds: new Set(['a', 'b']) },
      makeProgress('a', 'complete'),
    );
    expect(s.pendingSyncAccountIds.has('a')).toBe(false);
    expect(s.pendingSyncAccountIds.has('b')).toBe(true);
    expect(s.isSyncing).toBe(true);
  });

  it('terminal progress for the LAST pending account stops syncing', () => {
    const s = reduceSyncProgress({ ...base, pendingSyncAccountIds: new Set(['a']) }, makeProgress('a', 'complete'));
    expect(s.pendingSyncAccountIds.size).toBe(0);
    expect(s.isSyncing).toBe(false);
  });

  it('error progress records the error scoped to its account', () => {
    const s = reduceSyncProgress({ ...base, pendingSyncAccountIds: new Set(['a', 'b']) }, makeProgress('a', 'error'));
    expect(s.error).toContain('error for a');
    expect(s.errorAccountId).toBe('a');
    expect(s.isSyncing).toBe(true); // b still pending
  });

  it('does not mutate the input pending set', () => {
    const pending = new Set(['a']);
    reduceSyncProgress({ ...base, pendingSyncAccountIds: pending }, makeProgress('a', 'complete'));
    expect(pending.has('a')).toBe(true);
  });
});

// ── which account the app opens on ───────────────────────────────────────────

describe('selectStartupAccountId', () => {
  const accounts = [makeAccount('a'), makeAccount('b')];

  it('reopens the account the user last selected', () => {
    expect(selectStartupAccountId(accounts, 'b')).toBe('b');
  });

  it('reopens the unified view when that is where the user left off', () => {
    expect(selectStartupAccountId(accounts, ALL_ACCOUNTS_ID)).toBe(ALL_ACCOUNTS_ID);
  });

  it('falls back to the first account with nothing remembered', () => {
    expect(selectStartupAccountId(accounts, null)).toBe('a');
  });

  it('falls back when the remembered account is gone', () => {
    // Deleting the account that was last active must not strand the app on an
    // id the backend will reject as NotFound.
    expect(selectStartupAccountId(accounts, 'deleted')).toBe('a');
  });

  it('falls back when the remembered account was since disabled', () => {
    // A disabled account shows no mail, so landing on it looks like a broken
    // inbox rather than a remembered choice.
    expect(selectStartupAccountId([makeAccount('a'), makeAccount('b', false)], 'b')).toBe('a');
  });

  it('falls back to unified when every account is disabled', () => {
    // `selectEffectiveAccountId` still resolves the sentinel to a concrete
    // account, so this degrades rather than leaving the app with no selection.
    expect(selectStartupAccountId([makeAccount('a', false)], 'a')).toBe('a');
  });

  it('is null with no accounts at all', () => {
    expect(selectStartupAccountId([], 'a')).toBeNull();
  });

  it('drops a remembered unified view once a single account is left', () => {
    // The sidebar only offers "All accounts" with more than one, so restoring
    // it here would select an entry that is not on screen.
    expect(selectStartupAccountId([makeAccount('a')], ALL_ACCOUNTS_ID)).toBe('a');
  });
});

// ── store actions vs. the sentinel ───────────────────────────────────────────

describe('fetchAccounts', () => {
  it('auto-selects the first account when none is active', async () => {
    vi.mocked(api.listAccounts).mockResolvedValue([makeAccount('a'), makeAccount('b')]);
    await useAccountStore.getState().fetchAccounts();
    expect(useAccountStore.getState().activeAccountId).toBe('a');
  });

  it('reopens the account remembered from the last session', async () => {
    vi.mocked(api.listAccounts).mockResolvedValue([makeAccount('a'), makeAccount('b')]);
    vi.mocked(api.getPref).mockResolvedValue('b');
    await useAccountStore.getState().fetchAccounts();
    expect(useAccountStore.getState().activeAccountId).toBe('b');
  });

  it('still selects the first account when the pref read fails', async () => {
    // A preferences failure must not leave the app with no account selected.
    vi.mocked(api.listAccounts).mockResolvedValue([makeAccount('a'), makeAccount('b')]);
    vi.mocked(api.getPref).mockRejectedValue(new Error('db closed'));
    await useAccountStore.getState().fetchAccounts();
    expect(useAccountStore.getState().activeAccountId).toBe('a');
  });

  it('does not re-read the pref when an account is already active', async () => {
    vi.mocked(api.listAccounts).mockResolvedValue([makeAccount('a'), makeAccount('b')]);
    useAccountStore.setState({ activeAccountId: 'b' });
    await useAccountStore.getState().fetchAccounts();
    expect(api.getPref).not.toHaveBeenCalled();
  });

  it('does NOT clobber the All-accounts sentinel (regression)', async () => {
    // fetchAccounts re-runs after reorder/settings-save/account-add; unified
    // mode must survive those refetches.
    vi.mocked(api.listAccounts).mockResolvedValue([makeAccount('a'), makeAccount('b')]);
    useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
    await useAccountStore.getState().fetchAccounts();
    expect(useAccountStore.getState().activeAccountId).toBe(ALL_ACCOUNTS_ID);
  });
});

describe('setActiveAccount', () => {
  it('remembers the choice so the next launch lands there', async () => {
    useAccountStore.getState().setActiveAccount('b');
    expect(api.setPref).toHaveBeenCalledWith('active_account', 'b');
  });

  it('remembers the unified view too', () => {
    useAccountStore.getState().setActiveAccount(ALL_ACCOUNTS_ID);
    expect(api.setPref).toHaveBeenCalledWith('active_account', ALL_ACCOUNTS_ID);
  });

  it('does not persist a cleared selection', () => {
    // `null` means "no accounts exist" — writing it would erase a perfectly
    // good memory the moment the last account is deleted.
    useAccountStore.getState().setActiveAccount(null);
    expect(api.setPref).not.toHaveBeenCalled();
  });
});

describe('removeAccount', () => {
  it('keeps the sentinel active when a member account is removed', async () => {
    useAccountStore.setState({
      accounts: [makeAccount('a'), makeAccount('b')],
      activeAccountId: ALL_ACCOUNTS_ID,
    });
    await useAccountStore.getState().removeAccount('a');
    expect(useAccountStore.getState().activeAccountId).toBe(ALL_ACCOUNTS_ID);
  });

  it('rethrows on failure so the confirm dialog can show the real error (regression)', async () => {
    // Without the rethrow, the delete confirmation UI closes as if the
    // deletion succeeded — App.tsx logs "Account deleted" and the caller's
    // catch block (which surfaces `deleteError`) never runs — while the
    // account silently remains in the list underneath a stale success log.
    useAccountStore.setState({ accounts: [makeAccount('a')] });
    vi.mocked(api.removeAccount).mockRejectedValue(new Error('database is locked'));

    await expect(useAccountStore.getState().removeAccount('a')).rejects.toThrow('database is locked');

    expect(useAccountStore.getState().accounts).toEqual([makeAccount('a')]);
    expect(useAccountStore.getState().error).toBe('database is locked');
  });
});

describe('syncAllAccounts', () => {
  it('enqueues a sync for every given account and tracks them as pending', async () => {
    await useAccountStore.getState().syncAllAccounts(['a', 'b']);
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('a');
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('b');
    const state = useAccountStore.getState();
    expect(state.isSyncing).toBe(true);
    expect(state.pendingSyncAccountIds).toEqual(new Set(['a', 'b']));
  });

  it('is a no-op while a sync is already running', async () => {
    useAccountStore.setState({ isSyncing: true });
    await useAccountStore.getState().syncAllAccounts(['a']);
    expect(vi.mocked(api.syncAccount)).not.toHaveBeenCalled();
  });

  it('drops an account from pending when its enqueue fails, keeping the rest', async () => {
    vi.mocked(api.syncAccount).mockImplementation(async (id: string) => {
      if (id === 'a') throw new Error('enqueue failed');
    });
    await useAccountStore.getState().syncAllAccounts(['a', 'b']);
    const state = useAccountStore.getState();
    expect(state.pendingSyncAccountIds).toEqual(new Set(['b']));
    expect(state.errorAccountId).toBe('a');
    expect(state.isSyncing).toBe(true);
  });

  it('progress events drain pending until syncing stops', async () => {
    await useAccountStore.getState().syncAllAccounts(['a', 'b']);
    useAccountStore.getState().setSyncProgress(makeProgress('a', 'complete'));
    expect(useAccountStore.getState().isSyncing).toBe(true);
    useAccountStore.getState().setSyncProgress(makeProgress('b', 'complete'));
    expect(useAccountStore.getState().isSyncing).toBe(false);
  });
});

describe('planChatAccountChange', () => {
  it('moves the mail list with chat when the list shows one account', () => {
    // Lockstep: otherwise you can browse account A while chat answers from B,
    // and hand it an email it cannot read.
    expect(planChatAccountChange('acct-b', 'acct-a')).toEqual({
      chatAccountId: 'acct-b',
      mailAccountId: 'acct-b',
    });
  });

  it('leaves the list unified when it is showing All accounts', () => {
    // "All accounts" is a view the user deliberately chose; retargeting a chat
    // must not collapse it. Cross-account emails are simply not offered as
    // context while it is up — see offeredChatContext.
    expect(planChatAccountChange('acct-b', ALL_ACCOUNTS_ID)).toEqual({
      chatAccountId: 'acct-b',
      mailAccountId: null,
    });
  });

  it('treats a null selection as unified', () => {
    // Before accounts load there is no concrete selection to drag along.
    expect(planChatAccountChange('acct-b', null)).toEqual({
      chatAccountId: 'acct-b',
      mailAccountId: null,
    });
  });
});
