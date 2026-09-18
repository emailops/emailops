import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Account } from '@/types';

/**
 * Sentinel `activeAccountId` for the unified ("All accounts") view.
 *
 * Deliberately NOT `null`: `null` already means "no accounts exist", and
 * `fetchAccounts()` auto-selects the first account whenever `activeAccountId`
 * is falsy — a `null`-based unified mode would be evicted on every refetch
 * (reorder, settings save, account add). The sentinel is truthy, so every
 * existing `if (!activeAccountId)` guard keeps its current meaning.
 *
 * The sentinel must never reach the backend: translate with
 * `toQueryAccountId` (→ `null` → all enabled accounts) for queries that
 * support the unified view, or `selectEffectiveAccountId` for surfaces that
 * need one concrete account (compose-from, chat, feedback…). A leaked
 * sentinel fails loudly as NotFound rather than reading the wrong account.
 */
export const ALL_ACCOUNTS_ID = '__all_accounts__';

export function isUnifiedMode(id: string | null): boolean {
  return id === ALL_ACCOUNTS_ID;
}

/** Account id to send to backend queries: `null` in unified mode (maps to
 *  Rust `Option::None` = all enabled accounts), the id itself otherwise. */
export function toQueryAccountId(id: string | null): string | null {
  return isUnifiedMode(id) ? null : id;
}

/** Concrete account for surfaces that need exactly one (compose-from, chat,
 *  feedback): the first *enabled* account in unified mode, the active account
 *  otherwise. Falls back to the first account when none are enabled. */
export function selectEffectiveAccountId(accounts: Account[], activeAccountId: string | null): string | null {
  if (!isUnifiedMode(activeAccountId)) return activeAccountId;
  return accounts.find((a) => a.enabled)?.id ?? accounts[0]?.id ?? null;
}

/**
 * Account backing an id-keyed dialog (e.g. AccountSettingsDialog), or `null`
 * if it's gone. `removeAccount` filters `accounts` synchronously before the
 * caller clears its own "which account" id state, so there's a render frame
 * where the id still points at an account that just disappeared — a plain
 * `accounts.find(...)!` returns `undefined` there and crashes the component
 * (v0.6.4 regression: deleting an account blanked the whole app). Callers
 * should treat `null` as "unmount the dialog", not throw.
 */
/** Outcome of retargeting chat's account — see `planChatAccountChange`. */
export interface ChatAccountChange {
  /** Account chat answers from. */
  chatAccountId: string;
  /** Account the mail list should switch to, or `null` to leave it alone. */
  mailAccountId: string | null;
}

/**
 * What changes when the user picks a different account for chat.
 *
 * Chat is scoped to one account, so retargeting it has to be visible in what
 * the mail list shows — otherwise you can open an email from another account
 * and hand it to a chat that cannot read it. The two are therefore kept in
 * lockstep, with one deliberate exception: in "All accounts" the list is
 * showing everything *on purpose*, and collapsing it to a single account just
 * because a chat was retargeted would throw away the view the user chose. In
 * that mode the list stays unified and cross-account emails simply are not
 * offered as chat context (`offeredChatContext`).
 *
 * Pure so the coupling rule is testable without a store or a render.
 */
export function planChatAccountChange(nextAccountId: string, mailScopeId: string | null): ChatAccountChange {
  // Only move a list that is currently showing ONE concrete account. Unified
  // stays unified (above), and `null` — no selection yet, e.g. before accounts
  // finish loading — is not a selection to drag along either.
  const listShowsOneAccount = mailScopeId !== null && !isUnifiedMode(mailScopeId);
  return {
    chatAccountId: nextAccountId,
    mailAccountId: listShowsOneAccount ? nextAccountId : null,
  };
}

export function selectAccountById(accounts: Account[], id: string | null): Account | null {
  if (!id) return null;
  return accounts.find((a) => a.id === id) ?? null;
}

export interface SyncProgress {
  accountId: string;
  status: string;
  current: number;
  total: number;
  message: string;
}

/**
 * Pure reducer for sync-progress events.
 *
 * `syncingAccountIds` is the single source of truth for who is syncing: a
 * non-terminal event adds its account, a terminal one (`complete`/`error`)
 * removes it, and `isSyncing` is simply "the set is non-empty".
 *
 * It is deliberately keyed by account. A single global flag conflated every
 * account's progress, so one account working through a months-long backfill
 * made the whole app look busy — which starved a newly added account of its
 * first sync and painted a spinner over its empty inbox. See
 * `selectIsSyncing` for reading it back with the right scope.
 */
