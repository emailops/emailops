import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AddImapAccountModal } from '@/components/AddImapAccountModal';
import { AttachmentList } from '@/components/Attachments/AttachmentList';
import { AttachmentToolbar } from '@/components/Attachments/AttachmentToolbar';
import { AttachmentViewer } from '@/components/Attachments/AttachmentViewer';
import type { RuleFormPrefill } from '@/components/Attachments/RuleManagementModal';
import { RuleManagementModal } from '@/components/Attachments/RuleManagementModal';
import { ChatView } from '@/components/Chat/ChatView';
import { ComposeModal } from '@/components/ComposeModal';
import { ContactsView } from '@/components/Contacts/ContactsView';
import { Dashboard } from '@/components/Dashboard/Dashboard';
import { DraftsView } from '@/components/DraftsView';
import { AttachmentTabView } from '@/components/EmailView/AttachmentTabView';
import { ComposeTabView } from '@/components/EmailView/ComposeTabView';
import { EmailTabBar } from '@/components/EmailView/EmailTabBar';
import { EmailView } from '@/components/EmailView/EmailView';
import { ErrorBanner } from '@/components/ErrorBanner/ErrorBanner';
import type { RulePrefill } from '@/components/Inbox/EmailRow';
import { Inbox } from '@/components/Inbox/Inbox';
import { LensesView } from '@/components/Lenses/LensesView';
import { LockScreen } from '@/components/LockScreen';
import { LogPanel } from '@/components/LogPanel/LogPanel';
import { MemoryView } from '@/components/Memory/MemoryView';
import { OfflineBanner } from '@/components/OfflineBanner';
import { OnboardingWizard } from '@/components/Onboarding/OnboardingWizard';
import { SearchBar } from '@/components/Search/SearchBar';
import type { ClassificationRulePrefill } from '@/components/Settings/ClassificationSettings';
import { SettingsDialog, type SettingsTab } from '@/components/Settings/SettingsDialog';
import { AccountSettingsDialog } from '@/components/Sidebar/AccountSettingsDialog';
import { AddAccountModal } from '@/components/Sidebar/AddAccountModal';
import type { ViewMode } from '@/components/Sidebar/Sidebar';
import { Sidebar } from '@/components/Sidebar/Sidebar';
import { TasksPanel } from '@/components/Tasks/TasksPanel';
import { useAccounts } from '@/hooks/useAccounts';
import { useAttachments } from '@/hooks/useAttachments';
import { useEmails } from '@/hooks/useEmails';
import { usePersistedPref } from '@/hooks/usePersistedPref';
import { useSmartFilters } from '@/hooks/useSmartFilters';
import * as api from '@/lib/api';
import { type ChatToolEffectPayload, handleChatToolEffect } from '@/lib/chatToolEffects';
import { plainTextToHtml } from '@/lib/composeHtml';
import { errorText } from '@/lib/errors';
import { planViewChange } from '@/lib/viewNavigation';
import { useAccountStore } from '@/stores/accountStore';
import { useAiStore } from '@/stores/aiStore';
import { useChatStore } from '@/stores/chatStore';
import { useConnectivityStore } from '@/stores/connectivityStore';
import { useEmailStore } from '@/stores/emailStore';
import { useLensesEnabledStore, useMemoryEnabledStore, useTasksEnabledStore } from '@/stores/featureToggleStore';
import { useLensStore } from '@/stores/lensStore';
import type { LogLevel, LogSource } from '@/stores/logStore';
import { useLogStore } from '@/stores/logStore';
import { useMemoryStore } from '@/stores/memoryStore';
import { useTagStore } from '@/stores/tagStore';
import type {
  ActiveFilter,
  ChatPhaseEvent,
  ChatRenamedEvent,
  ChatSourcesEvent,
  ChatStreamEvent,
  ChatTraceEvent,
  Email,
  EmailCategory,
  InboxLayout,
} from '@/types';

const LOG_LEVELS: LogLevel[] = ['info', 'warn', 'error', 'debug', 'success'];
const LOG_SOURCES: LogSource[] = [
  'sync',
  'ai',
  'search',
  'account',
  'system',
  'embeddings',
  'attachments',
  'chat',
  'lens',
];
const DEFAULT_CATEGORIES: EmailCategory[] = ['primary', 'social', 'updates'];
const VALID_CATEGORIES = new Set<EmailCategory>(['primary', 'social', 'updates', 'forums', 'promotions']);

function viewModeToMailbox(mode: ViewMode): 'inbox' | 'sent' | 'spam' | 'deleted' {
  if (mode === 'sent' || mode === 'spam' || mode === 'deleted') return mode;
  return 'inbox';
}

function isLogLevel(value: string): value is LogLevel {
  return LOG_LEVELS.includes(value as LogLevel);
}

function isLogSource(value: string): value is LogSource {
  return LOG_SOURCES.includes(value as LogSource);
}

function App() {
  const [isLocked, setIsLocked] = useState(false);
  const [lockChecked, setLockChecked] = useState(false);

  useEffect(() => {
    api.hasMainPassword().then((has) => {
      setIsLocked(has);
      setLockChecked(true);
    });
  }, []);

  if (!lockChecked) return null;
  if (isLocked) return <LockScreen onUnlock={() => setIsLocked(false)} />;

  return <AppInner />;
}

