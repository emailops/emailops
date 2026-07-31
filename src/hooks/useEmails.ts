import { useCallback, useEffect } from 'react';
import type { MailboxView } from '@/lib/api';
import { toQueryAccountId, useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFilterStore } from '@/stores/filterStore';
import type { EmailCategory } from '@/types';

export function useEmails(selectedCategories: EmailCategory[] = [], mailbox: MailboxView = 'inbox') {
  const {
    emails,
    isLoading,
    isLoadingMore,
    isLoadingThread,
    hasMore,
    totalCount,
    error,
    searchQuery,
    fetchEmails,
    loadMoreEmails,
    selectedEmail,
    threadEmails,
    selectEmail,
    clearError,
    reset,
  } = useEmailStore();
  const { activeAccountId } = useAccountStore();
  const activeFilter = useFilterStore((s) => s.activeFilter);
  const selectedCategoriesKey = searchQuery ? selectedCategories.slice().sort().join(',') : '';
  // The truthy gate below keeps its "no accounts" meaning — the All-accounts
  // sentinel is truthy. Queries receive the translated id (null = unified).
  const queryAccountId = toQueryAccountId(activeAccountId);

  // biome-ignore lint/correctness/useExhaustiveDependencies: searchQuery + selectedCategoriesKey are deliberate refetch triggers (fetchEmails reads the query from the store)
  useEffect(() => {
    if (activeAccountId) {
      fetchEmails(queryAccountId, activeFilter, selectedCategories, false, mailbox);
    } else {
      // No account selected — either startup before accounts load (list is
      // already empty, a no-op) or the last remaining account was just
      // deleted (activeAccountId dropped to null). Without this, the
      // previously active account's emails stayed in the store forever.
      reset();
    }
  }, [
    activeAccountId,
    queryAccountId,
    activeFilter,
    searchQuery,
    selectedCategoriesKey,
    fetchEmails,
    selectedCategories,
    mailbox,
    reset,
  ]);

  const loadMore = useCallback(() => {
    if (activeAccountId) {
      loadMoreEmails(queryAccountId, activeFilter, selectedCategories, mailbox);
    }
  }, [activeAccountId, queryAccountId, activeFilter, loadMoreEmails, selectedCategories, mailbox]);

  const refetch = useCallback(() => {
    if (!activeAccountId) {
      return;
    }
    fetchEmails(queryAccountId, activeFilter, selectedCategories, false, mailbox);
  }, [activeAccountId, queryAccountId, activeFilter, fetchEmails, selectedCategories, mailbox]);

  /** Refresh emails in the background without showing a loading state or clearing the list.
   *  Use this after background syncs so the UI update is transparent to the user. */
  const silentRefetch = useCallback(() => {
    if (!activeAccountId) {
      return;
    }
    fetchEmails(queryAccountId, activeFilter, selectedCategories, true, mailbox);
  }, [activeAccountId, queryAccountId, activeFilter, fetchEmails, selectedCategories, mailbox]);

  return {
    emails,
    isLoading,
    isLoadingMore,
    isLoadingThread,
    hasMore,
    totalCount,
    error,
    selectedEmail,
    threadEmails,
    selectEmail,
    loadMore,
    searchQuery,
    clearError,
    reset,
    refetch,
    silentRefetch,
  };
}
