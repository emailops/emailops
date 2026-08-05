import { type ReactNode, useCallback, useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useResponsiveLayout } from '@/hooks/useResponsiveLayout';
import { accountColorClass } from '@/lib/colors';
import { shouldShowCategoryTabs } from '@/lib/viewChrome';
import { isUnifiedMode, type SyncProgress, selectEffectiveAccountId, useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFilterStore } from '@/stores/filterStore';
import { isHiddenFromInbox, isFlagged as isJunkFlagged, useJunkStore } from '@/stores/junkStore';
import { useTagStore } from '@/stores/tagStore';
import type { Email, EmailCategory } from '@/types';
import type { RulePrefill } from './EmailRow';
import { InboxSearchBox } from './InboxSearchBox';
import { VirtualEmailList } from './VirtualEmailList';

interface InboxProps {
  emails: Email[];
  isLoading: boolean;
  isSyncing: boolean;
  syncProgress: SyncProgress | null;
  isLoadingMore: boolean;
  hasMore: boolean;
  totalCount: number;
  selectedEmailId: string | null;
  onSelectEmail: (email: Email) => void;
  onLoadMore: () => void;
  onAddSenderFilter?: (senderEmail: string) => void;
  onBlockSender?: (senderEmail: string) => void;
  onCreateAttachmentRule?: (prefill: RulePrefill) => void;
  onCreateClassificationRule?: (prefill: RulePrefill) => void;
  selectedCategories: Set<EmailCategory>;
  /** Replace the current category selection with a new set. Called when the user picks a tab. */
  onSelectCategories: (categories: Set<EmailCategory>) => void;
  /** When false, category tabs are hidden and filtering is bypassed (e.g. for IMAP accounts). */
  showCategoryFilter?: boolean;
  /** Categories actually configured for sync on the active Gmail account. Drives which
   *  tabs are visible — users can't filter by a category they haven't opted to sync.
   *  When undefined, no tabs render (treated as "not yet known"). The parent should
   *  combine this with `showCategoryFilter` to gate visibility for non-Gmail accounts. */
  availableCategories?: EmailCategory[];
  onCollapse?: () => void;
  /** Start a fresh chat and dock the panel. Omitted when AI is disabled. */
  onNewChat?: () => void;
  onOpenInTab?: (email: Email) => void;
  /** Open a new chat session seeded with the cleaned email thread. */
  onChatAboutThread?: (email: Email) => void;
  /** When true, the first email is not auto-selected on load (used in full-width layout). */
  disableAutoSelect?: boolean;
  /** When true, the list fills the available width instead of using the fixed 384px sidebar width. */
  fullWidth?: boolean;
  /** Display name of the active account shown in the inbox title. */
  accountName?: string;
  /** Account ID used for sender autocomplete in the inline search box. */
  accountId?: string | null;
  /** Called when the user submits a search query from the inline search box. */
  onSearch?: (query: string) => void;
}

interface CategoryTabConfig {
  key: EmailCategory;
  label: string;
  /** Tailwind color classes for the active tab (text + bottom-border). */
  activeColor: string;
  icon: ReactNode;
}

const CATEGORIES: CategoryTabConfig[] = [
  {
    key: 'primary',
    label: 'Primary',
    activeColor: 'text-blue-600 border-blue-600',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
        />
      </svg>
    ),
  },
  {
    key: 'social',
    label: 'Social',
    activeColor: 'text-pink-600 border-pink-600',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M17 20h5v-2a4 4 0 00-3-3.87M9 20H4v-2a4 4 0 013-3.87m6-2a4 4 0 100-8 4 4 0 000 8zm6 0a3 3 0 100-6 3 3 0 000 6zM7 12a3 3 0 100-6 3 3 0 000 6z"
        />
      </svg>
    ),
  },
  {
    key: 'updates',
    label: 'Updates',
    activeColor: 'text-amber-600 border-amber-600',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
    ),
  },
  {
    key: 'forums',
    label: 'Forums',
    activeColor: 'text-purple-600 border-purple-600',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M17 8h2a2 2 0 012 2v6a2 2 0 01-2 2h-2v3l-4-3H7a2 2 0 01-2-2v-1m12-6V5a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2h2l4 3v-3h2"
        />
      </svg>
    ),
  },
  {
    key: 'promotions',
    label: 'Promotions',
    activeColor: 'text-emerald-600 border-emerald-600',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M7 7h.01M7 3h5c.512 0 1.024.195 1.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.994 1.994 0 013 12V7a4 4 0 014-4z"
        />
      </svg>
    ),
  },
];

