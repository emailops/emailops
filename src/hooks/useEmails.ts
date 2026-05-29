import { useCallback, useEffect } from 'react';
import type { MailboxView } from '@/lib/api';
import { useAccountStore } from '@/stores/accountStore';
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

  useEffect(() => {
    if (activeAccountId) {
      fetchEmails(activeAccountId, activeFilter, selectedCategories, false, mailbox);
    }
  }, [activeAccountId, activeFilter, searchQuery, selectedCategoriesKey, fetchEmails, selectedCategories, mailbox]);

  const loadMore = useCallback(() => {
    if (activeAccountId) {
      loadMoreEmails(activeAccountId, activeFilter, selectedCategories, mailbox);
    }
  }, [activeAccountId, activeFilter, loadMoreEmails, selectedCategories, mailbox]);

  const refetch = useCallback(() => {
    if (!activeAccountId) {
      return;
    }
    fetchEmails(activeAccountId, activeFilter, selectedCategories, false, mailbox);
  }, [activeAccountId, activeFilter, fetchEmails, selectedCategories, mailbox]);

  /** Refresh emails in the background without showing a loading state or clearing the list.
   *  Use this after background syncs so the UI update is transparent to the user. */
  const silentRefetch = useCallback(() => {
    if (!activeAccountId) {
      return;
    }
    fetchEmails(activeAccountId, activeFilter, selectedCategories, true, mailbox);
  }, [activeAccountId, activeFilter, fetchEmails, selectedCategories, mailbox]);

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