export function reduceSyncProgress(
  state: Pick<AccountStore, 'error' | 'errorAccountId' | 'syncingAccountIds'>,
  progress: SyncProgress | null,
): Pick<AccountStore, 'syncProgress' | 'isSyncing' | 'error' | 'errorAccountId' | 'syncingAccountIds'> {
  if (!progress) {
    return {
      syncProgress: null,
      isSyncing: false,
      error: state.error,
      errorAccountId: state.errorAccountId,
      syncingAccountIds: new Set<string>(),
    };
  }

  const isTerminal = progress.status === 'complete' || progress.status === 'error';
  const isError = progress.status === 'error';

  const syncing = new Set(state.syncingAccountIds);
  if (isTerminal) {
    syncing.delete(progress.accountId);
  } else {
    syncing.add(progress.accountId);
  }

  return {
    syncProgress: progress,
    isSyncing: syncing.size > 0,
    error: isError ? progress.message : state.error,
    // Tag the error with the account it came from so the UI can decide
    // whether to show the banner (only when this account is active).
    errorAccountId: isError ? progress.accountId : state.errorAccountId,
    syncingAccountIds: syncing,
  };
}

/**
 * Is the mailbox scope the user is looking at syncing?
 *
 * `scopeId` is an `activeAccountId`: a concrete account asks only about
 * itself, while "All accounts" (and a not-yet-resolved `null`) asks about any.
 * Surfaces that describe one account — the sidebar refresh button, the inbox
 * empty state — must use this rather than a global flag, so one account's
 * backfill never disables another account's controls, nor claims it is syncing
 * when nothing was ever enqueued for it.
 */
export function selectIsSyncing(syncingAccountIds: Set<string>, scopeId: string | null): boolean {
  if (scopeId === null || isUnifiedMode(scopeId)) return syncingAccountIds.size > 0;
  return syncingAccountIds.has(scopeId);
}

/** `syncingAccountIds` with `accountId` added — a copy, never a mutation. */
function withAccount(syncingAccountIds: Set<string>, accountId: string): Set<string> {
  return new Set(syncingAccountIds).add(accountId);
}

/**
 * Stop tracking `accountId` and record why, leaving every other account's sync
 * untouched. Used when *enqueuing* fails: no sync-progress event will ever
 * arrive for it, so nothing else would clear it from the set.
 */
function dropAccount(
  state: Pick<AccountStore, 'syncingAccountIds'>,
  accountId: string,
  error: string,
): Pick<AccountStore, 'syncingAccountIds' | 'isSyncing' | 'error' | 'errorAccountId'> {
  const syncing = new Set(state.syncingAccountIds);
  syncing.delete(accountId);
  return { syncingAccountIds: syncing, isSyncing: syncing.size > 0, error, errorAccountId: accountId };
}

