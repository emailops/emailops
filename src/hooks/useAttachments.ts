import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect } from 'react';
import * as api from '@/lib/api';
import { selectEffectiveAccountId, useAccountStore } from '@/stores/accountStore';
import { useAttachmentStore } from '@/stores/attachmentStore';
import { useLogStore } from '@/stores/logStore';
import type { Attachment, AttachmentRule } from '@/types';

export function useAttachments() {
  // Attachments stay per-account: in unified ("All accounts") mode we scope
  // to the first enabled account instead of leaking the sentinel id to the
  // backend. `selectEffectiveAccountId` returns the id unchanged otherwise.
  const activeAccountId = useAccountStore((s) => selectEffectiveAccountId(s.accounts, s.activeAccountId));
  const addLog = useLogStore((s) => s.addLog);

  const {
    rules,
    isLoadingRules,
    attachments,
    selectedAttachment,
    isLoading,
    isLoadingMore,
    hasMore,
    totalCount,
    selectedTag,
    availableTags,
    error,
    checkedIds,
    fetchRules,
    createRule,
    updateRule,
    deleteRule,
    fetchAttachments,
    loadMoreAttachments,
    selectAttachment,
    toggleChecked,
    toggleCheckAll,
    clearChecked,
    setSelectedTag,
    fetchTags,
    clearError,
    reset,
  } = useAttachmentStore();

  // Load rules and tags when account changes
  useEffect(() => {
    if (activeAccountId) {
      fetchRules(activeAccountId);
      fetchTags(activeAccountId);
    }
  }, [activeAccountId, fetchRules, fetchTags]);

  // Load attachments when account or selected tag changes
  useEffect(() => {
    if (activeAccountId) {
      fetchAttachments(activeAccountId, selectedTag);
    }
  }, [activeAccountId, selectedTag, fetchAttachments]);

  // Refresh when the backend reports new attachments were saved (during sync
  // or retroactive rule application). Without this, newly-saved attachments
  // only appear after restart.
  useEffect(() => {
    if (!activeAccountId) return;
    const unlisten = listen<string>('attachments-updated', (event) => {
      // Backend sends the account_id as payload — ignore events for other accounts.
      if (event.payload && event.payload !== activeAccountId) return;
      fetchAttachments(activeAccountId, selectedTag);
      fetchTags(activeAccountId);
    });
    return () => {
      void unlisten.then((u) => u());
    };
  }, [activeAccountId, selectedTag, fetchAttachments, fetchTags]);

  const handleCreateRule = useCallback(
    async (
      name: string,
      senderEmailPattern: string | null,
      subjectPattern: string | null,
      filenamePattern: string | null,
      tags: string[],
    ): Promise<AttachmentRule> => {
      if (!activeAccountId) throw new Error('No active account');
      const rule = await createRule(activeAccountId, name, senderEmailPattern, subjectPattern, filenamePattern, tags);
      addLog('success', 'attachments', `Created rule: ${name}`);
      return rule;
    },
    [activeAccountId, createRule, addLog],
  );

  const handleUpdateRule = useCallback(
    async (
      ruleId: string,
      name: string,
      senderEmailPattern: string | null,
      subjectPattern: string | null,
      filenamePattern: string | null,
      tags: string[],
      enabled: boolean,
    ): Promise<AttachmentRule> => {
      const rule = await updateRule(ruleId, name, senderEmailPattern, subjectPattern, filenamePattern, tags, enabled);
      addLog('info', 'attachments', `Updated rule "${name}", re-evaluating...`);
      // Backend cleared old attachments — re-scan existing emails
      if (activeAccountId && enabled) {
        try {
          const count = await api.applyRuleRetroactively(ruleId, activeAccountId);
          addLog('success', 'attachments', `Rule "${name}": found ${count} attachments`);
        } catch (err) {
          addLog('error', 'attachments', `Re-evaluation failed: ${err}`);
        }
      }
      if (activeAccountId) {
        fetchAttachments(activeAccountId, selectedTag);
        fetchTags(activeAccountId);
      }
      return rule;
    },
    [updateRule, addLog, activeAccountId, selectedTag, fetchAttachments, fetchTags],
  );

  const handleDeleteRule = useCallback(
    async (ruleId: string) => {
      if (!activeAccountId) return;
      await deleteRule(ruleId, activeAccountId);
      addLog('success', 'attachments', 'Rule deleted');
      // Refresh attachments and tags since deletions may have occurred
      fetchAttachments(activeAccountId, selectedTag);
      fetchTags(activeAccountId);
    },
    [activeAccountId, deleteRule, fetchAttachments, fetchTags, selectedTag, addLog],
  );

  const handleLoadMore = useCallback(() => {
    if (activeAccountId) {
      loadMoreAttachments(activeAccountId);
    }
  }, [activeAccountId, loadMoreAttachments]);

  const handleSelectAttachment = useCallback(
    (attachment: Attachment | null) => {
      selectAttachment(attachment);
    },
    [selectAttachment],
  );

  const handleSetSelectedTag = useCallback(
    (tag: string | null) => {
      setSelectedTag(tag);
    },
    [setSelectedTag],
  );

  const refreshAfterRuleApply = useCallback(() => {
    if (activeAccountId) {
      fetchAttachments(activeAccountId, selectedTag);
      fetchTags(activeAccountId);
    }
  }, [activeAccountId, selectedTag, fetchAttachments, fetchTags]);

  return {
    rules,
    isLoadingRules,
    attachments,
    selectedAttachment,
    isLoading,
    isLoadingMore,
    hasMore,
    totalCount,
    selectedTag,
    availableTags,
    error,
    checkedIds,
    createRule: handleCreateRule,
    updateRule: handleUpdateRule,
    deleteRule: handleDeleteRule,
    loadMore: handleLoadMore,
    selectAttachment: handleSelectAttachment,
    toggleChecked,
    toggleCheckAll,
    clearChecked,
    setSelectedTag: handleSetSelectedTag,
    clearError,
    reset,
    refreshAfterRuleApply,
  };
}
