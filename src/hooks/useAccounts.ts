import { listen } from '@tauri-apps/api/event';
import { useEffect } from 'react';
import { type SyncProgress, useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';

export function useAccounts() {
  const {
    accounts,
    activeAccountId,
    isLoading,
    isSyncing,
    syncProgress,
    error,
    errorAccountId,
    setActiveAccount,
    fetchAccounts,
    addAccount,
    registerImapAccount,
    removeAccount,
    reauthenticateAccount,
    syncAccount,
    setSyncProgress,
    moveAccountUp,
    moveAccountDown,
    setAccountEnabled,
    updateAccountSyncFrom,
    clearError,
  } = useAccountStore();
  const addLog = useLogStore((s) => s.addLog);

  useEffect(() => {
    fetchAccounts();
  }, [fetchAccounts]);

  // Listen for sync progress events from the backend.
  //
  // Mirror progress events into the output panel, prefixing messages with
  // `[account@email]` so users with multiple accounts can tell which sync
  // each line belongs to. Account email is resolved from the current
  // accounts list at emit time.
  useEffect(() => {
    const unlisten = listen<SyncProgress>('sync-progress', (event) => {
      setSyncProgress(event.payload);

      const { accountId, status, message } = event.payload;
      const accountEmail = useAccountStore.getState().accounts.find((a) => a.id === accountId)?.email;
      const prefixed = accountEmail ? `[${accountEmail}] ${message}` : message;
      // `complete` is intentionally not logged: an idle sync should stay quiet
      // (no "Inbox up to date" line), and a real sync's "Synced N new emails"
      // completion is already logged by the backend. Logging it here too would
      // duplicate that line in the output panel.
      if (status === 'error') {
        addLog('error', 'sync', prefixed);
      } else if (status === 'fetching') {
        addLog('info', 'sync', prefixed);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setSyncProgress, addLog]);

  return {
    accounts,
    activeAccountId,
    activeAccount: accounts.find((a) => a.id === activeAccountId) ?? null,
    isLoading,
    isSyncing,
    syncProgress,
    error,
    errorAccountId,
    setActiveAccount,
    addAccount,
    registerImapAccount,
    removeAccount,
    reauthenticateAccount,
    syncAccount,
    moveAccountUp,
    moveAccountDown,
    setAccountEnabled,
    updateAccountSyncFrom,
    clearError,
    refetch: fetchAccounts,
  };
}