function AppInner() {
  const { t } = useTranslation(['common', 'modal']);
  const { enabled: aiEnabled, refresh: refreshAi } = useAiStore();
  // Onboarding: shown when the `onboarding_completed` preference is missing.
  // `null` = still loading the preference; we render nothing AI-conditional
  // until we know, otherwise the empty inbox flashes behind the wizard.
  const [onboardingCompleted, setOnboardingCompleted] = useState<boolean | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('inbox');
  const [inboxLayout, setInboxLayout] = usePersistedPref<InboxLayout>('inbox_layout', 'split', {
    parse: (raw) => (raw === 'split' || raw === 'full-width' ? raw : null),
    serialize: (v) => v,
  });
  // The Memory and Tasks experimental flags ARE the master switches for the
  // backend extraction pipelines — they share the same `memory_enabled` /
  // `task_enabled` SQLite preference rows that `MemoryConfig.enabled` and
  // `TaskConfig.enabled` read on the Rust side. Toggling here both hides the
  // sidebar entry and stops extraction; previously these were two separate
  // keys, which let the pipeline keep running after the sidebar was hidden.
  const {
    enabled: tasksEnabled,
    setEnabled: setTasksEnabledRaw,
    refresh: refreshTasksEnabled,
  } = useTasksEnabledStore();
  const {
    enabled: memoriesEnabled,
    setEnabled: setMemoriesEnabledRaw,
    refresh: refreshMemoriesEnabled,
  } = useMemoryEnabledStore();
  const {
    enabled: lensesEnabled,
    setEnabled: setLensesEnabledRaw,
    refresh: refreshLensesEnabled,
  } = useLensesEnabledStore();
  const setTasksEnabled = useCallback(
    (v: boolean) => {
      setTasksEnabledRaw(v).catch((err) => console.error('Failed to persist task_enabled', err));
    },
    [setTasksEnabledRaw],
  );
  const setMemoriesEnabled = useCallback(
    (v: boolean) => {
      setMemoriesEnabledRaw(v).catch((err) => console.error('Failed to persist memory_enabled', err));
    },
    [setMemoriesEnabledRaw],
  );
  const setLensesEnabled = useCallback(
    (v: boolean) => {
      setLensesEnabledRaw(v).catch((err) => console.error('Failed to persist lenses_enabled', err));
    },
    [setLensesEnabledRaw],
  );
  const [isInboxCollapsed, setIsInboxCollapsed] = useState(false);
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isComposeOpen, setIsComposeOpen] = useState(false);
  const [composePrefillTo, setComposePrefillTo] = useState<string[] | undefined>(undefined);
  const [isAddAccountOpen, setIsAddAccountOpen] = useState(false);
  const [isAddAccountPickerOpen, setIsAddAccountPickerOpen] = useState(false);
  const [isAddImapAccountOpen, setIsAddImapAccountOpen] = useState(false);
  const [accountSettingsAccountId, setAccountSettingsAccountId] = useState<string | null>(null);
  const [isRuleModalOpen, setIsRuleModalOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab | null>(null);
  const [classificationRulePrefill, setClassificationRulePrefill] = useState<ClassificationRulePrefill | null>(null);
  const [rulePrefill, setRulePrefill] = useState<RuleFormPrefill | null>(null);
  const [selectedCategories, setSelectedCategories] = usePersistedPref<Set<EmailCategory>>(
    'inbox_categories',
    new Set(DEFAULT_CATEGORIES),
    {
      parse: (raw) => {
        const parsed = JSON.parse(raw);
        if (!Array.isArray(parsed)) return null;
        const valid = parsed.filter(
          (v): v is EmailCategory => typeof v === 'string' && VALID_CATEGORIES.has(v as EmailCategory),
        );
        return valid.length > 0 ? new Set(valid) : null;
      },
      serialize: (s) => JSON.stringify(Array.from(s)),
    },
  );
  const selectedCategoriesList = useMemo(() => Array.from(selectedCategories), [selectedCategories]);

  // If a user disables an experimental feature (or the master AI switch) while
  // currently viewing it, redirect to inbox. Chat / memory / tasks all require AI.
  useEffect(() => {
    if (viewMode === 'tasks' && (!tasksEnabled || !aiEnabled)) setViewMode('inbox');
    else if (viewMode === 'memory' && (!memoriesEnabled || !aiEnabled)) setViewMode('inbox');
    else if (viewMode === 'lenses' && (!lensesEnabled || !aiEnabled)) setViewMode('inbox');
    else if (viewMode === 'chat' && !aiEnabled) setViewMode('inbox');
  }, [viewMode, tasksEnabled, memoriesEnabled, lensesEnabled, aiEnabled]);
  const addLog = useLogStore((s) => s.addLog);
  const clearSearchQuery = useEmailStore((s) => s.clearSearchQuery);
  const tabs = useEmailStore((s) => s.tabs);
  const activeTabId = useEmailStore((s) => s.activeTabId);
  const openTab = useEmailStore((s) => s.openTab);
  const closeTab = useEmailStore((s) => s.closeTab);
  const setActiveTab = useEmailStore((s) => s.setActiveTab);
  const openComposeTab = useEmailStore((s) => s.openComposeTab);
  const setPendingChatDraft = useEmailStore((s) => s.setPendingChatDraft);
  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const previousSyncStatusRef = useRef<string | null>(null);

  const {
    accounts,
    activeAccount,
    activeAccountId,
    setActiveAccount,
    addAccount,
    registerImapAccount,
    removeAccount,
    syncAccount,
    reauthenticateAccount,
    moveAccountUp,
    moveAccountDown,
    setAccountEnabled,
    isSyncing,
    syncProgress,
    isLoading: accountsLoading,
    error: accountError,
    errorAccountId: accountErrorAccountId,
    clearError: clearAccountError,
    refetch: fetchAccounts,
  } = useAccounts();

  // Inbox category tabs for the active account. Drives the filter chips in
  // <Inbox/> so users only see what they've opted into (Gmail) or what the
  // provider exposes (Outlook). Bumped by `accountSettingsVersion` to refetch
  // after the user saves new settings.
  //
  // `null` means "not yet known" — initial mount before the API resolves, or
  // no active account. Inbox renders no chips while this is null, avoiding a
  // flash of the hard-coded default trio.
  const [availableCategories, setAvailableCategories] = useState<EmailCategory[] | null>(null);
  const [accountSettingsVersion, setAccountSettingsVersion] = useState(0);
  useEffect(() => {
    if (!activeAccountId) {
      setAvailableCategories(null);
      return;
    }
    let cancelled = false;
    api
      .getAvailableCategories(activeAccountId)
      .then((cats) => {
        if (cancelled) return;
        const valid = cats.filter((c): c is EmailCategory => VALID_CATEGORIES.has(c as EmailCategory));
        // Empty list (IMAP, unknown provider) → null so showCategoryFilter
        // hides the chip strip entirely. Otherwise pass the provider-aware
        // set straight through.
        setAvailableCategories(valid.length > 0 ? valid : null);
      })
      .catch(() => {
        if (cancelled) return;
        // Fallback to the conservative defaults rather than nothing — keeps
        // the inbox usable if the backend command transiently fails.
        setAvailableCategories([...DEFAULT_CATEGORIES]);
      });
    return () => {
      cancelled = true;
    };
  }, [activeAccountId, accountSettingsVersion]);
  const {
    emails,
    isLoading: emailsLoading,
    isLoadingMore,
    isLoadingThread,
    hasMore,
    totalCount,
    selectedEmail,
    threadEmails,
    selectEmail,
    loadMore,
    refetch: refetchEmails,
    silentRefetch: silentRefetchEmails,
    error: emailError,
    clearError: clearEmailError,
    reset: resetEmails,
  } = useEmails(selectedCategoriesList, viewModeToMailbox(viewMode));

  // Keep stable refs so the sync effects always call the latest version.
  const refetchEmailsRef = useRef(refetchEmails);
  refetchEmailsRef.current = refetchEmails;
  const silentRefetchEmailsRef = useRef(silentRefetchEmails);
  silentRefetchEmailsRef.current = silentRefetchEmails;

  const {
    displayedFilters: smartFilters,
    activeFilter,
    isLoadingStats: isLoadingFilters,
    toggleFilter,
    clearActiveFilter,
    pinFilter: handlePinFilter,
    unpinFilter: handleUnpinFilter,
    removeFilter: handleRemoveFilter,
    forceRefresh: forceRefreshFilters,
    addSenderAsFilter,
    blockSender: handleBlockSender,
    isPinned: isFilterPinned,
  } = useSmartFilters();

  const {
    rules: attachmentRules,
    attachments,
    selectedAttachment,
    isLoading: isLoadingAttachments,
    isLoadingMore: isLoadingMoreAttachments,
    hasMore: hasMoreAttachments,
    totalCount: attachmentTotalCount,
    selectedTag,
    availableTags,
    checkedIds,
    createRule: createAttachmentRule,
    updateRule: updateAttachmentRule,
    deleteRule: deleteAttachmentRule,
    loadMore: loadMoreAttachments,
    selectAttachment,
    toggleChecked,
    toggleCheckAll,
    clearChecked,
    setSelectedTag,
    refreshAfterRuleApply,
  } = useAttachments();

  const ruleNames = useMemo(() => {
    const map: Record<string, string> = {};
    for (const rule of attachmentRules) {
      map[rule.id] = rule.name;
    }
    return map;
  }, [attachmentRules]);

  const handleChangeLayout = setInboxLayout;

  // Show the window once the React tree is mounted (it starts hidden to avoid the
  // transparent-blank flash while the WebView initialises).
  useEffect(() => {
    api.showMainWindow().catch(console.error);
  }, []);

  // Load the master AI enable/disable flag from the backend once on boot. Until
  // this resolves the store reports `enabled: true` (matching backend default
  // for upgrading users), so AI surfaces don't flicker off on cold start.
  useEffect(() => {
    refreshAi().catch((err) => console.error('Failed to load ai_enabled pref', err));
    refreshMemoriesEnabled().catch((err) => console.error('Failed to load memory_enabled pref', err));
    refreshTasksEnabled().catch((err) => console.error('Failed to load task_enabled pref', err));
    refreshLensesEnabled().catch((err) => console.error('Failed to load lenses_enabled pref', err));
  }, [refreshAi, refreshMemoriesEnabled, refreshTasksEnabled, refreshLensesEnabled]);

  // Decide whether to show the onboarding wizard. Existing users (anyone with
  // accounts already connected) are auto-marked complete on this boot — they
  // shouldn't see the wizard after an upgrade. Truly new installs (no pref +
  // no accounts) get the wizard.
  //
  // This must only run once per launch: if it re-fires after the wizard has
  // started and the user has just connected their first account in step 3,
  // it would see `accounts.length > 0` and auto-mark onboarding complete,
  // unmounting the wizard mid-flow (and taking AccountSettingsDialog with it).
  // Gate on `onboardingCompleted === null` so subsequent account adds during
  // the wizard don't re-trigger the auto-decide path.
  useEffect(() => {
    if (accountsLoading) return;
    if (onboardingCompleted !== null) return;
    let cancelled = false;
    void (async () => {
      try {
        const raw = await api.getPref('onboarding_completed');
        if (cancelled) return;
        if (raw === 'true') {
          setOnboardingCompleted(true);
          return;
        }
        // No pref yet. If the user already has accounts, they're an upgrading
        // user — silently mark complete and skip the wizard.
        if (accounts.length > 0) {
          await api.setPref('onboarding_completed', 'true');
          if (!cancelled) setOnboardingCompleted(true);
          return;
        }
        if (!cancelled) setOnboardingCompleted(false);
      } catch (err) {
        console.error('Failed to load onboarding pref', err);
        if (!cancelled) setOnboardingCompleted(true); // fail open — never trap the user behind the wizard
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accountsLoading, accounts.length, onboardingCompleted]);

  const handleOnboardingComplete = useCallback(async () => {
    try {
      await api.setPref('onboarding_completed', 'true');
    } catch (err) {
      console.error('Failed to persist onboarding_completed', err);
    }
    setOnboardingCompleted(true);
  }, []);

  // Log on app startup
  useEffect(() => {
    addLog('info', 'system', 'EmailOps started');
  }, [addLog]);

  // Fix macOS WKWebView black screen after system sleep/wake.
  // The webview sometimes doesn't redraw itself; dispatching a resize event
  // forces a layout recalculation which triggers a repaint.
  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        window.dispatchEvent(new Event('resize'));
      }
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => document.removeEventListener('visibilitychange', handleVisibilityChange);
  }, []);

  // Log when accounts finish loading
  useEffect(() => {
    if (!accountsLoading && accounts.length > 0) {
      addLog('info', 'account', `Loaded ${accounts.length} account${accounts.length > 1 ? 's' : ''}`);
    }
    // Only run once when accounts first load
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountsLoading]);

  // Combine errors for display.
  //
  // Account-store errors are scoped: when `errorAccountId` is non-null the
  // error came from a sync-progress event (or a manual sync) for that
  // specific account. Suppress the banner unless the failing account is the
  // one currently selected — otherwise an auto-sync auth failure on
  // Account B would surface the "re-authenticate" banner while the user is
  // looking at Account A. Errors with `errorAccountId === null` are global
  // (fetch/add/remove account, etc.) and always display.
  const isScopedAccountError = accountErrorAccountId !== null && accountErrorAccountId !== activeAccountId;
  const visibleAccountError = isScopedAccountError ? null : accountError;
  const displayError = visibleAccountError || emailError;
  const clearError = useCallback(() => {
    clearAccountError();
    clearEmailError();
  }, [clearAccountError, clearEmailError]);

  const handleReauthenticate = useCallback(async () => {
    // Re-auth the account the error actually belongs to (if known), not
    // whichever account happens to be active when the user clicks "Sign in
    // again". For non-scoped errors fall back to the active account.
    const targetAccountId = accountErrorAccountId ?? activeAccountId;
    if (!targetAccountId) return;

    // IMAP accounts don't have OAuth — credentials live in the keychain and
    // typically need a fresh password (e.g. provider rotated the app
    // password). Open the account settings dialog where the user can update
    // and re-test their credentials. The dialog's Save handler already runs
    // a sync afterwards, mirroring the OAuth path below.
    const targetAccount = accounts.find((a) => a.id === targetAccountId);
    if (targetAccount?.provider === 'imap') {
      addLog('info', 'account', 'Update IMAP credentials to re-authenticate.');
      clearError();
      setAccountSettingsAccountId(targetAccountId);
      return;
    }

    try {
      addLog('info', 'account', 'Re-authenticating account...');
      await reauthenticateAccount(targetAccountId);
      addLog('success', 'account', 'Re-authentication successful');
      clearError();
      await syncAccount(targetAccountId);
    } catch (error) {
      addLog('error', 'account', `Re-authentication failed: ${error}`);
      console.error('Re-authentication failed:', error);
    }
  }, [accountErrorAccountId, activeAccountId, accounts, reauthenticateAccount, clearError, syncAccount, addLog]);

  // Keep the memory store (tasks + badge counts) in sync with the active
  // account. Runs once per account switch — individual refreshes after user
  // actions are driven by the store itself.
  useEffect(() => {
    if (activeAccountId) {
      void useMemoryStore.getState().loadForAccount(activeAccountId);
    } else {
      useMemoryStore.getState().reset();
    }
  }, [activeAccountId]);

  // Keyboard shortcut to open search (Cmd/Ctrl + K)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        setIsSearchOpen(true);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  // Listen for backend events and convert to log entries
  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      listen<{ status: string; current: number; total: number; message: string }>('embedding-progress', (event) => {
        const { status, message } = event.payload;
        if (status === 'starting' || status === 'clearing') {
          addLog('info', 'embeddings', message);
        } else if (status === 'complete') {
          addLog('success', 'embeddings', message);
        } else if (status === 'error') {
          addLog('error', 'embeddings', message);
        } else if (status === 'generating' && event.payload.current % 10 === 0) {
          addLog('debug', 'embeddings', message);
        }
      }),
    );

    unlisteners.push(
      listen<{ level: string; source: string; message: string }>('app-log', (event) => {
        const { level, source, message } = event.payload;
        addLog(isLogLevel(level) ? level : 'info', isLogSource(source) ? source : 'system', message);
      }),
    );

    unlisteners.push(
      listen<{
        provider: string;
        model: string;
        operation: string;
        promptTokens: number;
        completionTokens: number;
        costUsd: number;
        status: string;
        timestamp: number;
      }>('ai_log', (event) => {
        const { provider, model, operation, promptTokens, completionTokens, costUsd, status } = event.payload;
        const costStr = costUsd > 0 ? ` ($${costUsd.toFixed(4)})` : '';
        const tokens = promptTokens + completionTokens;
        addLog(
          status === 'ok' ? 'success' : 'error',
          'ai',
          `${provider}/${model} → ${operation}: ${tokens} tokens${costStr}`,
        );
      }),
    );

    // Listen for email classification events (real-time tag updates)
    unlisteners.push(
      listen<{ emailId: string; tags: { priority: string; intent: string; topic: string; confidence: number | null } }>(
        'email-classified',
        (event) => {
          const { emailId, tags } = event.payload;
          const now = Math.floor(Date.now() / 1000);
          useTagStore.getState().setEmailTags(emailId, [
            { emailId, tagType: 'priority', tagValue: tags.priority, confidence: tags.confidence, createdAt: now },
            { emailId, tagType: 'intent', tagValue: tags.intent, confidence: tags.confidence, createdAt: now },
            { emailId, tagType: 'topic', tagValue: tags.topic, confidence: tags.confidence, createdAt: now },
          ]);
        },
      ),
    );

    // Chat streaming — tokens and source citations from the backend chat service.
    unlisteners.push(
      listen<ChatStreamEvent>('chat-stream', (event) => {
        useChatStore.getState().handleStreamToken(event.payload);
      }),
    );
    unlisteners.push(
      listen<ChatPhaseEvent>('chat-phase', (event) => {
        useChatStore.getState().handlePhase(event.payload);
      }),
    );
    unlisteners.push(
      listen<ChatSourcesEvent>('chat-sources', (event) => {
        useChatStore.getState().handleSources(event.payload);
      }),
    );
    unlisteners.push(
      listen<ChatTraceEvent>('chat-trace', (event) => {
        useChatStore.getState().handleTrace(event.payload);
      }),
    );
    unlisteners.push(
      listen<ChatRenamedEvent>('chat-renamed', (event) => {
        useChatStore.getState().handleRenamed(event.payload);
      }),
    );

    // Chat tools can emit side-effects after a successful tool call (e.g.
    // `generate_email_draft` asks us to open the composer with the saved
    // draft). One channel, tagged-enum payload — the pure dispatcher in
    // `chatToolEffects.ts` switches on `kind`. Adding a new effect = add a
    // case in that helper + a variant on `ToolEffect` in
    // `src-tauri/src/services/chat/tools/mod.rs`. No per-effect plumbing.
    unlisteners.push(
      listen<ChatToolEffectPayload>('chat-tool-effect', (event) => {
        handleChatToolEffect(event.payload, {
          openComposeTab,
          // Reply path — seed the pending draft so EmailView can prepend
          // the body onto the freshly-built quoted template, then drive
          // navigation to the matching thread the same way the email-list
          // click handler does. `getState()` reads the latest action ref
          // rather than capturing a closure over the not-yet-declared
          // useEmailStore hook above the listener mount.
          openThreadReply: (accountId, emailId, body) => {
            setPendingChatDraft({ emailId, body });
            useEmailStore.getState().navigateToEmail(accountId, emailId);
          },
          // Switch away from the chat view so the compose tab the chat just
          // appended actually becomes visible. Without this the tab is
          // created but the user keeps seeing the chat panel and the draft
          // looks like a silent no-op.
          navigateToInbox: () => setViewMode('inbox'),
          log: addLog,
        });
      }),
    );

    return () => {
      unlisteners.forEach((p) => {
        void p.then((fn) => fn());
      });
    };
  }, [addLog]);

  // Sync emails when active account changes
  useEffect(() => {
    if (!activeAccountId || accountsLoading) return;

    const account = accounts.find((a) => a.id === activeAccountId);
    if (!account) return;

    // Track if this effect was cleaned up (account changed)
    const abortController = { cancelled: false };

    // Reset emails when switching accounts to avoid showing stale data
    resetEmails();
    // Also clear any chat state held from the previous account.
    useChatStore.getState().reset();

    // Skip sync for disabled accounts — still load cached emails
    if (!account.enabled) {
      refetchEmailsRef.current();
      return;
    }

    // Skip the auto-sync while the onboarding setup dialog is still open for
    // this account: the user hasn't picked a sync window yet, so syncing now
    // would run with sync_from_timestamp = null. The dialog's save handler
    // calls syncAccount() explicitly after persisting the chosen window.
    if (useAccountStore.getState().setupPendingAccountId === activeAccountId) {
      refetchEmailsRef.current();
      return;
    }

    // Skip the auto-sync when the OfflineBanner is already telling the user
    // why nothing's updating — a real sync attempt would only fail with an
    // HTTP error and spam the output panel for no actionable reason. The
    // backend's poll loops are similarly gated; both resume on reconnect.
    if (!useConnectivityStore.getState().isOnline) {
      refetchEmailsRef.current();
      addLog('info', 'sync', 'Sync skipped — currently offline.');
      return;
    }

    addLog('info', 'sync', `Syncing ${account.email}...`);

    syncAccount(activeAccountId).catch((err) => {
      // Only log if this sync wasn't cancelled. Suppress when offline: the
      // banner already communicates the reason, and `err` here is just the
      // generic transport failure.
      if (!abortController.cancelled && useConnectivityStore.getState().isOnline) {
        addLog('error', 'sync', `Sync failed: ${err}`);
        console.error('Sync failed:', err);
      }
    });

    // Cleanup: mark this sync as cancelled if account changes
    return () => {
      abortController.cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeAccountId, accountsLoading]);

  useEffect(() => {
    const status = syncProgress?.status ?? null;
    const previousStatus = previousSyncStatusRef.current;
    previousSyncStatusRef.current = status;

    if (!syncProgress || syncProgress.accountId !== activeAccountId) {
      return;
    }

    if (status === 'batch') {
      // New emails were just written to DB — refresh the list in-place so they
      // appear while the rest of the sync is still running.
      silentRefetchEmailsRef.current();
    }

    if (status === 'complete' && previousStatus !== 'complete') {
      // Use silent refresh so the list updates in-place without a loading spinner
      // or scroll-position reset — the sync should be transparent to the user.
      silentRefetchEmailsRef.current();
      forceRefreshFilters();
    }
  }, [activeAccountId, forceRefreshFilters, syncProgress]);

  // Email list views — these all render the inbox/sent/spam/deleted email list.
  // When applying a search from any other view, fall back to 'inbox'.
  const isEmailListView = (mode: ViewMode): boolean =>
    mode === 'inbox' || mode === 'sent' || mode === 'spam' || mode === 'deleted';

  const handleSearchSelect = useCallback(
    (email: Parameters<typeof selectEmail>[0]) => {
      if (!email) return;
      setViewMode((prev) => (isEmailListView(prev) ? prev : 'inbox'));
      setActiveTab(null);
      selectEmail(email);
    },
    [selectEmail, setActiveTab],
  );

  const handleApplySearch = useCallback(
    (query: string) => {
      setViewMode((prev) => (isEmailListView(prev) ? prev : 'inbox'));
      clearActiveFilter();
      useEmailStore.getState().setSearchQuery(query);
      addLog('info', 'search', `Filtering inbox: "${query}"`);
    },
    [addLog, clearActiveFilter],
  );

  const handleApplySearchWithResults = useCallback(
    (query: string, emails: Email[]) => {
      setViewMode((prev) => (isEmailListView(prev) ? prev : 'inbox'));
      clearActiveFilter();
      useEmailStore.getState().applySearchResults(query, emails);
      addLog('info', 'search', `Filtering inbox: "${query}" (reused ${emails.length} results)`);
    },
    [addLog, clearActiveFilter],
  );

  const handleToggleSmartFilter = useCallback(
    (filter: ActiveFilter) => {
      setViewMode('inbox');

      const isSelectingNewFilter =
        !activeFilter || activeFilter.type !== filter.type || activeFilter.value !== filter.value;

      if (isSelectingNewFilter) {
        clearSearchQuery();
        // A smart filter (sender/contact, domain, company, tag…) can match
        // emails in any category, so default to All categories. Otherwise the
        // category tab stays on e.g. "Primary" and hides the contact's emails
        // that live in Updates/Social/etc.
        setSelectedCategories(new Set(VALID_CATEGORIES));
      }

      // Close any open email / tab so the filtered list becomes visible.
      // Without this the EmailView keeps rendering on top of the list when
      // the user clicks a sidebar filter (Bug: filter changes but open email stays).
      setActiveTab(null);
      void selectEmail(null);

      toggleFilter(filter);
    },
    [activeFilter, clearSearchQuery, selectEmail, setActiveTab, setSelectedCategories, toggleFilter],
  );

  const navigateToEmail = useEmailStore((s) => s.navigateToEmail);
  const handleViewAttachmentEmail = useCallback(
    async (emailId: string) => {
      if (!activeAccountId) return;
      clearActiveFilter();
      clearSearchQuery();
      setViewMode('inbox');
      navigateToEmail(activeAccountId, emailId);
    },
    [activeAccountId, clearActiveFilter, clearSearchQuery, navigateToEmail],
  );

  const handleCreateAttachmentRule = useCallback((prefill: RulePrefill) => {
    // Build a sensible rule name from the sender
    const name = prefill.senderName || prefill.senderEmail.split('@')[0];
    setRulePrefill({
      name: `${name} attachments`,
      senderEmailPattern: prefill.senderEmail,
      subjectPattern: prefill.subject ? `*${prefill.subject}*` : '',
    });
    setIsRuleModalOpen(true);
  }, []);

  const handleChatAboutThread = useCallback(
    async (email: Email) => {
      if (!aiEnabled) {
        addLog('error', 'ai', 'Enable AI to chat about a thread');
        return;
      }
      try {
        addLog('info', 'ai', `Preparing chat for thread "${email.subject || '(no subject)'}"`);
        await useChatStore.getState().createConversationFromThread(email.accountId, email.threadId);
        setViewMode('chat');
      } catch (e) {
        addLog('error', 'ai', `Failed to start thread chat: ${errorText(e)}`);
      }
    },
    [aiEnabled, addLog],
  );

  const handleSelectCategories = useCallback(
    (categories: Set<EmailCategory>) => {
      setSelectedCategories(new Set(categories));
    },
    [setSelectedCategories],
  );

  const [pendingOAuthProvider, setPendingOAuthProvider] = useState<'gmail' | 'outlook'>('gmail');
  const [addAccountError, setAddAccountError] = useState<string | null>(null);

  const handleAddAccount = async (syncFromTimestamp: number | null) => {
    const provider = pendingOAuthProvider;
    const providerLabel = provider === 'outlook' ? 'Outlook' : 'Gmail';
    setAddAccountError(null);
    try {
      addLog('info', 'account', `Adding ${providerLabel} account...`);
      const account = await addAccount(provider, syncFromTimestamp);
      addLog('success', 'account', `Account added: ${account.email}`);
      setIsAddAccountOpen(false);
    } catch (error) {
      const msg = errorText(error);
      addLog('error', 'account', `Failed to add account: ${msg}`);
      console.error('Failed to add account:', error);
      setAddAccountError(msg);
      // Keep the modal open so the user sees the error; they can retry or cancel.
      throw error;
    }
  };

  const handleImapAccountAdded = (account: import('@/types').Account) => {
    registerImapAccount(account);
    addLog('success', 'account', `IMAP account added: ${account.email}`);
    setIsAddImapAccountOpen(false);
  };

  const handleSync = async () => {
    if (activeAccountId) {
      // Skip the network round-trip when we know we're offline — the request
      // would fail and the user would see a misleading "Sync failed: ..."
      // error in the log instead of the offline banner.
      if (!useConnectivityStore.getState().isOnline) {
        addLog('info', 'sync', 'Sync skipped — currently offline.');
        return;
      }
      const accountEmail = accounts.find((a) => a.id === activeAccountId)?.email;
      addLog('info', 'sync', `Manual sync started for ${accountEmail ?? 'account'}`);
      try {
        await syncAccount(activeAccountId);
      } catch (error) {
        addLog('error', 'sync', `Sync failed: ${error}`);
        console.error('Sync failed:', error);
      }
    }
  };

  return (
    <div className="flex flex-col h-screen bg-gray-50">
      {onboardingCompleted === false && (
        <OnboardingWizard
          currentLayout={inboxLayout}
          onChangeLayout={handleChangeLayout}
          onComplete={handleOnboardingComplete}
        />
      )}
      <OfflineBanner />
      <ErrorBanner message={displayError} onDismiss={clearError} onReauthenticate={handleReauthenticate} />

      <div className="flex flex-1 overflow-hidden">
        <Sidebar
          accounts={accounts}
          activeAccount={activeAccount}
          onSelectAccount={(id) => {
            setViewMode('inbox');
            clearSearchQuery();
            clearActiveFilter();
            setSelectedCategories(new Set<EmailCategory>(['primary']));
            setActiveAccount(id);
          }}
          onAddAccount={() => setIsAddAccountPickerOpen(true)}
          onMoveAccountUp={moveAccountUp}
          onMoveAccountDown={moveAccountDown}
          onToggleAccountEnabled={setAccountEnabled}
          onSync={handleSync}
          onCompose={() => setIsComposeOpen(true)}
          onOpenSearch={() => setIsSearchOpen(true)}
          onOpenAccountSettings={(id) => setAccountSettingsAccountId(id)}
          onOpenAppSettings={() => setSettingsTab('appearance')}
          isSyncing={isSyncing}
          viewMode={viewMode}
          onSetViewMode={(mode) => {
            const plan = planViewChange(mode, inboxLayout);
            if (plan.resetInboxFilters) {
              clearSearchQuery();
              clearActiveFilter();
              setSelectedCategories(new Set<EmailCategory>(['primary']));
            }
            if (plan.closeOpenEmail) {
              setActiveTab(null);
              void selectEmail(null);
            }
            setViewMode(mode);
          }}
          smartFilters={smartFilters}
          activeFilter={activeFilter}
          isLoadingFilters={isLoadingFilters}
          onToggleFilter={handleToggleSmartFilter}
          onClearFilter={clearActiveFilter}
          onPinFilter={handlePinFilter}
          onUnpinFilter={handleUnpinFilter}
          onRemoveFilter={handleRemoveFilter}
          onRefreshFilters={forceRefreshFilters}
          isFilterPinned={isFilterPinned}
          tasksEnabled={tasksEnabled}
          memoriesEnabled={memoriesEnabled}
          lensesEnabled={lensesEnabled}
          onSelectLens={(lensId) => {
            // Selecting a lens in the sidebar: fire the store action so the
            // Lenses view picks it up (it already subscribes to activeLensId),
            // then switch the view mode.
            void useLensStore.getState().selectLens(lensId);
            setViewMode('lenses');
          }}
        />

        <main className="flex flex-1 overflow-hidden">
          {viewMode === 'contacts' ? (
            <ContactsView
              accountId={activeAccountId}
              onComposeTo={(address) => {
                setComposePrefillTo([address]);
                setIsComposeOpen(true);
              }}
              onViewEmailsFrom={(address) => {
                setViewMode('inbox');
                clearSearchQuery();
                handleToggleSmartFilter({ type: 'sender', value: address });
              }}
            />
          ) : viewMode === 'drafts' ? (
            <DraftsView
              accountId={activeAccountId}
              accounts={accounts}
              onOpenComposeTab={(draft) =>
                openComposeTab(draft.accountId, draft.toAddresses, draft.subject, plainTextToHtml(draft.body))
              }
            />
          ) : viewMode === 'chat' ? (
            <ChatView accountId={activeAccountId} onNavigateToInbox={() => setViewMode('inbox')} />
          ) : viewMode === 'tasks' && tasksEnabled ? (
            <TasksPanel accountId={activeAccountId} />
          ) : viewMode === 'memory' && memoriesEnabled ? (
            <MemoryView accountId={activeAccountId} />
          ) : viewMode === 'lenses' && lensesEnabled ? (
            <LensesView />
          ) : viewMode === 'dashboard' ? (
            <Dashboard accounts={accounts} onOpenAccountSettings={(id) => setAccountSettingsAccountId(id)} />
          ) : viewMode === 'inbox' || viewMode === 'sent' || viewMode === 'spam' || viewMode === 'deleted' ? (
            // biome-ignore lint/complexity/noUselessFragments: IIFE result needs a fragment wrapper for the ternary
            <>
              {(() => {
                // When a tab is active, show its content; otherwise show the main selected email.
                const displayThreadEmails = activeTab?.type === 'thread' ? activeTab.threadEmails : threadEmails;
                const displayIsLoading = activeTab?.type === 'thread' ? activeTab.isLoading : isLoadingThread;
                // In full-width mode, "close/back" always returns to the inbox list by
                // clearing both the active tab and the selected email.
                const displayOnClose =
                  inboxLayout === 'full-width'
                    ? () => {
                        if (activeTab) closeTab(activeTab.id);
                        void selectEmail(null);
                      }
                    : activeTab
                      ? () => closeTab(activeTab.id)
                      : () => selectEmail(null);
                // "Open in tab" only makes sense when viewing the main (non-tab) email.
                const handleOpenInTab = !activeTab && selectedEmail ? () => openTab(selectedEmail) : undefined;
                const hasEmailToShow = activeTab !== null || selectedEmail !== null;

                const handleInboxSelect = (email: Email) => {
                  setActiveTab(null);
                  selectEmail(email);
                };

                const emailPane = (
                  <div className="flex flex-col flex-1 overflow-hidden">
                    {tabs.length > 0 && (
                      <EmailTabBar
                        mainEmail={selectedEmail}
                        isMainTabActive={activeTabId === null}
                        tabs={tabs}
                        activeTabId={activeTabId}
                        onSelectMainTab={() => setActiveTab(null)}
                        onSelectTab={setActiveTab}
                        onCloseTab={closeTab}
                      />
                    )}
                    {activeTab?.type === 'attachment' ? (
                      <AttachmentTabView
                        tab={activeTab}
                        onClose={
                          inboxLayout === 'full-width'
                            ? () => {
                                closeTab(activeTab.id);
                                void selectEmail(null);
                              }
                            : () => closeTab(activeTab.id)
                        }
                      />
                    ) : activeTab?.type === 'compose' ? (
                      <ComposeTabView tab={activeTab} accounts={accounts} onClose={() => closeTab(activeTab.id)} />
                    ) : (
                      <EmailView
                        threadEmails={displayThreadEmails}
                        isLoading={displayIsLoading}
                        onClose={displayOnClose}
                        accounts={accounts}
                        activeAccountId={activeAccountId}
                        fullWidth={inboxLayout === 'full-width'}
                        onOpenInTab={handleOpenInTab}
                      />
                    )}
                  </div>
                );

                const inboxList = isInboxCollapsed ? (
                  <button
                    onClick={() => setIsInboxCollapsed(false)}
                    title={t('modal:inbox.expand')}
                    className="w-8 flex-shrink-0 border-r border-gray-200 bg-white flex items-start justify-center pt-4 text-gray-400 hover:text-gray-600 hover:bg-gray-50 transition-colors"
                  >
                    <svg className="h-4 w-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
                      <path d="M6 3l5 5-5 5" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                  </button>
                ) : (
                  <Inbox
                    emails={emails}
                    isLoading={emailsLoading || accountsLoading}
                    isSyncing={isSyncing}
                    syncProgress={syncProgress}
                    isLoadingMore={isLoadingMore}
                    hasMore={hasMore}
                    totalCount={totalCount}
                    selectedEmailId={inboxLayout === 'split' ? (selectedEmail?.id ?? null) : null}
                    disableAutoSelect={inboxLayout === 'full-width'}
                    fullWidth={inboxLayout === 'full-width'}
                    onSelectEmail={handleInboxSelect}
                    onLoadMore={loadMore}
                    onAddSenderFilter={addSenderAsFilter}
                    onBlockSender={handleBlockSender}
                    onCreateAttachmentRule={handleCreateAttachmentRule}
                    onCreateClassificationRule={(prefill) => {
                      setClassificationRulePrefill(prefill);
                      setSettingsTab('classification');
                    }}
                    selectedCategories={selectedCategories}
                    onSelectCategories={handleSelectCategories}
                    showCategoryFilter={activeAccount?.provider !== 'imap' && viewMode === 'inbox'}
                    availableCategories={availableCategories ?? undefined}
                    onCollapse={inboxLayout === 'split' ? () => setIsInboxCollapsed(true) : undefined}
                    onOpenInTab={openTab}
                    onChatAboutThread={aiEnabled ? handleChatAboutThread : undefined}
                    accountName={activeAccount?.name || activeAccount?.email}
                    accountId={activeAccountId}
                    onSearch={handleApplySearch}
                  />
                );

                if (inboxLayout === 'full-width') {
                  // Keep the inbox mounted (hidden) so scroll position is preserved when going back.
                  return (
                    <>
                      <div className={hasEmailToShow ? 'hidden' : 'contents'}>{inboxList}</div>
                      {hasEmailToShow && emailPane}
                    </>
                  );
                }

                // Split layout: inbox list + email pane side by side
                return (
                  <>
                    {inboxList}
                    {emailPane}
                  </>
                );
              })()}
            </>
          ) : (
            <div className="flex flex-col flex-1">
              <AttachmentToolbar
                accountId={activeAccountId}
                totalCount={attachmentTotalCount}
                selectedTag={selectedTag}
                availableTags={availableTags}
                checkedCount={checkedIds.size}
                allChecked={attachments.length > 0 && checkedIds.size === attachments.length}
                onSetSelectedTag={setSelectedTag}
                onToggleCheckAll={toggleCheckAll}
                onClearChecked={clearChecked}
                checkedIds={checkedIds}
                onOpenRules={() => setIsRuleModalOpen(true)}
              />
              <div className="flex flex-1 overflow-hidden">
                <AttachmentList
                  attachments={attachments}
                  selectedAttachment={selectedAttachment}
                  ruleNames={ruleNames}
                  checkedIds={checkedIds}
                  isLoading={isLoadingAttachments}
                  isLoadingMore={isLoadingMoreAttachments}
                  hasMore={hasMoreAttachments}
                  onSelectAttachment={selectAttachment}
                  onToggleChecked={toggleChecked}
                  onLoadMore={loadMoreAttachments}
                  onOpenRules={() => setIsRuleModalOpen(true)}
                />
                <AttachmentViewer attachment={selectedAttachment} onViewEmail={handleViewAttachmentEmail} />
              </div>
            </div>
          )}
        </main>
      </div>

      <LogPanel onOpenAiSettings={() => setSettingsTab('ai')} />

      {isSearchOpen && (
        <SearchBar
          accountId={activeAccountId}
          onSelectEmail={handleSearchSelect}
          onApplySearch={handleApplySearch}
          onApplySearchWithResults={handleApplySearchWithResults}
          selectedCategories={selectedCategoriesList}
          onClose={() => setIsSearchOpen(false)}
        />
      )}

      {isAddAccountPickerOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-neutral-900 border border-neutral-700 rounded-lg w-80 p-6 shadow-xl">
            <h2 className="text-lg font-semibold text-white mb-4">{t('modal:addAccount.title')}</h2>
            <p className="text-sm text-neutral-400 mb-4">{t('modal:addAccount.subtitle')}</p>
            <div className="space-y-3">
              <button
                className="w-full px-4 py-3 rounded bg-neutral-800 border border-neutral-600 text-white text-sm hover:bg-neutral-700 transition-colors text-left"
                onClick={() => {
                  setPendingOAuthProvider('gmail');
                  setIsAddAccountPickerOpen(false);
                  setIsAddAccountOpen(true);
                }}
              >
                <span className="font-medium">{t('modal:addAccount.gmail')}</span>
                <span className="block text-xs text-neutral-400 mt-0.5">{t('modal:addAccount.gmailHint')}</span>
              </button>
              <button
                className="w-full px-4 py-3 rounded bg-neutral-800 border border-neutral-600 text-white text-sm hover:bg-neutral-700 transition-colors text-left"
                onClick={() => {
                  setPendingOAuthProvider('outlook');
                  setIsAddAccountPickerOpen(false);
                  setIsAddAccountOpen(true);
                }}
              >
                <span className="font-medium">{t('modal:addAccount.outlook')}</span>
                <span className="block text-xs text-neutral-400 mt-0.5">{t('modal:addAccount.outlookHint')}</span>
              </button>
              <button
                className="w-full px-4 py-3 rounded bg-neutral-800 border border-neutral-600 text-white text-sm hover:bg-neutral-700 transition-colors text-left"
                onClick={() => {
                  setIsAddAccountPickerOpen(false);
                  setIsAddImapAccountOpen(true);
                }}
              >
                <span className="font-medium">{t('modal:addAccount.imap')}</span>
                <span className="block text-xs text-neutral-400 mt-0.5">{t('modal:addAccount.imapHint')}</span>
              </button>
            </div>
            <button
              className="mt-4 w-full px-4 py-2 rounded bg-neutral-700 text-neutral-300 text-sm hover:bg-neutral-600 transition-colors"
              onClick={() => setIsAddAccountPickerOpen(false)}
            >
              {t('modal:addAccount.cancel')}
            </button>
          </div>
        </div>
      )}

      {isAddAccountOpen && (
        <AddAccountModal
          isSubmitting={accountsLoading}
          onClose={() => {
            setAddAccountError(null);
            setIsAddAccountOpen(false);
          }}
          onConfirm={handleAddAccount}
          providerLabel={pendingOAuthProvider === 'outlook' ? 'Outlook' : 'Gmail'}
          warningMessage={addAccountError ?? undefined}
        />
      )}

      {isAddImapAccountOpen && (
        <AddImapAccountModal onSuccess={handleImapAccountAdded} onCancel={() => setIsAddImapAccountOpen(false)} />
      )}

      {isRuleModalOpen && activeAccountId && (
        <RuleManagementModal
          rules={attachmentRules}
          accountId={activeAccountId}
          prefill={rulePrefill}
          onClose={() => {
            setIsRuleModalOpen(false);
            setRulePrefill(null);
          }}
          onCreateRule={createAttachmentRule}
          onUpdateRule={updateAttachmentRule}
          onDeleteRule={deleteAttachmentRule}
          onRefreshAfterApply={refreshAfterRuleApply}
        />
      )}

      {accountSettingsAccountId && (
        <AccountSettingsDialog
          account={accounts.find((a) => a.id === accountSettingsAccountId)!}
          onClose={() => setAccountSettingsAccountId(null)}
          onSaved={async () => {
            setAccountSettingsAccountId(null);
            const id = accountSettingsAccountId;
            await fetchAccounts();
            // Trigger a refetch of the active account's synced categories so
            // the inbox filter chips reflect the new selection without a reload.
            setAccountSettingsVersion((v) => v + 1);
            if (id) {
              addLog('info', 'sync', 'Settings saved. Starting sync...');
              try {
                await syncAccount(id);
              } catch (e) {
                addLog('error', 'sync', `Sync failed: ${e}`);
              }
            }
          }}
          onToggleEnabled={async (enabled) => {
            await setAccountEnabled(accountSettingsAccountId!, enabled);
            await fetchAccounts();
          }}
          onDelete={async () => {
            const id = accountSettingsAccountId!;
            addLog('info', 'account', `Deleting account...`);
            await removeAccount(id);
            setAccountSettingsAccountId(null);
            addLog('success', 'account', 'Account deleted.');
          }}
        />
      )}

      {isComposeOpen && activeAccountId && (
        <ComposeModal
          accounts={accounts}
          defaultAccountId={activeAccountId}
          defaultToRecipients={composePrefillTo}
          onClose={() => {
            setIsComposeOpen(false);
            setComposePrefillTo(undefined);
          }}
          onMaximize={(state) => {
            setIsComposeOpen(false);
            setComposePrefillTo(undefined);
            setViewMode('inbox');
            openComposeTab(state.accountId, state.toAddresses, state.subject, state.bodyHtml);
          }}
        />
      )}

      {settingsTab && (
        <SettingsDialog
          initialTab={settingsTab}
          activeAccountId={activeAccountId}
          accounts={accounts}
          currentLayout={inboxLayout}
          onChangeLayout={handleChangeLayout}
          classificationPrefill={classificationRulePrefill}
          tasksEnabled={tasksEnabled}
          onChangeTasksEnabled={setTasksEnabled}
          memoriesEnabled={memoriesEnabled}
          onChangeMemoriesEnabled={setMemoriesEnabled}
          lensesEnabled={lensesEnabled}
          onChangeLensesEnabled={setLensesEnabled}
          onClose={() => {
            setSettingsTab(null);
            setClassificationRulePrefill(null);
          }}
        />
      )}
    </div>
  );
}

export default App;
