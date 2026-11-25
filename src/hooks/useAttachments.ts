import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { errorText } from '@/lib/errors';
import { isUnifiedMode, selectEffectiveAccountId, toQueryAccountId, useAccountStore } from '@/stores/accountStore';
import { useAttachmentStore } from '@/stores/attachmentStore';
import { useLogStore } from '@/stores/logStore';
import type { Attachment, AttachmentRule, AttachmentRuleSuggestion } from '@/types';
import { useRuleApplyEvents } from './useRuleApplyEvents';

/** A sync of several accounts sends one `attachments-updated` per account: reload once they settle. */
const ATTACHMENT_EVENTS_SETTLE_MS = 500;

const NO_RULES: AttachmentRule[] = [];
const NO_SUGGESTIONS: AttachmentRuleSuggestion[] = [];

// Each rule and suggestion is changed in its own account, read from the store
// when the action runs.
function accountOfRule(ruleId: string): string {
  const rule = useAttachmentStore.getState().rules.find((r) => r.id === ruleId);
  if (!rule) throw new Error(`Unknown attachment rule ${ruleId}`);
  return rule.accountId;
}

function accountOfSuggestion(suggestionId: string): string {
  const { suggestions, dismissedSuggestions } = useAttachmentStore.getState();
  const suggestion = [...suggestions, ...dismissedSuggestions].find((s) => s.id === suggestionId);
  if (!suggestion) throw new Error(`Unknown suggested attachment rule ${suggestionId}`);
  return suggestion.accountId;
}

