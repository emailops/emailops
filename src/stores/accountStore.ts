import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Account } from '@/types';

export interface SyncProgress {
  accountId: string;
  status: string;
  current: number;
  total: number;
  message: string;
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

  setActiveAccount: (id) => set({ activeAccountId: id, error: null, errorAccountId: null }),

  markSetupPending: (accountId) => set({ setupPendingAccountId: accountId }),
  clearSetupPending: (accountId) =>
    set((state) => (state.setupPendingAccountId === accountId ? { setupPendingAccountId: null } : {})),

  setSyncProgress: (progress) =>
    set((state) => {
      if (!progress) {
        return { syncProgress: null, isSyncing: false };
      }

      const isTerminal = progress.status === 'complete' || progress.status === 'error';
      const isError = progress.status === 'error';

      return {
        syncProgress: progress,
        isSyncing: !isTerminal,
        error: isError ? progress.message : state.error,
        // Tag the error with the account it came from so the UI can decide
        // whether to show the banner (only when this account is active).
        errorAccountId: isError ? progress.accountId : state.errorAccountId,
      };
    }),

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