interface AccountStore {
  accounts: Account[];
  activeAccountId: string | null;
  isLoading: boolean;
  isSyncing: boolean;
  syncProgress: SyncProgress | null;
  error: string | null;
  /// Account id the current `error` belongs to, when the error came from a
  /// sync-progress event. `null` for non-account-scoped errors (e.g.
  /// `fetchAccounts` failures). The UI uses this to decide whether to show
  /// the error banner — sync errors only display while their account is the
  /// active one, so a background auto-sync failure on Account B doesn't
  /// surface the banner while Account A is selected.
  errorAccountId: string | null;
  // Track the current sync operation to prevent race conditions
  currentSyncId: number;
  /// Account whose initial setup dialog (sync window picker) is still open.
  /// While this matches activeAccountId, the auto-sync effect in App.tsx
  /// must skip starting a sync — otherwise sync runs with sync_from_timestamp
  /// = null before the user can choose a window. Cleared when the dialog
  /// closes (saved or dismissed).
  setupPendingAccountId: string | null;
  /// Bumped whenever an account's settings are saved. The inbox's category
  /// chips key their refetch off this, because the synced-category list is
  /// read once when an account becomes active — which, during onboarding, is
  /// before the user has chosen anything. It lives here rather than in App's
  /// local state because the onboarding wizard renders its own copy of the
  /// settings dialog: a counter only App could bump left onboarding's save
  /// invisible, and the strip stuck on Primary for the whole session.
  accountSettingsVersion: number;
  bumpAccountSettingsVersion: () => void;
  /// Accounts with a sync enqueued or in flight — the source of truth behind
  /// `isSyncing`. Populated optimistically when a sync is enqueued and drained
  /// by terminal progress events; see `reduceSyncProgress` and
  /// `selectIsSyncing`.
  syncingAccountIds: Set<string>;
  setActiveAccount: (id: string | null) => void;
  fetchAccounts: () => Promise<void>;
  addAccount: (
    provider: 'gmail' | 'outlook',
    syncFromTimestamp?: number | null,
    options?: { deferSetup?: boolean },
  ) => Promise<Account>;
  registerImapAccount: (account: Account, options?: { deferSetup?: boolean }) => void;
  removeAccount: (accountId: string) => Promise<void>;
  reauthenticateAccount: (accountId: string) => Promise<void>;
  syncAccount: (accountId: string) => Promise<void>;
  /// Enqueue a sync for every given account (unified "All accounts" mode).
  /// The backend runs per-account queues, so syncs proceed independently;
  /// completion is tracked per account via sync-progress events.
  syncAllAccounts: (accountIds: string[]) => Promise<void>;
  setSyncProgress: (progress: SyncProgress | null) => void;
  moveAccountUp: (accountId: string) => Promise<void>;
  moveAccountDown: (accountId: string) => Promise<void>;
  setAccountEnabled: (accountId: string, enabled: boolean) => Promise<void>;
  updateAccountSyncFrom: (accountId: string, syncFromTimestamp?: number | null) => Promise<Account>;
  markSetupPending: (accountId: string) => void;
  clearSetupPending: (accountId: string) => void;
  clearError: () => void;
}