export function Inbox({
  emails,
  isLoading,
  isSyncing,
  isLoadingMore,
  hasMore,
  selectedEmailId,
  onSelectEmail,
  onLoadMore,
  onAddSenderFilter,
  onBlockSender,
  onCreateAttachmentRule,
  onCreateClassificationRule,
  selectedCategories,
  onSelectCategories,
  showCategoryFilter = true,
  availableCategories,
  onCollapse,
  onNewChat,
  onOpenInTab,
  onChatAboutThread,
  disableAutoSelect = false,
  fullWidth = false,
  accountName,
  accountId,
  onSearch,
}: InboxProps) {
  const { t } = useTranslation(['inbox', 'common', 'chat']);
  const { isStacked } = useResponsiveLayout();
  // Only render tabs for categories the active account is actually
  // syncing. Unspecified → render no tabs. Previously this fell back to a
  // hard-coded Primary/Social/Updates trio, which on a fresh app start
  // briefly showed Updates/Social as filterable even when the Gmail account
  // was configured to sync only Primary (the parent passes `undefined` until
  // getAccountSettings resolves).
  const visibleCategories = useMemo(() => {
    if (!availableCategories) return [];
    const allowed = new Set(availableCategories);
    return CATEGORIES.filter((c) => allowed.has(c.key));
  }, [availableCategories]);

  // Active tab selection model:
  // - "all" when every visible category is selected (or none — treat as "all")
  // - a single category key when exactly that one is selected
  // - falls back to "all" for mixed states (which only happen via legacy persisted prefs)
  const activeTabKey: 'all' | EmailCategory = useMemo(() => {
    if (visibleCategories.length === 0) return 'all';
    const allSelected = visibleCategories.every((c) => selectedCategories.has(c.key));
    if (allSelected || selectedCategories.size === 0) return 'all';
    if (selectedCategories.size === 1) {
      const only = Array.from(selectedCategories)[0];
      if (visibleCategories.some((c) => c.key === only)) return only;
    }
    return 'all';
  }, [visibleCategories, selectedCategories]);

  // Per-account guard: ensures the auto-expand-to-All recovery (see effect below)
  // fires at most once per account per app session, so an explicit user pick
  // isn't immediately overridden.
  const autoExpandedAccountsRef = useRef<Set<string>>(new Set());

  const handleTabClick = useCallback(
    (key: 'all' | EmailCategory) => {
      if (key === 'all') {
        onSelectCategories(new Set(visibleCategories.map((c) => c.key)));
      } else {
        onSelectCategories(new Set([key]));
      }
      // User made an explicit choice — mark this account as handled so the
      // auto-expand effect below stops nudging the selection.
      if (accountId) autoExpandedAccountsRef.current.add(accountId);
    },
    [accountId, onSelectCategories, visibleCategories],
  );
  // `min-w-0` is load-bearing, not decorative: a flex item defaults to
  // `min-width: auto`, which refuses to shrink below its content's intrinsic
  // width. On a desktop that bites when something else — e.g. the docked chat
  // panel — takes horizontal space, and the column overflows (clipping its own
  // toolbar buttons) instead of narrowing. On a phone it let the header and the
  // empty-state text push the pane wider than the screen and clip off the right
  // edge.
  const widthClass = fullWidth ? 'flex-1 min-w-0' : 'w-96';
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  // Unified ("All accounts") mode: rows get a colored per-account indicator,
  // and sender autocomplete falls back to the first enabled account (the
  // sentinel id must never reach the backend).
  const isUnified = useAccountStore((s) => isUnifiedMode(s.activeAccountId));
  const allAccounts = useAccountStore((s) => s.accounts);
  const autocompleteAccountId = useAccountStore((s) => selectEffectiveAccountId(s.accounts, s.activeAccountId));
  const getAccountBadge = useMemo(() => {
    if (!isUnified) return undefined;
    const emailById = new Map(allAccounts.map((a) => [a.id, a.email]));
    return (email: Email) => ({
      colorClass: accountColorClass(email.accountId),
      label: emailById.get(email.accountId) ?? email.accountId,
    });
  }, [isUnified, allAccounts]);

  const activeFilter = useFilterStore((s) => s.activeFilter);
  const searchQuery = useEmailStore((s) => s.searchQuery);
  const clearSearchQuery = useEmailStore((s) => s.clearSearchQuery);
  const focusEmailId = useEmailStore((s) => s.focusEmailId);
  const navigationMode = useEmailStore((s) => s.navigationMode);

  const filteredEmails = useMemo(() => {
    // When search or smart filter is active, emails are already server-filtered
    if (searchQuery || activeFilter) return emails;
    // In navigation mode (jumped to a specific email), skip category filtering
    if (navigationMode) return emails;
    // Category filter is Gmail-specific; skip for IMAP and other non-Gmail accounts
    if (!showCategoryFilter) return emails;
    if (selectedCategories.size === 0) return emails;
    const filtered = emails.filter((email) => selectedCategories.has(email.category));
    // Edge case: during an initial sync the first batch may all be promotions/forums
    // (categories typically unchecked). Showing "no emails" would be misleading — the
    // first primary messages are still on their way. Only bypass the filter when it
    // yields zero results *and* a sync is actively running.
    if (isSyncing && filtered.length === 0 && emails.length > 0) return emails;
    return filtered;
  }, [emails, selectedCategories, activeFilter, navigationMode, showCategoryFilter, searchQuery, isSyncing]);

  // Junk removal is applied last and only when the user asked for it. It is a
  // view filter, nothing more: the message stays where the server put it and
  // remains reachable through search and the provider's own folders. A search
  // or an explicit smart filter bypasses it — when the user went looking for
  // something, hiding results would be the wrong answer.
  const junkFlaggedAction = useJunkStore((s) => s.flaggedAction);
  const junkVerdicts = useJunkStore((s) => s.verdictsByEmail);
  const visibleEmails = useMemo(() => {
    if (junkFlaggedAction !== 'hide') return filteredEmails;
    if (searchQuery || activeFilter) return filteredEmails;
    return filteredEmails.filter((email) => !isHiddenFromInbox(junkVerdicts[email.id], junkFlaggedAction));
  }, [filteredEmails, junkFlaggedAction, junkVerdicts, searchQuery, activeFilter]);

  // Whether any of the loaded messages are flagged, regardless of the current
  // setting — the toggle only appears when it would do something.
  //
  // Deliberately not shown as a number. This counts the *loaded* rows, and the
  // list loads incrementally, so the figure climbed as the user scrolled: a
  // label that changes while you read it describes nothing, and reads as though
  // junk were arriving live.
  const flaggedCount = useMemo(
    () => filteredEmails.filter((email) => isJunkFlagged(junkVerdicts[email.id])).length,
    [filteredEmails, junkVerdicts],
  );
  const setJunkFlaggedAction = useJunkStore((s) => s.setFlaggedAction);

  // Auto-expand the category selection to "All" when the active filter would
  // show an empty list despite emails being available in other categories.
  // Common after the first sync of a new account: the default selection is
  // "Primary" but the initial batch may only contain Updates/Promotions, so
  // the user would otherwise see a misleading empty inbox. We only nudge once
  // per account per session (tracked via the ref) so an explicit choice by
  // the user — including via handleTabClick above — is respected afterwards.
  useEffect(() => {
    if (!accountId || !showCategoryFilter) return;
    if (isLoading || isSyncing) return;
    // Skip while search or a smart filter is active: `emails` is server-filtered
    // to those results and category tabs are visually bypassed (see filteredEmails
    // above). Mutating the category selection here would silently overwrite the
    // user's preference for when they clear the search.
    if (searchQuery || activeFilter) return;
    if (visibleCategories.length <= 1) return;
    if (emails.length === 0) return;
    if (autoExpandedAccountsRef.current.has(accountId)) return;
    const allSelected = visibleCategories.every((c) => selectedCategories.has(c.key));
    if (allSelected) {
      autoExpandedAccountsRef.current.add(accountId);
      return;
    }
    const hasAnyMatch = emails.some((e) => selectedCategories.has(e.category));
    if (!hasAnyMatch) {
      autoExpandedAccountsRef.current.add(accountId);
      onSelectCategories(new Set(visibleCategories.map((c) => c.key)));
    }
  }, [
    accountId,
    emails,
    selectedCategories,
    visibleCategories,
    isSyncing,
    isLoading,
    showCategoryFilter,
    searchQuery,
    activeFilter,
    onSelectCategories,
  ]);

  // Load tags for visible emails
  const loadTags = useTagStore((s) => s.loadTags);
  useEffect(() => {
    if (filteredEmails.length === 0) return;
    const ids = filteredEmails.map((e) => e.id);
    void useJunkStore.getState().loadVerdicts(ids);
    void loadTags(ids);
  }, [filteredEmails, loadTags]);

  // Auto-select first visible email only when nothing is selected (split layout only)
  useEffect(() => {
    if (disableAutoSelect || filteredEmails.length === 0 || isLoading || selectedEmailId) return;
    onSelectEmail(filteredEmails[0]);
  }, [disableAutoSelect, filteredEmails, selectedEmailId, isLoading, onSelectEmail]);

  // Check if we need to load more (content doesn't fill container)
  const checkIfNeedMoreEmails = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container || isLoadingMore || !hasMore || isLoading) return;

    const { scrollHeight, clientHeight } = container;
    // If content doesn't fill the container (no scrollbar), load more
    if (scrollHeight <= clientHeight + 10) {
      onLoadMore();
    }
  }, [isLoadingMore, hasMore, isLoading, onLoadMore]);

  // Infinite scroll handler
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container || isLoadingMore || !hasMore) return;

    const { scrollTop, scrollHeight, clientHeight } = container;
    const distanceFromBottom = scrollHeight - scrollTop - clientHeight;

    // Load more when user scrolls to within 200px of the bottom
    if (distanceFromBottom < 200) {
      onLoadMore();
    }
  }, [isLoadingMore, hasMore, onLoadMore]);

  // Attach scroll listener
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (container) {
      container.addEventListener('scroll', handleScroll);
      return () => container.removeEventListener('scroll', handleScroll);
    }
  }, [handleScroll]);

  // Auto-load more if content doesn't fill the container
  useEffect(() => {
    // Small delay to let the DOM render first
    const timeoutId = setTimeout(checkIfNeedMoreEmails, 150);
    return () => clearTimeout(timeoutId);
  }, [checkIfNeedMoreEmails]);

  // Show loading only on initial load when we have no emails
  if (isLoading && emails.length === 0) {
    return (
      <div className={`${widthClass} border-r border-gray-200 bg-white flex items-center justify-center`}>
        <div className="text-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600 mx-auto"></div>
          <p className="mt-2 text-sm text-gray-500">{t('inbox:loadingEmails')}</p>
        </div>
      </div>
    );
  }

  // Decide which empty-state message to show in the list area.
  // The header (title + search + categories) stays visible in all cases so the
  // user can edit their search or switch categories even when the current
  // query/filter yields zero results.
  const hasActiveFilter = Boolean(searchQuery) || Boolean(activeFilter);
  const hasCategoryFilter =
    showCategoryFilter && selectedCategories.size > 0 && selectedCategories.size < visibleCategories.length;
  const emptyStateMessage: string = isSyncing
    ? 'Syncing emails...'
    : searchQuery
      ? 'No emails match your search'
      : hasActiveFilter || hasCategoryFilter
        ? 'No emails match the selected filters'
        : 'No emails yet — try syncing or adding an account';

  const showTabs = shouldShowCategoryTabs(showCategoryFilter, visibleCategories.length);
  return (
    <div className={`${widthClass} border-r border-gray-200 bg-gradient-to-b from-white to-gray-50/30 flex flex-col`}>
      <div className={`px-4 pt-4 ${showTabs ? '' : 'pb-4'} border-b border-gray-200 bg-white`}>
        {/* Phones put the search field on its own full-width row above the
            title, the way Gmail does; `md` and up keeps the original single
            row of [title | search | actions]. */}
        <div className="flex flex-col md:flex-row md:items-center gap-2">
          {/* Title. Hidden when stacked: the app's top bar already titles the
              view there, and repeating it costs a row of a phone screen. */}
          {!isStacked && (
            <div className="flex items-center gap-1.5 flex-shrink-0 max-w-full md:max-w-[45%] min-w-0">
              <h2 className="text-lg font-semibold text-gray-900 truncate">
                {accountName ? `Inbox — ${accountName}` : 'Inbox'}
              </h2>
              {isSyncing && (
                <div className="animate-spin rounded-full h-3.5 w-3.5 border-b-2 border-primary-600 flex-shrink-0" />
              )}
            </div>
          )}

          {/* Search and the toolbar buttons share one row on a phone — the
              title above them is hidden there, so two rows would leave the
              buttons alone on a line of their own. `md:contents` dissolves
              this wrapper from `md` up, leaving the desktop row untouched. */}
          <div className="order-first md:order-none flex w-full items-center gap-2 min-w-0 md:contents">
            {/* Inline search — centered, ~50ch wide.
                Only shown in full-width layout. In split layout the lateral
                search bar is used instead, so we hide this to avoid duplication. */}
            {fullWidth && (
              <div className="flex-1 md:flex-1 flex justify-center min-w-0">
                <InboxSearchBox
                  accountId={isUnified ? autocompleteAccountId : accountId}
                  externalQuery={searchQuery ?? ''}
                  onSubmit={(q) => onSearch?.(q)}
                  onClear={clearSearchQuery}
                />
              </div>
            )}

            {/* Action buttons */}
            <div className="flex items-center gap-1 flex-shrink-0">
              {/* Hidden when stacked: the list already loads the next page on
                scroll, and a button competing with the search field for a
                phone's toolbar row buys nothing. */}
              {hasMore && !isLoadingMore && !isStacked && (
                <button
                  onClick={onLoadMore}
                  className="px-3 py-1.5 text-xs font-medium text-primary-600 hover:text-primary-700 hover:bg-primary-50 border border-primary-200 rounded-lg transition-colors"
                >
                  {t('inbox:loadMore')}
                </button>
              )}
              {isLoadingMore && (
                <div className="flex items-center gap-2 text-xs text-gray-500">
                  <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-primary-600"></div>
                  {t('common:state.loading')}
                </div>
              )}
              {/* Always-visible new-chat affordance. Sits in the list toolbar
                rather than the AI FEATURES sidebar section so starting a chat
                never depends on that section being expanded or scrolled into
                view. Starts a fresh conversation and docks the panel. */}
              {onNewChat && (
                <button
                  onClick={onNewChat}
                  title={t('chat:panel.newChat')}
                  aria-label={t('chat:panel.newChat')}
                  className="p-1.5 rounded text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors"
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                    />
                  </svg>
                </button>
              )}
              {onCollapse && (
                <button
                  onClick={onCollapse}
                  title={t('inbox:collapse')}
                  className="p-1.5 rounded text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors"
                >
                  <svg className="h-4 w-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
                    <path d="M10 3L5 8l5 5" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                </button>
              )}
            </div>
          </div>
        </div>

        {/* Category tabs — Gmail only. Single-select (clicking a tab replaces
            the active filter rather than toggling). When more than one category
            is available we prepend an "All" tab so users can clear the filter
            without clicking through every category. */}
        {showTabs && (
          <div
            role="tablist"
            aria-label={t('inbox:categoriesAria')}
            className="mt-3 -mx-4 px-4 flex items-stretch gap-1 overflow-x-auto border-b border-transparent"
          >
            {visibleCategories.length > 1 && (
              <CategoryTab
                isActive={activeTabKey === 'all'}
                onClick={() => handleTabClick('all')}
                activeColor="text-primary-600 border-primary-600"
                label={t('inbox:allCategories')}
                icon={
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M4 6h16M4 12h16M4 18h16" />
                  </svg>
                }
              />
            )}
            {visibleCategories.map(({ key, label, activeColor, icon }) => (
              <CategoryTab
                key={key}
                isActive={activeTabKey === key}
                onClick={() => handleTabClick(key)}
                activeColor={activeColor}
                label={label}
                icon={icon}
              />
            ))}
          </div>
        )}
      </div>
      {flaggedCount > 0 && !searchQuery && !activeFilter && (
        <label className="flex items-center gap-2 px-4 py-1.5 text-xs text-gray-500 border-b border-gray-100 cursor-pointer select-none">
          <input
            type="checkbox"
            className="rounded border-gray-300"
            checked={junkFlaggedAction === 'hide'}
            onChange={(e) => void setJunkFlaggedAction(e.target.checked ? 'hide' : 'dim')}
          />
          <span>{t('inbox:junk.hideFlagged')}</span>
        </label>
      )}
      <VirtualEmailList
        emails={visibleEmails}
        selectedEmailId={selectedEmailId}
        focusEmailId={focusEmailId}
        scrollContainerRef={scrollContainerRef}
        isLoadingMore={isLoadingMore}
        hasMore={hasMore}
        isSyncing={isSyncing}
        emptyStateMessage={emptyStateMessage}
        onSelectEmail={onSelectEmail}
        onLoadMore={onLoadMore}
        onAddSenderFilter={onAddSenderFilter}
        onBlockSender={onBlockSender}
        onCreateAttachmentRule={onCreateAttachmentRule}
        onCreateClassificationRule={onCreateClassificationRule}
        onOpenInTab={onOpenInTab}
        onChatAboutThread={onChatAboutThread}
        compact={fullWidth}
        getAccountBadge={getAccountBadge}
      />
    </div>
  );
}

interface CategoryTabProps {
  isActive: boolean;
  onClick: () => void;
  /** Tailwind classes applied to text + border-bottom when active. */
  activeColor: string;
  label: string;
  icon: ReactNode;
}

function CategoryTab({ isActive, onClick, activeColor, label, icon }: CategoryTabProps) {
  return (
    <button
      role="tab"
      aria-selected={isActive}
      onClick={onClick}
      className={`group relative flex items-center gap-1.5 px-3 py-2 text-xs font-medium border-b-2 -mb-px transition-colors whitespace-nowrap ${
        isActive ? `${activeColor} bg-white` : 'text-gray-500 border-transparent hover:text-gray-800 hover:bg-gray-50'
      }`}
    >
      <span className={isActive ? '' : 'text-gray-400 group-hover:text-gray-600'}>{icon}</span>
      <span>{label}</span>
    </button>
  );
}
