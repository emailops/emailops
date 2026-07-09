import { useCallback, useEffect, useMemo } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useAccountStore } from '@/stores/accountStore';
import { filterMatchKey, useFilterStore } from '@/stores/filterStore';
import { useLogStore } from '@/stores/logStore';
import type { ActiveFilter } from '@/types';

export function useSmartFilters() {
  const {
    activeFilter,
    isLoadingStats,
    suggestions,
    loadSaved,
    fetchPrefs,
    forceRefresh,
    toggleFilter,
    clearActiveFilter,
    pinFilter,
    unpinFilter,
    removeFilter,
    addSenderAsFilter,
    restoreFilter,
    getDisplayedFilters,
    prefs,
    reset,
  } = useFilterStore(
    useShallow((state) => ({
      activeFilter: state.activeFilter,
      isLoadingStats: state.isLoadingStats,
      suggestions: state.suggestions,
      loadSaved: state.loadSaved,
      fetchPrefs: state.fetchPrefs,
      forceRefresh: state.forceRefresh,
      toggleFilter: state.toggleFilter,
      clearActiveFilter: state.clearActiveFilter,
      pinFilter: state.pinFilter,
      unpinFilter: state.unpinFilter,
      removeFilter: state.removeFilter,
      addSenderAsFilter: state.addSenderAsFilter,
      restoreFilter: state.restoreFilter,
      getDisplayedFilters: state.getDisplayedFilters,
      prefs: state.prefs,
      reset: state.reset,
    })),
  );
  const { activeAccountId, accounts } = useAccountStore();
  const addLog = useLogStore((s) => s.addLog);

  const formatAccountLog = useCallback(
    (message: string) => {
      const email = accounts.find((a) => a.id === activeAccountId)?.email;
      return email ? `[${email}] ${message}` : message;
    },
    [accounts, activeAccountId],
  );

  // Load saved suggestions + prefs on account change
  useEffect(() => {
    if (activeAccountId) {
      clearActiveFilter();
      loadSaved(activeAccountId).catch((error) => {
        addLog('error', 'system', `Failed to load smart filters: ${error}`);
      });
      fetchPrefs(activeAccountId).catch((error) => {
        addLog('error', 'system', `Failed to load filter preferences: ${error}`);
      });
    } else {
      reset();
    }
  }, [activeAccountId, loadSaved, fetchPrefs, clearActiveFilter, reset, addLog]);

  const displayedFilters = useMemo(
    () => getDisplayedFilters(),
    // prefs and suggestions are the reactive data; getDisplayedFilters is a stable ref
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [prefs, suggestions],
  );

  const handlePin = useCallback(
    (filter: ActiveFilter) => {
      if (activeAccountId) {
        addLog('info', 'system', `Pinned filter: ${filter.value}`);
        pinFilter(activeAccountId, filter).catch((error) => {
          addLog('error', 'system', `Failed to pin filter: ${error}`);
        });
      }
    },
    [activeAccountId, pinFilter, addLog],
  );

  const handleUnpin = useCallback(
    (filter: ActiveFilter) => {
      if (activeAccountId) {
        addLog('info', 'system', `Unpinned filter: ${filter.value}`);
        unpinFilter(activeAccountId, filter).catch((error) => {
          addLog('error', 'system', `Failed to unpin filter: ${error}`);
        });
      }
    },
    [activeAccountId, unpinFilter, addLog],
  );

  const handleRemove = useCallback(
    (filter: ActiveFilter) => {
      if (activeAccountId) {
        addLog('info', 'system', `Removed filter: ${filter.value}`);
        removeFilter(activeAccountId, filter).catch((error) => {
          addLog('error', 'system', `Failed to remove filter: ${error}`);
        });
      }
    },
    [activeAccountId, removeFilter, addLog],
  );

  const handleRestore = useCallback(
    (filter: ActiveFilter) => {
      if (activeAccountId) {
        restoreFilter(activeAccountId, filter).catch((error) => {
          addLog('error', 'system', `Failed to restore filter: ${error}`);
        });
      }
    },
    [activeAccountId, restoreFilter, addLog],
  );

  const handleAddSenderAsFilter = useCallback(
    (senderEmail: string) => {
      if (activeAccountId) {
        addLog('info', 'system', formatAccountLog(`Adding sender filter: ${senderEmail}`));
        addSenderAsFilter(activeAccountId, senderEmail).catch((error) => {
          addLog('error', 'system', formatAccountLog(`Failed to add sender filter: ${error}`));
        });
      }
    },
    [activeAccountId, addSenderAsFilter, addLog, formatAccountLog],
  );

  const handleBlockSender = useCallback(
    (senderEmail: string) => {
      if (activeAccountId) {
        addLog('info', 'system', formatAccountLog(`Blocking sender: ${senderEmail}`));
        removeFilter(activeAccountId, { type: 'sender', value: senderEmail }).catch((error) => {
          addLog('error', 'system', formatAccountLog(`Failed to block sender: ${error}`));
        });
      }
    },
    [activeAccountId, removeFilter, addLog, formatAccountLog],
  );

  const handleForceRefresh = useCallback(async () => {
    if (activeAccountId) {
      addLog('info', 'system', formatAccountLog('Recalculating smart filters...'));
      try {
        await forceRefresh(activeAccountId);
        const count = useFilterStore.getState().suggestions.length;
        addLog('success', 'system', formatAccountLog(`Smart filters updated (${count} suggestions)`));
      } catch (error) {
        addLog('error', 'system', formatAccountLog(`Failed to refresh filters: ${error}`));
      }
    }
  }, [activeAccountId, forceRefresh, addLog, formatAccountLog]);

  const isPinned = useCallback(
    (filter: ActiveFilter) => {
      const key = filterMatchKey(filter.type, filter.value);
      return prefs.some((p) => p.status === 'pinned' && filterMatchKey(p.filterType, p.filterValue) === key);
    },
    [prefs],
  );

  return {
    displayedFilters,
    activeFilter,
    isLoadingStats,
    toggleFilter,
    clearActiveFilter,
    pinFilter: handlePin,
    unpinFilter: handleUnpin,
    removeFilter: handleRemove,
    restoreFilter: handleRestore,
    addSenderAsFilter: handleAddSenderAsFilter,
    blockSender: handleBlockSender,
    forceRefresh: handleForceRefresh,
    isPinned,
  };
}
