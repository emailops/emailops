import { listen } from '@tauri-apps/api/event';
import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AddImapAccountModal } from '@/components/AddImapAccountModal';
import { AttachmentList } from '@/components/Attachments/AttachmentList';
import { AttachmentToolbar } from '@/components/Attachments/AttachmentToolbar';
import { AttachmentViewer } from '@/components/Attachments/AttachmentViewer';
import type { RuleFormPrefill } from '@/components/Attachments/RuleManagementModal';
import { RuleManagementModal } from '@/components/Attachments/RuleManagementModal';
import { CalendarView } from '@/components/Calendar/CalendarView';
import { MeetingReminderBanner } from '@/components/Calendar/MeetingReminderBanner';
import { ChatPanelDock } from '@/components/Chat/ChatPanelDock';
import { ChatView } from '@/components/Chat/ChatView';
import { ComposeModal } from '@/components/ComposeModal';
import { ContactsView } from '@/components/Contacts/ContactsView';
import { ToastHost } from '@/components/common/ToastHost';
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
import { LogView } from '@/components/LogPanel/LogView';
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
import { UnifiedScopeBar } from '@/components/shared/UnifiedScopeBar';
import { TasksPanel } from '@/components/Tasks/TasksPanel';
import { useAccounts } from '@/hooks/useAccounts';
import { useAttachments } from '@/hooks/useAttachments';
import { useEmails } from '@/hooks/useEmails';
import { usePersistedPref } from '@/hooks/usePersistedPref';
import { useResponsiveLayout } from '@/hooks/useResponsiveLayout';
import { useSmartFilters } from '@/hooks/useSmartFilters';
import { useSwipeNavigation } from '@/hooks/useSwipeNavigation';
import { useTheme } from '@/hooks/useTheme';
import { i18n } from '@/i18n';
import type { MailboxView } from '@/lib/api';
import * as api from '@/lib/api';
import { handleUpdateAvailable, type UpdateAvailablePayload } from '@/lib/appUpdate';
import { deriveChatContext } from '@/lib/chatContext';
import { type ChatToolEffectPayload, handleChatToolEffect } from '@/lib/chatToolEffects';
import { plainTextToHtml, plainTextToParagraphsHtml } from '@/lib/composeHtml';
import { freshDraftToOpen } from '@/lib/draftOpen';
import { errorText } from '@/lib/errors';
import { buildFeedbackEmail, type FeedbackType } from '@/lib/feedback';
import { planBackTarget } from '@/lib/swipeGesture';
import { viewTitleKey } from '@/lib/viewChrome';
import { isEmailListView, planViewChange } from '@/lib/viewNavigation';
import { isUnifiedMode, planChatAccountChange, selectAccountById, useAccountStore } from '@/stores/accountStore';
import { useAiStore } from '@/stores/aiStore';
import { calendarEnabledAccounts, useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useChatStore } from '@/stores/chatStore';
import { useConnectivityStore } from '@/stores/connectivityStore';
import { useEmailStore } from '@/stores/emailStore';
import {
  useLensesEnabledStore,
  useMemoryEnabledStore,
  useTasksEnabledStore,
  useTranslationEnabledStore,
} from '@/stores/featureToggleStore';
import { useJunkStore } from '@/stores/junkStore';
import { useLensStore } from '@/stores/lensStore';
import type { LogLevel, LogSource } from '@/stores/logStore';
import { useLogStore } from '@/stores/logStore';
import { useMemoryStore } from '@/stores/memoryStore';
import { useReminderStore } from '@/stores/reminderStore';
import { useTagStore } from '@/stores/tagStore';
import { useToastStore } from '@/stores/toastStore';
import { initTranslationListeners } from '@/stores/translationStore';
import { useUpdateStore } from '@/stores/updateStore';
import type {
  ActiveFilter,
  CalendarEvent,
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

function viewModeToMailbox(mode: ViewMode): MailboxView {
  if (mode === 'sent' || mode === 'spam' || mode === 'deleted') return mode;
  if (mode.startsWith('folder:')) return mode as MailboxView;
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
  const { t } = useTranslation(['common', 'modal', 'sidebar']);
  // Once, at the root: `useTheme` owns the `dark` class on <html>, and two
  // callers would fight over it.
  useTheme();
  const { enabled: aiEnabled, refresh: refreshAi } = useAiStore();
  // Onboarding: shown when the `onboarding_completed` preference is missing.
  // `null` = still loading the preference; we render nothing AI-conditional
  // until we know, otherwise the empty inbox flashes behind the wizard.
  const [onboardingCompleted, setOnboardingCompleted] = useState<boolean | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('inbox');
  // Where a back gesture returns to. One level, not a stack: "back" undoes the
  // navigation you just made, and a deep history would make the same gesture
  // mean something different depending on how you arrived. Falls back to the
  // inbox, which is the root.
  //
  // Recorded by observing `viewMode` rather than by routing every navigation
  // through one setter: `setViewMode` is called from ~20 places (sidebar, tool
  // effects, feature-disabled guards, deep links), and a helper that each of
  // them had to remember to use would be stale the first time one didn't.
  const previousViewMode = useRef<ViewMode>('inbox');
  const currentViewMode = useRef<ViewMode>(viewMode);
  useEffect(() => {
    if (currentViewMode.current !== viewMode) {
      previousViewMode.current = currentViewMode.current;
      currentViewMode.current = viewMode;
    }
  }, [viewMode]);
  const [inboxLayout, setInboxLayout] = usePersistedPref<InboxLayout>('inbox_layout', 'split', {
    parse: (raw) => (raw === 'split' || raw === 'full-width' ? raw : null),
    serialize: (v) => v,
  });
  const { isStacked } = useResponsiveLayout();
  // On a phone (or a desktop window dragged under the breakpoint) the two-pane
  // split has nowhere to put the second pane, so the effective layout is forced
  // to `full-width` — which is already a list → thread navigation stack,
  // complete with scroll restoration (see the note at the `full-width` branch
  // below). The user's *stored* preference is deliberately left untouched: it
  // is a desktop preference, and clobbering it here would silently rewrite it
  // the first time someone narrowed a window.
  const effectiveInboxLayout: InboxLayout = isStacked ? 'full-width' : inboxLayout;
  // Sidebar is a permanent column on desktop and an overlay drawer when
  // stacked, where it would otherwise consume most of the viewport.
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);
  // Bumped on every *open*. The drawer stays mounted while closed so it keeps
  // its expanded groups, which also meant it kept its scroll offset and came
  // back parked mid-list; this hands the Sidebar a change to reset on.
  const [sidebarOpenCount, setSidebarOpenCount] = useState(0);
  const openSidebar = useCallback(() => {
    setSidebarOpenCount((n) => n + 1);
    setIsSidebarOpen(true);
  }, []);
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
  const { refresh: refreshTranslationEnabled } = useTranslationEnabledStore();
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
  // Right-docked chat panel. Persisted so the workspace layout survives a
  // restart, the same way `inbox_layout` does.
  // Docked by default: chat is a primary surface, and a panel that starts
  // hidden makes it look absent. Persisted, so closing it sticks.
  const [isChatPanelOpen, setIsChatPanelOpen] = usePersistedPref<boolean>('chat_panel_open', true, {
    parse: (raw) => (raw === 'true' ? true : raw === 'false' ? false : null),
    serialize: (v) => String(v),
  });
  // Account the chat surfaces answer from. Chat is scoped to ONE account; in
  // unified ("All accounts") mode `effectiveAccountId` is merely the first
  // enabled one, so the user can retarget it independently of the list.
  // `null` = not yet retargeted, fall back to the mail view's account.
  const [chatAccountOverride, setChatAccountOverride] = useState<string | null>(null);
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
    isUnified,
    queryAccountId,
    effectiveAccountId,
    setActiveAccount,
    addAccount,
    registerImapAccount,
    removeAccount,
    syncAccount,
    syncAllAccounts,
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

  // `accounts` can drop the id `removeAccount` filters it out synchronously,
  // before the `onDelete` handler below gets to clear this state — so this
  // can legitimately be null for one render right after a successful delete.
  const accountSettingsAccount = selectAccountById(accounts, accountSettingsAccountId);

  // Per-account calendar-integration opt-in (Settings → Calendar). The
  // backend gates sync/notifications/chat on the same pref; this store only
  // controls which surfaces are visible. Calendar UI (sidebar entry, view)
  // exists only while at least one account has the integration enabled.
  const calendarIntegrationIds = useCalendarIntegrationStore((s) => s.enabledIds);
  const calendarIntegrationLoaded = useCalendarIntegrationStore((s) => s.isLoaded);
  const loadCalendarIntegration = useCalendarIntegrationStore((s) => s.loadForAccounts);
  useEffect(() => {
    void loadCalendarIntegration(accounts);
  }, [accounts, loadCalendarIntegration]);
  const calendarFeatureEnabled = useMemo(
    () => calendarEnabledAccounts(accounts, calendarIntegrationIds).length > 0,
    [accounts, calendarIntegrationIds],
  );
  // Leaving the calendar view when its last account gets switched off.
  useEffect(() => {
    if (viewMode === 'calendar' && calendarIntegrationLoaded && !calendarFeatureEnabled) setViewMode('inbox');
  }, [viewMode, calendarIntegrationLoaded, calendarFeatureEnabled]);

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
  // Stable key over the enabled non-IMAP accounts so the categories effect
  // (and unified-mode consumers) don't re-run on every accounts array identity
  // change from fetchAccounts.
  const enabledCategoryAccountsKey = useMemo(
    () =>
      accounts
        .filter((a) => a.enabled && a.provider !== 'imap')
        .map((a) => a.id)
        .join(','),
    [accounts],
  );
  useEffect(() => {
    if (!activeAccountId) {
      setAvailableCategories(null);
      return;
    }
    // Unified mode: union of every enabled non-IMAP account's categories, so
    // the tab strip can filter the merged list. Single account: as before.
    const targets = isUnified ? enabledCategoryAccountsKey.split(',').filter(Boolean) : [activeAccountId];
    if (targets.length === 0) {
      setAvailableCategories(null);
      return;
    }
    let cancelled = false;
    Promise.all(targets.map((id) => api.getAvailableCategories(id)))
      .then((results) => {
        if (cancelled) return;
        const valid = results.flat().filter((c): c is EmailCategory => VALID_CATEGORIES.has(c as EmailCategory));
        const union = Array.from(new Set(valid));
        // Empty list (IMAP, unknown provider) → null so showCategoryFilter
        // hides the chip strip entirely. Otherwise pass the provider-aware
        // set straight through.
        setAvailableCategories(union.length > 0 ? union : null);
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
  }, [activeAccountId, accountSettingsVersion, isUnified, enabledCategoryAccountsKey]);
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
    refreshTranslationEnabled().catch((err) => console.error('Failed to load ai_translation_enabled pref', err));
  }, [refreshAi, refreshMemoriesEnabled, refreshTasksEnabled, refreshLensesEnabled, refreshTranslationEnabled]);

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
  // In unified ("All accounts") mode every enabled account is "active", so
  // scoped errors from any of them stay visible.
  const isScopedAccountError =
    accountErrorAccountId !== null && accountErrorAccountId !== activeAccountId && !isUnified;
  const visibleAccountError = isScopedAccountError ? null : accountError;
  const displayError = visibleAccountError || emailError;
  // Tell the user WHICH account the banner is about — essential in unified
  // mode / with several accounts, where "Sync error: …" alone is unactionable.
  const displayErrorAccountEmail =
    visibleAccountError && accountErrorAccountId
      ? (accounts.find((a) => a.id === accountErrorAccountId)?.email ?? null)
      : null;
  const clearError = useCallback(() => {
    clearAccountError();
    clearEmailError();
  }, [clearAccountError, clearEmailError]);

  const handleReauthenticate = useCallback(async () => {
    // Re-auth the account the error actually belongs to (if known), not
    // whichever account happens to be active when the user clicks "Sign in
    // again". For non-scoped errors fall back to the active account (first
    // enabled account in unified mode — never the sentinel).
    const targetAccountId = accountErrorAccountId ?? effectiveAccountId;
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
  }, [accountErrorAccountId, effectiveAccountId, accounts, reauthenticateAccount, clearError, syncAccount, addLog]);

  // Open a compose tab pre-filled with a feedback email in the current UI
  // language. Runtime facts (app version, OS, AI provider) are gathered here
  // and interpolated into the localized body's "technical info" line.
  const handleGiveFeedback = useCallback(
    async (type: FeedbackType) => {
      if (!effectiveAccountId) return;
      try {
        const [diag, ai] = await Promise.all([api.getAppDiagnostics(), api.getAiConfig()]);
        // Use the i18next singleton so the fully-qualified `compose:` keys type-
        // check without widening this component's `t` namespace binding.
        const { to, subject, body } = buildFeedbackEmail(type, (key, options) => i18n.t(key, options), {
          appVersion: diag.appVersion,
          osPlatform: diag.osPlatform,
          osVersion: diag.osVersion,
          arch: diag.arch,
          translated: diag.translated,
          aiProvider: ai.provider,
          aiModel: ai.model,
        });
        setViewMode('inbox');
        // Preserve the template's blank lines (answer space between questions)
        // as empty paragraphs rather than collapsing them.
        openComposeTab(effectiveAccountId, [to], subject, plainTextToParagraphsHtml(body));
      } catch (error) {
        addLog('error', 'system', `Failed to open feedback email: ${errorText(error)}`);
      }
    },
    [effectiveAccountId, openComposeTab, addLog],
  );

  // Keep the memory store (tasks + badge counts) in sync with the active
  // account (first enabled account in unified mode — memory/tasks stay
  // per-account). Runs once per account switch — individual refreshes after
  // user actions are driven by the store itself.
  useEffect(() => {
    if (effectiveAccountId) {
      void useMemoryStore.getState().loadForAccount(effectiveAccountId);
    } else {
      useMemoryStore.getState().reset();
    }
  }, [effectiveAccountId]);

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

  // Route translation events (language-detected / email-translated /
  // translation-failed) into the translation store. Idempotent.
  useEffect(() => {
    initTranslationListeners();
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

    // Upcoming-meeting reminder — the backend fires this alongside the OS
    // notification (whose click macOS can't deliver), so the in-app banner is
    // the actionable surface. Validate the payload shape before trusting it.
    unlisteners.push(
      listen<{ event: CalendarEvent }>('meeting-reminder', (event) => {
        const meeting = event.payload?.event;
        if (meeting && typeof meeting.id === 'string' && typeof meeting.startTime === 'number') {
          useReminderStore.getState().show(meeting);
        } else {
          console.error('Ignoring malformed meeting-reminder payload', event.payload);
        }
      }),
    );

    // New-release notification — the backend checks GitHub daily and emits
    // this at most once per version. Validation + toast routing live in the
    // pure handler (`appUpdate.ts`); the i18n singleton resolves the message
    // in the current language at event time. The sticky toast announces; the
    // update store feeds the persistent sidebar link, seeded at startup from
    // the prefs the backend check persists.
    void useUpdateStore.getState().load();
    unlisteners.push(
      listen<UpdateAvailablePayload>('app-update-available', (event) => {
        handleUpdateAvailable(event.payload, {
          addToast: useToastStore.getState().addToast,
          t: (key, opts) => i18n.t(key, opts),
          openUrl: (url) => {
            void openExternal(url).catch((err) => {
              addLog('error', 'system', `Failed to open release page: ${errorText(err)}`);
            });
          },
          onAvailable: useUpdateStore.getState().setAvailable,
        });
      }),
    );

    // Backend-initiated calendar-integration change (permission-denied
    // auto-disable): the pref is already persisted — just mirror it so the
    // calendar surfaces hide immediately.
    unlisteners.push(
      listen<{ accountId: string; enabled: boolean }>('calendar-integration-changed', (event) => {
        const payload = event.payload;
        if (payload && typeof payload.accountId === 'string' && typeof payload.enabled === 'boolean') {
          useCalendarIntegrationStore.getState().applyBackendChange(payload.accountId, payload.enabled);
        } else {
          console.error('Ignoring malformed calendar-integration-changed payload', payload);
        }
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

    // The junk display preference drives whether flagged rows are faded or
    // removed from the list, so it has to be in the store before the first render
    // of the inbox rather than fetched per row.
    void useJunkStore.getState().loadConfig();

    // Junk verdicts land asynchronously after each sync pass. Merge the chip
    // into whatever tags the row already cached rather than replacing them —
    // classification and junk scoring run independently and neither should
    // clobber the other's result.
    unlisteners.push(
      listen<{ emailId: string; kind: string }>('email-junk-scored', (event) => {
        const { emailId, kind } = event.payload;
        const now = Math.floor(Date.now() / 1000);
        // Clear the junk store's cached miss so an open message picks up a
        // verdict that landed after it was first rendered.
        useJunkStore.getState().invalidate(emailId);
        void useJunkStore.getState().loadVerdicts([emailId]);
        const existing = useTagStore.getState().tagsByEmail[emailId] ?? [];
        useTagStore
          .getState()
          .setEmailTags(emailId, [
            ...existing.filter((t) => t.tagType !== 'junk'),
            { emailId, tagType: 'junk', tagValue: kind, confidence: null, createdAt: now },
          ]);
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

    // Unified ("All accounts") mode: reset the list and enqueue a sync for
    // every enabled account. The backend runs per-account queues, so the
    // syncs proceed independently; progress events drive list refreshes.
    if (isUnified) {
      resetEmails();
      useChatStore.getState().reset();

      if (!useConnectivityStore.getState().isOnline) {
        addLog('info', 'sync', 'Sync skipped — currently offline.');
        return;
      }
      const pendingSetupId = useAccountStore.getState().setupPendingAccountId;
      const toSync = accounts.filter((a) => a.enabled && a.id !== pendingSetupId).map((a) => a.id);
      if (toSync.length === 0) return;

      addLog('info', 'sync', `Syncing ${toSync.length} account${toSync.length > 1 ? 's' : ''}...`);
      syncAllAccounts(toSync).catch((err) => {
        if (useConnectivityStore.getState().isOnline) {
          addLog('error', 'sync', `Sync failed: ${err}`);
          console.error('Sync failed:', err);
        }
      });
      return;
    }

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

    // In unified mode progress from ANY account affects the merged list, so
    // accept every account's events; otherwise only the active account's.
    if (!syncProgress || (!isUnified && syncProgress.accountId !== activeAccountId)) {
      return;
    }

    // Refresh the open thread (selected pane / thread tab) for the synced
    // account, so a just-sent optimistic row is transparently swapped for the
    // provider's reconciled copy. Read the store fresh — capturing thread
    // state in this closure would go stale (see src/CLAUDE.md).
    const refreshOpenThread = () => {
      const emailStore = useEmailStore.getState();
      const selected = emailStore.selectedEmail;
      if (selected && selected.accountId === syncProgress.accountId) {
        void emailStore.refreshThread(selected.accountId, selected.threadId);
      }
      const activeTab = emailStore.tabs.find((t) => t.id === emailStore.activeTabId);
      if (
        activeTab?.type === 'thread' &&
        activeTab.accountId === syncProgress.accountId &&
        activeTab.threadId !== selected?.threadId
      ) {
        void emailStore.refreshThread(activeTab.accountId, activeTab.threadId);
      }
    };

    if (status === 'batch') {
      // New emails were just written to DB — refresh the list in-place so they
      // appear while the rest of the sync is still running.
      silentRefetchEmailsRef.current();
      refreshOpenThread();
    }

    if (status === 'complete' && previousStatus !== 'complete') {
      // Use silent refresh so the list updates in-place without a loading spinner
      // or scroll-position reset — the sync should be transparent to the user.
      silentRefetchEmailsRef.current();
      refreshOpenThread();
      forceRefreshFilters();
    }
  }, [activeAccountId, isUnified, forceRefreshFilters, syncProgress]);

  // After every successful send the backend has already stored the optimistic
  // Sent row — refetch the list so the message appears instantly (most visibly
  // in the Sent view). `sentRefreshTick` is bumped by the reply/compose flows.
  const sentRefreshTick = useEmailStore((s) => s.sentRefreshTick);
  useEffect(() => {
    if (sentRefreshTick === 0) return;
    silentRefetchEmailsRef.current();
  }, [sentRefreshTick]);

  // Refresh the merged list when the enabled-account set changes while in
  // unified mode (toggling an account on/off must add/remove its emails).
  const enabledAccountsKey = useMemo(
    () =>
      accounts
        .filter((a) => a.enabled)
        .map((a) => a.id)
        .join(','),
    [accounts],
  );
  useEffect(() => {
    if (!isUnified) return;
    silentRefetchEmailsRef.current();
  }, [isUnified, enabledAccountsKey]);

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
      // Attachments are scoped to the effective account (first enabled in
      // unified mode), so the email always belongs to that account.
      if (!effectiveAccountId) return;
      clearActiveFilter();
      clearSearchQuery();
      setViewMode('inbox');
      navigateToEmail(effectiveAccountId, emailId);
    },
    [effectiveAccountId, clearActiveFilter, clearSearchQuery, navigateToEmail],
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

  // What the right-hand chat panel offers as ambient context: the thread the
  // main view is currently showing, or null everywhere else. Pure derivation —
  // see `deriveChatContext` for the precedence rules.
  const chatPanelContext = useMemo(
    () => deriveChatContext({ viewMode, activeTab, selectedEmail }),
    [viewMode, activeTab, selectedEmail],
  );

  // Header new-chat icon: start a fresh conversation and dock the panel in one
  // click, from anywhere in the app. Open the panel even if creating the
  // conversation fails — the panel's own empty state is a better place to see
  // the error than a silently ignored click.
  // Selecting ONE account in the sidebar re-points chat at it, so browsing an
  // account and chatting about it stay in step. Switching the list to "All
  // accounts" deliberately does NOT: chat cannot answer from every account, so
  // it keeps the concrete one it had (shown in its picker).
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed on the mail
  // selection changing, not on the override's own value.
  useEffect(() => {
    if (!isUnifiedMode(activeAccountId)) setChatAccountOverride(activeAccountId);
  }, [activeAccountId]);

  const chatAccountId = chatAccountOverride ?? effectiveAccountId;

  // The other direction: retargeting chat drags the list along, EXCEPT in
  // unified mode where the list stays as the user left it (see
  // `planChatAccountChange`).
  const handleChatAccountChange = useCallback(
    (nextAccountId: string) => {
      const plan = planChatAccountChange(nextAccountId, activeAccountId);
      setChatAccountOverride(plan.chatAccountId);
      if (plan.mailAccountId) setActiveAccount(plan.mailAccountId);
    },
    [activeAccountId, setActiveAccount],
  );

  const handleNewChat = useCallback(async () => {
    // Stacked has nowhere to dock a 280px-minimum panel beside the mail, so a
    // new chat opens the full-page view instead. Same conversation either way.
    if (isStacked) {
      setViewMode('chat');
    } else {
      if (viewMode === 'chat') setViewMode('inbox');
      setIsChatPanelOpen(true);
    }
    if (!effectiveAccountId) return;
    try {
      await useChatStore.getState().createConversation(effectiveAccountId);
    } catch (e) {
      addLog('error', 'ai', `Failed to start a new chat: ${errorText(e)}`);
    }
  }, [isStacked, viewMode, effectiveAccountId, setIsChatPanelOpen, addLog]);

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
    if (!activeAccountId) return;
    // Skip the network round-trip when we know we're offline — the request
    // would fail and the user would see a misleading "Sync failed: ..."
    // error in the log instead of the offline banner.
    if (!useConnectivityStore.getState().isOnline) {
      addLog('info', 'sync', 'Sync skipped — currently offline.');
      return;
    }
    try {
      if (isUnified) {
        const toSync = accounts.filter((a) => a.enabled).map((a) => a.id);
        if (toSync.length === 0) return;
        addLog('info', 'sync', `Manual sync started for ${toSync.length} account${toSync.length > 1 ? 's' : ''}`);
        await syncAllAccounts(toSync);
      } else {
        const accountEmail = accounts.find((a) => a.id === activeAccountId)?.email;
        addLog('info', 'sync', `Manual sync started for ${accountEmail ?? 'account'}`);
        await syncAccount(activeAccountId);
      }
    } catch (error) {
      addLog('error', 'sync', `Sync failed: ${error}`);
      console.error('Sync failed:', error);
    }
  };

  // Swipe navigation, phone only. One handler for the whole navigation stack:
  // two window listeners racing to interpret the same swipe is the only
  // alternative, and it loses.
  //
  // "Back" unwinds one level (see `planBackTarget` for the rule):
  //   1. an open message -> the list it came from
  //   2. any other view  -> the view the user came from
  // The inbox is the root and has nowhere to go, so the gesture is inert there
  // rather than guessing. Chat is deliberately not special-cased: swiping out
  // of a conversation leaves chat, it does not stop at the conversation list.
  // A compose tab is excluded: "back" there would throw away what the user is
  // typing, which is not what a stray flick should be able to do.
  const isThreadOpen =
    isEmailListView(viewMode) && activeTab?.type !== 'compose' && (activeTab !== null || selectedEmail !== null);
  useSwipeNavigation({
    enabled: isStacked,
    isSidebarOpen,
    canGoBack: planBackTarget({ viewMode, isThreadOpen }) !== 'none',
    onBack: () => {
      switch (planBackTarget({ viewMode, isThreadOpen })) {
        case 'closeThread':
          if (activeTab) closeTab(activeTab.id);
          void selectEmail(null);
          break;
        case 'previousView':
          // Guard the degenerate case: if nothing was recorded (first view of
          // the session was not the inbox), fall back to the root.
          setViewMode(previousViewMode.current === viewMode ? 'inbox' : previousViewMode.current);
          break;
        case 'none':
          break;
      }
    },
    onCloseSidebar: () => setIsSidebarOpen(false),
  });

  // Title for the stacked top bar: "<view> — <mailbox>". Folder views have no
  // locale key (the name is the user's own), so they fall back to the server
  // path's last segment.
  const viewKey = viewTitleKey(viewMode);
  const viewLabel = viewKey
    ? t(viewKey)
    : viewMode.startsWith('folder:')
      ? (viewMode.slice('folder:'.length).split(/[/.]/).pop() ?? '')
      : '';
  const mobileHeaderScope = isUnified ? t('sidebar:allAccounts') : activeAccount?.name || activeAccount?.email;
  const mobileHeaderTitle = mobileHeaderScope ? `${viewLabel} — ${mobileHeaderScope}` : viewLabel;

  return (
    // `h-full` rather than `h-screen`: #root is already sized to 100dvh and
    // carries the safe-area padding, and `h-screen` (100vh) would overflow it
    // by the height of the iOS home indicator.
    <div className="flex flex-col h-full bg-gray-50 dark:bg-surface-raised">
      {onboardingCompleted === false && (
        <OnboardingWizard
          currentLayout={inboxLayout}
          onChangeLayout={handleChangeLayout}
          onComplete={handleOnboardingComplete}
        />
      )}
      <OfflineBanner />
      <MeetingReminderBanner />
      <ErrorBanner
        message={displayError}
        accountEmail={displayErrorAccountEmail}
        onDismiss={clearError}
        onReauthenticate={handleReauthenticate}
      />

      {isStacked && (
        // Stacked mode has no permanent sidebar column, so navigation needs an
        // entry point. Rendered outside the drawer so it stays reachable.
        <div className="flex items-center gap-2 border-b border-gray-200 bg-white px-2 py-1 dark:border-gray-700 dark:bg-surface">
          <button
            type="button"
            onClick={openSidebar}
            aria-label={t('sidebar:openMenu')}
            className="flex h-11 w-11 items-center justify-center rounded-md text-gray-600 active:bg-gray-100 dark:text-gray-400 dark:active:bg-surface-hover"
          >
            <svg className="h-5 w-5" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={2}>
              <path d="M3 5h14M3 10h14M3 15h14" strokeLinecap="round" />
            </svg>
          </button>
          {/* Which view, and whose mailbox. Each screen used to title itself,
              which cost a row per view and left several (chat, calendar)
              with no title at all. The trailing spacer matches the menu
              button so the title is centered on the bar, not on the space
              left over beside it. */}
          <div className="flex min-w-0 flex-1 items-center justify-center gap-1.5">
            <h1 className="truncate text-sm font-semibold text-gray-900 dark:text-gray-100">{mobileHeaderTitle}</h1>
            {isSyncing && (
              <div className="h-3 w-3 flex-shrink-0 animate-spin rounded-full border-b-2 border-primary-600" />
            )}
          </div>
          <div className="h-11 w-11 flex-shrink-0" aria-hidden="true" />
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        {isStacked && isSidebarOpen && (
          <button
            type="button"
            aria-label={t('sidebar:closeMenu')}
            onClick={() => setIsSidebarOpen(false)}
            className="fixed inset-0 z-40 bg-black/40"
          />
        )}
        <div
          className={
            isStacked
              ? // `fixed` + translate rather than conditional mounting: keeping
                // the Sidebar mounted preserves its internal state (expanded
                // account groups, filter list) across open/close.
                `fixed inset-y-0 left-0 z-50 flex w-[85%] max-w-xs transform transition-transform duration-200 
                  pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] pl-[env(safe-area-inset-left)] 
                  bg-gray-900 ${isSidebarOpen ? 'translate-x-0' : '-translate-x-full'}`
              : // `contents` makes this wrapper invisible to layout, so the
                // desktop flex row is byte-for-byte what it was before.
                'contents'
          }
        >
          <Sidebar
            scrollResetToken={sidebarOpenCount}
            accounts={accounts}
            activeAccount={activeAccount}
            isUnifiedActive={isUnified}
            onSelectAccount={(id) => {
              setViewMode('inbox');
              clearSearchQuery();
              clearActiveFilter();
              setSelectedCategories(new Set<EmailCategory>(['primary']));
              setActiveAccount(id);
              setIsSidebarOpen(false);
            }}
            onAddAccount={() => setIsAddAccountPickerOpen(true)}
            onMoveAccountUp={moveAccountUp}
            onMoveAccountDown={moveAccountDown}
            onToggleAccountEnabled={setAccountEnabled}
            onSync={handleSync}
            onCompose={() => setIsComposeOpen(true)}
            onGiveFeedback={handleGiveFeedback}
            onOpenSearch={() => setIsSearchOpen(true)}
            onOpenAccountSettings={(id) => setAccountSettingsAccountId(id)}
            onOpenAppSettings={() => setSettingsTab('appearance')}
            isSyncing={isSyncing}
            viewMode={viewMode}
            onOpenChatView={() => {
              setViewMode('chat');
              // Dismiss the drawer so the chat view is visible; on desktop
              // this is a no-op because the drawer is never open.
              setIsSidebarOpen(false);
            }}
            onSetViewMode={(mode) => {
              const plan = planViewChange(mode, effectiveInboxLayout);
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
              // Dismiss the drawer so the view the user just picked is visible;
              // on desktop this is a no-op because the drawer is never open.
              setIsSidebarOpen(false);
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
            calendarEnabled={calendarFeatureEnabled}
            onSelectLens={(lensId) => {
              // Selecting a lens in the sidebar: fire the store action so the
              // Lenses view picks it up (it already subscribes to activeLensId),
              // then switch the view mode.
              void useLensStore.getState().selectLens(lensId);
              setViewMode('lenses');
              setIsSidebarOpen(false);
            }}
          />
        </div>

        {/* `min-w-0` is load-bearing: a flex item defaults to min-width:auto,
            which refuses to shrink below its content's min-content width. With
            the chat panel docked alongside, that made this column overflow and
            clip its own right edge (hiding the inbox toolbar buttons) instead
            of narrowing. */}
        <main className="flex flex-1 min-w-0 overflow-hidden">
          {viewMode === 'contacts' ? (
            // Per-account views wrap in a column with the unified-mode scope
            // chip on top (self-gating — renders nothing outside All accounts).
            <div className="flex flex-col flex-1 overflow-hidden">
              <UnifiedScopeBar accountId={effectiveAccountId} />
              <ContactsView
                accountId={effectiveAccountId}
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
            </div>
          ) : viewMode === 'drafts' ? (
            <div className="flex flex-col flex-1 overflow-hidden">
              <UnifiedScopeBar accountId={effectiveAccountId} />
              <DraftsView
                accountId={effectiveAccountId}
                accounts={accounts}
                syncProgress={syncProgress}
                onOpenComposeTab={async (draft) => {
                  // Compose tabs render inside the inbox/mail pane, not the drafts
                  // view — switch back so the opened tab is actually visible.
                  setViewMode('inbox');
                  // Open what the provider has: a draft edited in Gmail would
                  // otherwise open with the row this list rendered.
                  const fresh = await freshDraftToOpen(draft);
                  openComposeTab(
                    fresh.accountId,
                    fresh.toAddresses,
                    fresh.subject,
                    fresh.bodyHtml ?? plainTextToHtml(fresh.body),
                    { draftId: fresh.id, ccAddresses: fresh.ccAddresses, attachments: fresh.attachments },
                  );
                }}
              />
            </div>
          ) : viewMode === 'chat' ? (
            // Chat conversations are hard-scoped to one account; in unified
            // mode they fall back to the first enabled account. ChatView
            // renders its own scope chip (chat-specific hint).
            <ChatView
              accountId={chatAccountId}
              onAccountChange={handleChatAccountChange}
              onNavigateToInbox={() => setViewMode('inbox')}
            />
          ) : viewMode === 'tasks' && tasksEnabled ? (
            <div className="flex flex-col flex-1 overflow-hidden">
              <UnifiedScopeBar accountId={effectiveAccountId} />
              <TasksPanel accountId={effectiveAccountId} />
            </div>
          ) : viewMode === 'memory' && memoriesEnabled ? (
            <div className="flex flex-col flex-1 overflow-hidden">
              <UnifiedScopeBar accountId={effectiveAccountId} />
              <MemoryView accountId={effectiveAccountId} />
            </div>
          ) : viewMode === 'lenses' && lensesEnabled ? (
            <LensesView />
          ) : viewMode === 'logs' ? (
            // Phone only — the sidebar offers this entry only when stacked,
            // because the desktop window docks the same content at the bottom.
            <LogView />
          ) : viewMode === 'dashboard' ? (
            <Dashboard accounts={accounts} onOpenAccountSettings={(id) => setAccountSettingsAccountId(id)} />
          ) : viewMode === 'calendar' ? (
            // Calendar is per-account only (docs/DECISIONS.md) — it renders its
            // own compact account selector, so no UnifiedScopeBar here.
            <CalendarView accounts={accounts} defaultAccountId={effectiveAccountId} />
          ) : isEmailListView(viewMode) ? (
            // biome-ignore lint/complexity/noUselessFragments: IIFE result needs a fragment wrapper for the ternary
            <>
              {(() => {
                // When a tab is active, show its content; otherwise show the main selected email.
                const displayThreadEmails = activeTab?.type === 'thread' ? activeTab.threadEmails : threadEmails;
                const displayIsLoading = activeTab?.type === 'thread' ? activeTab.isLoading : isLoadingThread;
                // In full-width mode, "close/back" always returns to the inbox list by
                // clearing both the active tab and the selected email.
                const displayOnClose =
                  effectiveInboxLayout === 'full-width'
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
                          effectiveInboxLayout === 'full-width'
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
                        // null in unified mode → EmailView's own fallback picks
                        // the thread's latest email's account as the reply-from.
                        activeAccountId={queryAccountId}
                        fullWidth={effectiveInboxLayout === 'full-width'}
                        onOpenInTab={handleOpenInTab}
                      />
                    )}
                  </div>
                );

                const inboxList = isInboxCollapsed ? (
                  <button
                    onClick={() => setIsInboxCollapsed(false)}
                    title={t('modal:inbox.expand')}
                    className="w-8 flex-shrink-0 border-r border-gray-200 bg-white flex items-start justify-center pt-4 text-gray-400 hover:text-gray-600 hover:bg-gray-50 transition-colors dark:border-gray-700 dark:bg-surface dark:text-gray-500 dark:hover:text-gray-400 dark:hover:bg-surface-raised"
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
                    selectedEmailId={effectiveInboxLayout === 'split' ? (selectedEmail?.id ?? null) : null}
                    disableAutoSelect={effectiveInboxLayout === 'full-width'}
                    fullWidth={effectiveInboxLayout === 'full-width'}
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
                    showCategoryFilter={
                      (isUnified
                        ? accounts.some((a) => a.enabled && a.provider !== 'imap')
                        : activeAccount?.provider !== 'imap') && viewMode === 'inbox'
                    }
                    availableCategories={availableCategories ?? undefined}
                    onCollapse={effectiveInboxLayout === 'split' ? () => setIsInboxCollapsed(true) : undefined}
                    onNewChat={aiEnabled ? () => void handleNewChat() : undefined}
                    onOpenInTab={openTab}
                    onChatAboutThread={aiEnabled ? handleChatAboutThread : undefined}
                    accountName={isUnified ? t('sidebar:allAccounts') : activeAccount?.name || activeAccount?.email}
                    accountId={activeAccountId}
                    onSearch={handleApplySearch}
                  />
                );

                if (effectiveInboxLayout === 'full-width') {
                  // Keep the inbox mounted (hidden) so its state — loaded pages,
                  // filters, virtualizer measurements — survives going back.
                  //
                  // NOTE: `hidden` is display:none, which does NOT preserve
                  // scroll position. The browser resets the scroll container's
                  // scrollTop to 0 and fires no scroll event, which also strands
                  // the row virtualizer on a stale offset (the "blank band above
                  // the rows after Back" bug). VirtualEmailList saves and
                  // restores the position itself — see src/lib/scrollRestore.ts.
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
              <UnifiedScopeBar accountId={effectiveAccountId} />
              <AttachmentToolbar
                accountId={effectiveAccountId}
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

        {/* Right-docked chat. Gated on the master AI switch like every other
            AI surface, and suppressed while the full-page chat view is open so
            the same conversation isn't rendered twice side by side. */}
        {/* Never docked when stacked: the panel's 280px minimum would leave a
            phone ~110px of mail beside it. The full-page chat view is the
            mobile equivalent, and `handleNewChat` routes there instead. */}
        {aiEnabled && !isStacked && isChatPanelOpen && viewMode !== 'chat' && (
          <ChatPanelDock
            accountId={chatAccountId}
            onAccountChange={handleChatAccountChange}
            context={chatPanelContext}
            onClose={() => setIsChatPanelOpen(false)}
            onExpand={() => {
              setIsChatPanelOpen(false);
              setViewMode('chat');
            }}
            onNavigateToInbox={() => setViewMode('inbox')}
          />
        )}
      </div>

      {/* The output panel is a developer/diagnostics surface whose collapsed bar
          still costs a permanent strip of vertical space, and its controls
          (module filter, provider picker, gear, trash) do not fit a phone width
          — they overlap at 390px. Desktop keeps it exactly as before. Backend
          `app-log` events still flow into useLogStore either way, so nothing is
          lost; the log is simply not displayed on a phone. */}
      {!isStacked && <LogPanel onOpenAiSettings={() => setSettingsTab('ai')} />}
      <ToastHost />

      {isSearchOpen && (
        <SearchBar
          accountId={queryAccountId}
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

      {isRuleModalOpen && effectiveAccountId && (
        <RuleManagementModal
          rules={attachmentRules}
          accountId={effectiveAccountId}
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

      {accountSettingsAccount && (
        <AccountSettingsDialog
          account={accountSettingsAccount}
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

      {isComposeOpen && effectiveAccountId && (
        <ComposeModal
          accounts={accounts}
          defaultAccountId={effectiveAccountId}
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
          activeAccountId={effectiveAccountId}
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