export const useAccountStore = create<AccountStore>((set, get) => ({
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

  setActiveAccount: (id) => set({ activeAccountId: id, error: null, errorAccountId: null }),

  accountSettingsVersion: 0,
  bumpAccountSettingsVersion: () => set((state) => ({ accountSettingsVersion: state.accountSettingsVersion + 1 })),

  markSetupPending: (accountId) => set({ setupPendingAccountId: accountId }),
  clearSetupPending: (accountId) =>
    set((state) => (state.setupPendingAccountId === accountId ? { setupPendingAccountId: null } : {})),

  setSyncProgress: (progress) => set((state) => reduceSyncProgress(state, progress)),

  clearError: () => set({ error: null, errorAccountId: null }),

  fetchAccounts: async () => {
    set({ isLoading: true, error: null, errorAccountId: null });
    try {
      const accounts = await api.listAccounts();
      set({ accounts, isLoading: false });
      if (accounts.length > 0 && !get().activeAccountId) {
        set({ activeAccountId: accounts[0].id });
      }
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null, isLoading: false });
    }
  },

  addAccount: async (provider, syncFromTimestamp, options) => {
    set({ isLoading: true, error: null, errorAccountId: null });
    try {
      const account = await api.addAccount(provider, syncFromTimestamp);
      // When the caller (onboarding) is about to open the sync-window dialog,
      // mark setup as pending in the same atomic update so the auto-sync
      // effect in App.tsx — which fires on activeAccountId change — sees the
      // pending flag and skips. Without this, sync would race ahead with
      // sync_from_timestamp = null before the user picks a window.
      set((state) => ({
        accounts: [...state.accounts, account],
        activeAccountId: account.id,
        setupPendingAccountId: options?.deferSetup ? account.id : state.setupPendingAccountId,
        isLoading: false,
      }));
      return account;
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null, isLoading: false });
      throw error;
    }
  },

  registerImapAccount: (account, options) => {
    set((state) => ({
      accounts: [...state.accounts, account],
      activeAccountId: account.id,
      setupPendingAccountId: options?.deferSetup ? account.id : state.setupPendingAccountId,
    }));
  },

  removeAccount: async (accountId) => {
    set({ isLoading: true, error: null, errorAccountId: null });
    try {
      await api.removeAccount(accountId);
      set((state) => ({
        accounts: state.accounts.filter((a) => a.id !== accountId),
        activeAccountId: (() => {
          if (state.activeAccountId !== accountId) {
            return state.activeAccountId;
          }
          const remainingAccounts = state.accounts.filter((a) => a.id !== accountId);
          return remainingAccounts[0]?.id ?? null;
        })(),
        isLoading: false,
      }));
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null, isLoading: false });
      throw error;
    }
  },

  reauthenticateAccount: async (accountId) => {
    set({ isLoading: true, error: null, errorAccountId: null });
    try {
      await api.reauthenticateAccount(accountId);
      set({ isLoading: false });
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null, isLoading: false });
      throw error;
    }
  },

  syncAccount: async (accountId) => {
    // De-duplicate per account, never globally. The account's own queue is
    // FIFO with concurrency 1, so a second request would not run alongside the
    // first — it would run *after* it, repeating the whole backfill. Scoping
    // the check to `accountId` is what keeps this from becoming the starvation
    // bug it replaced: one account's multi-hour backfill used to hold a single
    // global flag true, so a newly added account's only sync attempt returned
    // without invoking anything and nothing ever retried it.
    if (get().syncingAccountIds.has(accountId)) return;

    const syncId = get().currentSyncId + 1;
    set((state) => ({
      syncingAccountIds: withAccount(state.syncingAccountIds, accountId),
      isSyncing: true,
      error: null,
      errorAccountId: null,
      syncProgress: null,
      currentSyncId: syncId,
    }));

    try {
      await api.syncAccount(accountId);
    } catch (error) {
      // Only update state if this is still the current sync operation
      if (get().currentSyncId === syncId) {
        // Scope this manual-sync error to the account that initiated it so
        // the banner is account-aware (consistent with sync-progress events).
        set((state) => dropAccount(state, accountId, errorText(error)));
      }
      throw error;
    }
  },

  syncAllAccounts: async (accountIds) => {
    // Same per-account rule as `syncAccount`: skip the ones already in flight,
    // enqueue the rest. A batch that bailed out wholesale because *something*
    // was syncing is what left newly added accounts empty.
    const toSync = accountIds.filter((id) => !get().syncingAccountIds.has(id));
    if (toSync.length === 0) return;

    const syncId = get().currentSyncId + 1;
    set((state) => ({
      isSyncing: true,
      error: null,
      errorAccountId: null,
      syncProgress: null,
      currentSyncId: syncId,
      syncingAccountIds: toSync.reduce(withAccount, state.syncingAccountIds),
    }));

    for (const accountId of toSync) {
      try {
        // Enqueue-only: the backend command submits to the account's own sync
        // queue and returns; completion arrives via sync-progress events.
        await api.syncAccount(accountId);
      } catch (error) {
        if (get().currentSyncId !== syncId) return;
        // Enqueue failed for this account — stop tracking it so the batch can
        // still finish, and surface the error scoped to the account.
        set((state) => dropAccount(state, accountId, errorText(error)));
      }
    }
  },

  moveAccountUp: async (accountId) => {
    const { accounts, fetchAccounts } = get();
    const idx = accounts.findIndex((a) => a.id === accountId);
    if (idx <= 0) return;
    const newOrder = [...accounts];
    [newOrder[idx - 1], newOrder[idx]] = [newOrder[idx], newOrder[idx - 1]];
    const ids = newOrder.map((a) => a.id);
    try {
      await api.reorderAccounts(ids);
      await fetchAccounts();
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null });
    }
  },

  moveAccountDown: async (accountId) => {
    const { accounts, fetchAccounts } = get();
    const idx = accounts.findIndex((a) => a.id === accountId);
    if (idx < 0 || idx >= accounts.length - 1) return;
    const newOrder = [...accounts];
    [newOrder[idx], newOrder[idx + 1]] = [newOrder[idx + 1], newOrder[idx]];
    const ids = newOrder.map((a) => a.id);
    try {
      await api.reorderAccounts(ids);
      await fetchAccounts();
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null });
    }
  },

  setAccountEnabled: async (accountId, enabled) => {
    try {
      await api.setAccountEnabled(accountId, enabled);
      set((state) => ({
        accounts: state.accounts.map((a) => (a.id === accountId ? { ...a, enabled } : a)),
      }));
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null });
    }
  },

  updateAccountSyncFrom: async (accountId, syncFromTimestamp) => {
    try {
      const updatedAccount = await api.updateAccountSyncFrom(accountId, syncFromTimestamp);
      set((state) => ({
        accounts: state.accounts.map((account) => (account.id === accountId ? updatedAccount : account)),
      }));
      return updatedAccount;
    } catch (error) {
      set({ error: errorText(error), errorAccountId: null });
      throw error;
    }
  },
}));