export function useAttachments() {
  // The list reads one account, or every enabled account in unified ("All
  // accounts") mode, where `toQueryAccountId` sends null. Nothing is read
  // until an account is selected and the accounts have loaded, so a null id
  // never means "all" by accident.
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const accounts = useAccountStore((s) => s.accounts);
  const effectiveAccountId = useAccountStore((s) => selectEffectiveAccountId(s.accounts, s.activeAccountId));
  const isUnified = isUnifiedMode(activeAccountId);
  const enabledIds = useMemo(() => accounts.filter((a) => a.enabled).map((a) => a.id), [accounts]);
  const queryAccountId = toQueryAccountId(activeAccountId);
  // Which accounts the list covers: enabling or disabling one starts a new list.
  const scopeKey =
    activeAccountId === null || accounts.length === 0
      ? null
      : isUnified
        ? `all:${enabledIds.join(',')}`
        : `account:${activeAccountId}`;

  // Rules and their suggestions belong to one account. In unified mode the
  // rules modal lists those of every enabled account (ALL_ACCOUNTS_ID) and the
  // badge counts all their suggestions; with one account selected, that
  // account's, or those of the account of the email the modal was opened
  // from. A new rule goes to that email's account, else to the effective one.
  const [ruleAccountFromEmail, setRuleAccountFromEmail] = useState<string | null>(null);
  const rulesAccountId = isUnified ? activeAccountId : (ruleAccountFromEmail ?? effectiveAccountId);
  const newRuleAccountId = ruleAccountFromEmail ?? effectiveAccountId;
  const addLog = useLogStore((s) => s.addLog);

  const {
    rules,
    rulesAccountId: loadedRulesAccountId,
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
    listError,
    isSelectingAll,
    checkedIds,
    fetchRules,
    createRule,
    updateRule,
    deleteRule,
    fetchAttachments,
    loadMoreAttachments,
    selectAttachment,
    toggleChecked,
    toggleSelectAll,
    clearChecked,
    setSelectedTag,
    fetchTags,
    clearError,
    reset,
    suggestions,
    suggestionsAccountId,
    fetchSuggestions,
    refreshSuggestions,
    dismissSuggestion,
    acceptSuggestion,
    suggestionsLoading,
    dismissedSuggestions,
    fetchDismissedSuggestions,
    restoreSuggestion,
  } = useAttachmentStore();

  // Suggestion loads run in the background (account switch, post-sync
  // re-mine, modal open); a failure goes to the output panel.
  const loadSuggestionsLogged = useCallback(
    (accountId: string, load: (accountId: string) => Promise<void>) => {
      load(accountId).catch((err) => {
        addLog('error', 'attachments', `Failed to load suggested attachment rules: ${errorText(err)}`);
      });
    },
    [addLog],
  );

  // Tag chips load alongside the list; a failure goes to the output panel and
  // the list loads anyway.
  const loadTagsLogged = useCallback(
    (accountId: string | null) => {
      fetchTags(accountId).catch((err) => {
        addLog('error', 'attachments', `Failed to load attachment tags: ${errorText(err)}`);
      });
    },
    [fetchTags, addLog],
  );

  // Reload the list and its tag chips. With the same account and tag the list
  // refreshes in place, keeping the selection and the open attachment.
  const reloadList = useCallback(() => {
    if (!scopeKey) return;
    fetchAttachments(queryAccountId, selectedTag, scopeKey);
    loadTagsLogged(queryAccountId);
  }, [queryAccountId, scopeKey, selectedTag, fetchAttachments, loadTagsLogged]);

  // The list shows its own error state; the output panel gets the details.
  useEffect(() => {
    if (listError) addLog('error', 'attachments', `Failed to load attachments: ${listError}`);
  }, [listError, addLog]);

  // Load the rules and rule suggestions of the rules account.
  useEffect(() => {
    if (rulesAccountId) {
      fetchRules(rulesAccountId);
      loadSuggestionsLogged(rulesAccountId, fetchSuggestions);
    }
  }, [rulesAccountId, fetchRules, fetchSuggestions, loadSuggestionsLogged]);

  // The backend re-mines rule suggestions after every sync that brought new
  // mail; reload them so the badge reflects the new candidates.
  useEffect(() => {
    if (!rulesAccountId) return;
    // The payload is the re-mined account.
    const covers = (accountId: string) => (isUnified ? enabledIds.includes(accountId) : accountId === rulesAccountId);
    const unlisten = listen<string>('attachment-rule-suggestions-updated', (event) => {
      if (event.payload && !covers(event.payload)) return;
      loadSuggestionsLogged(rulesAccountId, fetchSuggestions);
    });
    return () => {
      void unlisten.then((u) => u());
    };
  }, [rulesAccountId, isUnified, enabledIds, fetchSuggestions, loadSuggestionsLogged]);

  // Load the tag chips when the scope changes.
  useEffect(() => {
    if (scopeKey) loadTagsLogged(queryAccountId);
  }, [scopeKey, queryAccountId, loadTagsLogged]);

  // Load attachments when the scope or the selected tag changes.
  useEffect(() => {
    if (scopeKey) fetchAttachments(queryAccountId, selectedTag, scopeKey);
  }, [scopeKey, queryAccountId, selectedTag, fetchAttachments]);

  // Refresh when the backend reports new attachments were saved (during sync
  // or retroactive rule application). Without this, newly-saved attachments
  // only appear after restart.
  useEffect(() => {
    if (!scopeKey) return;
    // The payload is the account that saved them; other accounts' events are
    // ignored, and so are those of a disabled account in unified mode.
    const inScope = (accountId: string) => (isUnified ? enabledIds.includes(accountId) : accountId === activeAccountId);
    let settle: ReturnType<typeof setTimeout> | undefined;
    const unlisten = listen<string>('attachments-updated', (event) => {
      if (event.payload && !inScope(event.payload)) return;
      clearTimeout(settle);
      settle = setTimeout(reloadList, ATTACHMENT_EVENTS_SETTLE_MS);
    });
    return () => {
      clearTimeout(settle);
      void unlisten.then((u) => u());
    };
  }, [scopeKey, isUnified, enabledIds, activeAccountId, reloadList]);

  const handleCreateRule = useCallback(
    async (
      accountId: string,
      name: string,
      senderEmailPattern: string | null,
      subjectPattern: string | null,
      filenamePattern: string | null,
      tags: string[],
    ): Promise<AttachmentRule> => {
      if (!rulesAccountId) throw new Error('No active account');
      const rule = await createRule(accountId, name, senderEmailPattern, subjectPattern, filenamePattern, tags);
      addLog('success', 'attachments', `Created rule: ${name}`);
      // A suggestion the new rule covers must leave the list.
      loadSuggestionsLogged(rulesAccountId, refreshSuggestions);
      return rule;
    },
    [rulesAccountId, createRule, addLog, loadSuggestionsLogged, refreshSuggestions],
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
      if (!rulesAccountId) throw new Error('No account is selected');
      const rule = await updateRule(
        accountOfRule(ruleId),
        ruleId,
        name,
        senderEmailPattern,
        subjectPattern,
        filenamePattern,
        tags,
        enabled,
      );
      addLog('info', 'attachments', `Updated rule "${name}"`);
      // The backend dropped attachments the new patterns no longer match; the
      // rules modal re-scans existing mail in the background with progress.
      reloadList();
      // New patterns may now cover — or stop covering — a suggestion.
      loadSuggestionsLogged(rulesAccountId, refreshSuggestions);
      return rule;
    },
    [updateRule, addLog, rulesAccountId, reloadList, loadSuggestionsLogged, refreshSuggestions],
  );

  const handleDeleteRule = useCallback(
    async (ruleId: string) => {
      if (!rulesAccountId) return;
      await deleteRule(ruleId, accountOfRule(ruleId));
      addLog('success', 'attachments', 'Rule deleted');
      // Refresh attachments and tags since deletions may have occurred, and
      // suggestions since the deleted rule's documents may be candidates again.
      reloadList();
      loadSuggestionsLogged(rulesAccountId, refreshSuggestions);
    },
    [rulesAccountId, deleteRule, reloadList, refreshSuggestions, addLog, loadSuggestionsLogged],
  );

  // A failed page leaves the rows already shown; the output panel says why.
  const handleLoadMore = useCallback(() => {
    loadMoreAttachments().catch((err) => {
      addLog('error', 'attachments', `Failed to load more attachments: ${errorText(err)}`);
    });
  }, [loadMoreAttachments, addLog]);

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

  // The rules modal re-mines on open and loads the dismissed list for undo.
  const handleRefreshSuggestions = useCallback(() => {
    if (!rulesAccountId) return;
    loadSuggestionsLogged(rulesAccountId, refreshSuggestions);
    loadSuggestionsLogged(rulesAccountId, fetchDismissedSuggestions);
  }, [rulesAccountId, refreshSuggestions, fetchDismissedSuggestions, loadSuggestionsLogged]);

  const handleRestoreSuggestion = useCallback(
    async (suggestionId: string) => {
      if (!rulesAccountId) return;
      try {
        await restoreSuggestion(accountOfSuggestion(suggestionId), suggestionId);
      } catch (err) {
        addLog('error', 'attachments', `Failed to restore suggested attachment rule: ${errorText(err)}`);
        throw err;
      }
      addLog('success', 'attachments', 'Restored suggested attachment rule');
    },
    [rulesAccountId, restoreSuggestion, addLog],
  );

  // Both rethrow after logging so the rules modal can show the failure inline.
  const handleDismissSuggestion = useCallback(
    async (suggestionId: string) => {
      if (!rulesAccountId) return;
      try {
        await dismissSuggestion(accountOfSuggestion(suggestionId), suggestionId);
      } catch (err) {
        addLog('error', 'attachments', `Failed to dismiss suggested attachment rule: ${errorText(err)}`);
        throw err;
      }
      addLog('success', 'attachments', 'Dismissed suggested attachment rule');
    },
    [rulesAccountId, dismissSuggestion, addLog],
  );

  const handleAcceptSuggestion = useCallback(
    async (suggestionId: string) => {
      if (!rulesAccountId) return;
      try {
        await acceptSuggestion(accountOfSuggestion(suggestionId), suggestionId);
      } catch (err) {
        addLog('error', 'attachments', `Failed to mark suggested attachment rule as accepted: ${errorText(err)}`);
        throw err;
      }
    },
    [rulesAccountId, acceptSuggestion, addLog],
  );

  // Rule scans run in the background and may finish after the rules modal
  // closed; this app-wide listener records their outcome.
  useRuleApplyEvents(reloadList);

  // For one render after `rulesAccountId` changes the store still holds the
  // previous account's rules and suggestions; never hand them to the modal.
  const rulesReady = loadedRulesAccountId === rulesAccountId;

  return {
    rulesAccountId,
    newRuleAccountId,
    setRuleAccountFromEmail,
    rules: rulesReady ? rules : NO_RULES,
    isLoadingRules: isLoadingRules || !rulesReady,
    attachments,
    selectedAttachment,
    isLoading,
    isLoadingMore,
    hasMore,
    totalCount,
    selectedTag,
    availableTags,
    error,
    listError,
    isSelectingAll,
    retry: reloadList,
    checkedIds,
    createRule: handleCreateRule,
    updateRule: handleUpdateRule,
    deleteRule: handleDeleteRule,
    loadMore: handleLoadMore,
    selectAttachment: handleSelectAttachment,
    toggleChecked,
    toggleSelectAll,
    clearChecked,
    setSelectedTag: handleSetSelectedTag,
    clearError,
    reset,
    suggestions: suggestionsAccountId === rulesAccountId ? suggestions : NO_SUGGESTIONS,
    suggestionsLoading,
    dismissedSuggestions,
    restoreSuggestion: handleRestoreSuggestion,
    refreshSuggestions: handleRefreshSuggestions,
    dismissSuggestion: handleDismissSuggestion,
    acceptSuggestion: handleAcceptSuggestion,
  };
}
