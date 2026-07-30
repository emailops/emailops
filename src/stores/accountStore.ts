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
 * `pendingSyncAccountIds` tracks the accounts a `syncAllAccounts` batch is
 * still waiting on; a terminal event (`complete`/`error`) removes its account
 * from the set, and `isSyncing` stays true until the set drains. For
 * single-account syncs the set is empty and the behavior is unchanged
 * (`isSyncing` follows the latest event's terminality).
 */
export function reduceSyncProgress(
  state: Pick<AccountStore, 'error' | 'errorAccountId' | 'pendingSyncAccountIds'>,
  progress: SyncProgress | null,
): Pick<AccountStore, 'syncProgress' | 'isSyncing' | 'error' | 'errorAccountId' | 'pendingSyncAccountIds'> {
  if (!progress) {
    return {
      syncProgress: null,
      isSyncing: false,
      error: state.error,
      errorAccountId: state.errorAccountId,
      pendingSyncAccountIds: new Set<string>(),
    };
  }

  const isTerminal = progress.status === 'complete' || progress.status === 'error';
  const isError = progress.status === 'error';

  let pending = state.pendingSyncAccountIds;
  if (isTerminal && pending.has(progress.accountId)) {
    pending = new Set(pending);
    pending.delete(progress.accountId);
  }

  return {
    syncProgress: progress,
    isSyncing: !isTerminal || pending.size > 0,
    error: isError ? progress.message : state.error,
    // Tag the error with the account it came from so the UI can decide
    // whether to show the banner (only when this account is active).
    errorAccountId: isError ? progress.accountId : state.errorAccountId,
    pendingSyncAccountIds: pending,
  };
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
  /// Accounts a `syncAllAccounts` batch is still waiting on. See
  /// `reduceSyncProgress` — terminal progress events drain this set and
  /// `isSyncing` stays true until it empties.
  pendingSyncAccountIds: Set<string>;
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
  pendingSyncAccountIds: new Set<string>(),

  setActiveAccount: (id) => set({ activeAccountId: id, error: null, errorAccountId: null }),

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
    // Only one sync at a time — ignore if already running
    if (get().isSyncing) return;

    // Increment sync ID to track this operation and cancel stale ones
    const syncId = get().currentSyncId + 1;
    set({ isSyncing: true, error: null, errorAccountId: null, syncProgress: null, currentSyncId: syncId });

    try {
      await api.syncAccount(accountId);
    } catch (error) {
      // Only update state if this is still the current sync operation
      if (get().currentSyncId === syncId) {
        // Scope this manual-sync error to the account that initiated it so
        // the banner is account-aware (consistent with sync-progress events).
        set({ error: errorText(error), errorAccountId: accountId, isSyncing: false, syncProgress: null });
      }
      throw error;
    }
  },

  syncAllAccounts: async (accountIds) => {
    // Only one sync batch at a time — same rule as single-account syncs.
    if (get().isSyncing || accountIds.length === 0) return;

    const syncId = get().currentSyncId + 1;
    set({
      isSyncing: true,
      error: null,
      errorAccountId: null,
      syncProgress: null,
      currentSyncId: syncId,
      pendingSyncAccountIds: new Set(accountIds),
    });

    for (const accountId of accountIds) {
      try {
        // Enqueue-only: the backend command submits to the account's own sync
        // queue and returns; completion arrives via sync-progress events.
        await api.syncAccount(accountId);
      } catch (error) {
        if (get().currentSyncId !== syncId) return;
        // Enqueue failed for this account — drop it from pending so the batch
        // can still finish, and surface the error scoped to the account.
        set((state) => {
          const pending = new Set(state.pendingSyncAccountIds);
          pending.delete(accountId);
          return {
            pendingSyncAccountIds: pending,
            isSyncing: pending.size > 0,
            error: errorText(error),
            errorAccountId: accountId,
          };
        });
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
