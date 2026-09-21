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
  selectIsSyncing,
  toQueryAccountId,
  useAccountStore,
} from './accountStore';

vi.mock('@/lib/api', () => ({
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
  // `clearAllMocks` only clears recorded calls — a `mockRejectedValue` from a
  // previous test survives it and silently poisons the next one. Re-establish
  // the happy-path implementations explicitly.
  vi.mocked(api.listAccounts).mockImplementation(async () => []);
  vi.mocked(api.removeAccount).mockImplementation(async () => {});
  vi.mocked(api.syncAccount).mockImplementation(async () => {});
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
    syncingAccountIds: new Set<string>(),
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
  const base = { error: null, errorAccountId: null, syncingAccountIds: new Set<string>() };

  it('null progress clears sync state and the syncing set', () => {
    const s = reduceSyncProgress({ ...base, syncingAccountIds: new Set(['a']) }, null);
    expect(s.syncProgress).toBeNull();
    expect(s.isSyncing).toBe(false);
    expect(s.syncingAccountIds.size).toBe(0);
  });

  it('non-terminal progress marks its own account as syncing', () => {
    const s = reduceSyncProgress(base, makeProgress('a', 'fetching'));
    expect(s.syncingAccountIds.has('a')).toBe(true);
    expect(s.isSyncing).toBe(true);
  });

  it('non-terminal progress does not mark any OTHER account as syncing (regression)', () => {
    // The starvation bug: a single global `isSyncing` meant account A's backlog
    // made the whole app look busy, which blocked B's first sync and showed a
    // spinner over B's empty inbox.
    const s = reduceSyncProgress(base, makeProgress('a', 'fetching'));
    expect(s.syncingAccountIds.has('b')).toBe(false);
  });

  it('terminal progress for an untracked account leaves nothing syncing', () => {
    const s = reduceSyncProgress(base, makeProgress('a', 'complete'));
    expect(s.isSyncing).toBe(false);
  });

  it('terminal progress clears only its own account and keeps the others syncing', () => {
    const s = reduceSyncProgress({ ...base, syncingAccountIds: new Set(['a', 'b']) }, makeProgress('a', 'complete'));
    expect(s.syncingAccountIds.has('a')).toBe(false);
    expect(s.syncingAccountIds.has('b')).toBe(true);
    expect(s.isSyncing).toBe(true);
  });

  it('terminal progress for the LAST syncing account stops syncing', () => {
    const s = reduceSyncProgress({ ...base, syncingAccountIds: new Set(['a']) }, makeProgress('a', 'complete'));
    expect(s.syncingAccountIds.size).toBe(0);
    expect(s.isSyncing).toBe(false);
  });

  it('error progress records the error scoped to its account', () => {
    const s = reduceSyncProgress({ ...base, syncingAccountIds: new Set(['a', 'b']) }, makeProgress('a', 'error'));
    expect(s.error).toContain('error for a');
    expect(s.errorAccountId).toBe('a');
    expect(s.isSyncing).toBe(true); // b still syncing
  });

  it('does not mutate the input syncing set', () => {
    const syncing = new Set(['a']);
    reduceSyncProgress({ ...base, syncingAccountIds: syncing }, makeProgress('a', 'complete'));
    expect(syncing.has('a')).toBe(true);
  });
});

// ── selectIsSyncing ───────────────────────────────────────────────────────────

describe('selectIsSyncing', () => {
  it('is true for an account that is syncing', () => {
    expect(selectIsSyncing(new Set(['a']), 'a')).toBe(true);
  });

  it('is FALSE for an idle account while a different one syncs (regression)', () => {
    // A freshly added account must not inherit another account's spinner —
    // that is what made a stalled first sync look like work in progress.
    expect(selectIsSyncing(new Set(['a']), 'b')).toBe(false);
  });

  it('is true in unified mode when any account is syncing', () => {
    expect(selectIsSyncing(new Set(['a']), ALL_ACCOUNTS_ID)).toBe(true);
  });

  it('is false in unified mode when nothing is syncing', () => {
    expect(selectIsSyncing(new Set(), ALL_ACCOUNTS_ID)).toBe(false);
  });

  it('falls back to "any account" for a null scope', () => {
    expect(selectIsSyncing(new Set(['a']), null)).toBe(true);
    expect(selectIsSyncing(new Set(), null)).toBe(false);
  });
});

// ── store actions vs. the sentinel ───────────────────────────────────────────

describe('fetchAccounts', () => {
  it('auto-selects the first account when none is active', async () => {
    vi.mocked(api.listAccounts).mockResolvedValue([makeAccount('a'), makeAccount('b')]);
    await useAccountStore.getState().fetchAccounts();
    expect(useAccountStore.getState().activeAccountId).toBe('a');
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

describe('syncAccount', () => {
  it('enqueues the sync and marks that account as syncing', async () => {
    await useAccountStore.getState().syncAccount('a');
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('a');
    expect(useAccountStore.getState().syncingAccountIds).toEqual(new Set(['a']));
  });

  it('enqueues even while a DIFFERENT account is already syncing (regression)', async () => {
    // The reported bug: after 3 months offline, the existing accounts held a
    // global `isSyncing` latch true for the whole backfill, so a newly added
    // account's one-and-only auto-sync returned before invoking anything and
    // was never retried — 15 minutes later it still had zero emails and no
    // `sync_state` row at all.
    useAccountStore.setState({ syncingAccountIds: new Set(['busy-account']) });

    await useAccountStore.getState().syncAccount('brand-new');

    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('brand-new');
    expect(useAccountStore.getState().syncingAccountIds).toEqual(new Set(['busy-account', 'brand-new']));
  });

  it('does not re-enqueue an account that is already syncing', async () => {
    // Scoped to the one account, so it cannot starve any other. Switching
    // back and forth between accounts during a long backfill would otherwise
    // queue a second full backfill behind the first.
    useAccountStore.setState({ syncingAccountIds: new Set(['a']) });
    await useAccountStore.getState().syncAccount('a');
    expect(vi.mocked(api.syncAccount)).not.toHaveBeenCalled();
  });

  it('stops tracking the account when its enqueue fails', async () => {
    vi.mocked(api.syncAccount).mockRejectedValue(new Error('enqueue failed'));

    await expect(useAccountStore.getState().syncAccount('a')).rejects.toThrow('enqueue failed');

    const state = useAccountStore.getState();
    expect(state.syncingAccountIds.has('a')).toBe(false);
    expect(state.errorAccountId).toBe('a');
  });

  it('leaves other accounts tracked when one enqueue fails', async () => {
    useAccountStore.setState({ syncingAccountIds: new Set(['busy-account']) });
    vi.mocked(api.syncAccount).mockRejectedValue(new Error('enqueue failed'));

    await expect(useAccountStore.getState().syncAccount('a')).rejects.toThrow('enqueue failed');

    expect(useAccountStore.getState().syncingAccountIds).toEqual(new Set(['busy-account']));
  });
});

describe('syncAllAccounts', () => {
  it('enqueues a sync for every given account and tracks them all', async () => {
    await useAccountStore.getState().syncAllAccounts(['a', 'b']);
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('a');
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('b');
    const state = useAccountStore.getState();
    expect(state.isSyncing).toBe(true);
    expect(state.syncingAccountIds).toEqual(new Set(['a', 'b']));
  });

  it('enqueues even while another account is already syncing (regression)', async () => {
    // Same latch as syncAccount: switching to "All accounts" during a backlog
    // used to enqueue nothing at all.
    useAccountStore.setState({ syncingAccountIds: new Set(['busy-account']) });

    await useAccountStore.getState().syncAllAccounts(['a']);

    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('a');
    expect(useAccountStore.getState().syncingAccountIds).toEqual(new Set(['busy-account', 'a']));
  });

  it('skips the accounts already syncing and enqueues the rest', async () => {
    useAccountStore.setState({ syncingAccountIds: new Set(['a']) });
    await useAccountStore.getState().syncAllAccounts(['a', 'b']);
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.syncAccount)).toHaveBeenCalledWith('b');
  });

  it('is a no-op for an empty account list', async () => {
    await useAccountStore.getState().syncAllAccounts([]);
    expect(vi.mocked(api.syncAccount)).not.toHaveBeenCalled();
  });

  it('drops an account from tracking when its enqueue fails, keeping the rest', async () => {
    vi.mocked(api.syncAccount).mockImplementation(async (id: string) => {
      if (id === 'a') throw new Error('enqueue failed');
    });
    await useAccountStore.getState().syncAllAccounts(['a', 'b']);
    const state = useAccountStore.getState();
    expect(state.syncingAccountIds).toEqual(new Set(['b']));
    expect(state.errorAccountId).toBe('a');
    expect(state.isSyncing).toBe(true);
  });

  it('progress events drain the syncing set until syncing stops', async () => {
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

describe('account settings version', () => {
  it('bumps so the inbox refetches the account categories after a save', () => {
    // Regression: the onboarding wizard and the sidebar each render their own
    // AccountSettingsDialog, and only the sidebar's save invalidated the
    // category chips (the counter lived in App's local state). A user who
    // picked Promotions during onboarding got a strip showing Primary alone —
    // the categories were fetched at account creation, before the settings
    // existed, and nothing re-ran the query. The counter lives in the store so
    // any dialog's save invalidates it.
    const before = useAccountStore.getState().accountSettingsVersion;
    useAccountStore.getState().bumpAccountSettingsVersion();
    expect(useAccountStore.getState().accountSettingsVersion).toBe(before + 1);
  });
});
