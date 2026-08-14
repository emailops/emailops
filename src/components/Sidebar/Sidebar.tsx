import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { currentPlatform, type Folder, type MailboxView } from '@/lib/api';
import { isEmailDrag, readEmailDragPayload } from '@/lib/emailDrag';
import { errorText } from '@/lib/errors';
import type { FeedbackType } from '@/lib/feedback';
import { folderLabel } from '@/lib/folderDisplay';
import { formatShortcut } from '@/lib/platform';
import { ALL_ACCOUNTS_ID } from '@/stores/accountStore';
import { useAiStore } from '@/stores/aiStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { useLensStore } from '@/stores/lensStore';
import { useLogStore } from '@/stores/logStore';
import { useMemoryStore } from '@/stores/memoryStore';
import { useUpdateStore } from '@/stores/updateStore';
import type { Account, ActiveFilter, SmartFilter } from '@/types';
import { FeedbackMenu } from './FeedbackMenu';
import { SmartFilters } from './SmartFilters';
import { VersionLabel } from './VersionLabel';

function CollapseChevron({ open }: { open: boolean }) {
  return (
    <svg
      className={`w-3 h-3 transition-transform ${open ? 'rotate-0' : '-rotate-90'}`}
      fill="currentColor"
      viewBox="0 0 20 20"
    >
      <path
        fillRule="evenodd"
        d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export type ViewMode =
  | 'inbox'
  | 'attachments'
  | 'contacts'
  | 'drafts'
  | 'sent'
  | 'spam'
  | 'deleted'
  | 'calendar'
  | 'chat'
  | 'tasks'
  | 'memory'
  | 'lenses'
  | 'dashboard'
  | `folder:${string}`;

interface SidebarProps {
  accounts: Account[];
  activeAccount: Account | null;
  /** True when the unified "All accounts" pseudo-entry is selected. */
  isUnifiedActive: boolean;
  onSelectAccount: (accountId: string) => void;
  onAddAccount: () => void;
  onMoveAccountUp: (accountId: string) => void;
  onMoveAccountDown: (accountId: string) => void;
  onToggleAccountEnabled: (accountId: string, enabled: boolean) => void;
  onOpenAccountSettings: (accountId: string) => void;
  onSync: () => void;
  onOpenSearch: () => void;
  onCompose: () => void;
  onGiveFeedback: (type: FeedbackType) => void;
  onOpenAppSettings: () => void;
  isSyncing: boolean;
  viewMode: ViewMode;
  onSetViewMode: (mode: ViewMode) => void;

  /** Open the full-page chat view. */
  onOpenChatView: () => void;
  smartFilters: SmartFilter[];
  activeFilter: ActiveFilter | null;
  isLoadingFilters: boolean;
  onToggleFilter: (filter: ActiveFilter) => void;
  onClearFilter: () => void;
  onPinFilter: (filter: ActiveFilter) => void;
  onUnpinFilter: (filter: ActiveFilter) => void;
  onRemoveFilter: (filter: ActiveFilter) => void;
  onRefreshFilters: () => void;
  isFilterPinned: (filter: ActiveFilter) => boolean;
  tasksEnabled: boolean;
  memoriesEnabled: boolean;
  lensesEnabled: boolean;
  /** True when at least one account has calendar integration enabled
   *  (Settings → Calendar) — the Calendar entry is hidden otherwise. */
  calendarEnabled: boolean;
  onSelectLens: (lensId: string) => void;
}

export function Sidebar({
  accounts,
  activeAccount,
  isUnifiedActive,
  onSelectAccount,
  onAddAccount,
  onMoveAccountUp,
  onMoveAccountDown,
  onToggleAccountEnabled,
  onOpenAccountSettings,
  onSync,
  onOpenSearch,
  onCompose,
  onGiveFeedback,
  onOpenAppSettings,
  isSyncing,
  viewMode,
  onSetViewMode,
  onOpenChatView,
  smartFilters,
  activeFilter,
  isLoadingFilters,
  onToggleFilter,
  onClearFilter,
  onPinFilter,
  onUnpinFilter,
  onRemoveFilter,
  onRefreshFilters,
  isFilterPinned,
  tasksEnabled,
  memoriesEnabled,
  lensesEnabled,
  calendarEnabled,
  onSelectLens,
}: SidebarProps) {
  const { counts } = useMemoryStore();
  const tasksBadge = counts.totalOpen + counts.awaitingThem;
  // Master AI switch — when disabled, hide Chat / Tasks / Memory entries entirely.
  // Tasks and Memory are *also* gated on their per-feature experimental flags;
  // both must be on for the entry to appear. Dashboard stays visible (it shows
  // general account stats, not AI output).
  const { enabled: aiEnabled } = useAiStore();
  // Lenses sub-list under the "Lenses" entry. We load the list once when AI is
  // enabled so the sidebar shows them even before the user opens the Lenses view.
  const { lenses, activeLensId, initialize: initializeLenses } = useLensStore();

  const { t } = useTranslation(['common', 'sidebar', 'chat']);
  const [accountsOpen, setAccountsOpen] = useState(true);
  const [viewsOpen, setViewsOpen] = useState(true);
  const [aiFeaturesOpen, setAiFeaturesOpen] = useState(true);
  const [otherViewsOpen, setOtherViewsOpen] = useState(false);
  const [foldersOpen, setFoldersOpen] = useState(true);
  const [lensesListOpen, setLensesListOpen] = useState(true);

  useEffect(() => {
    if (!aiEnabled || !lensesEnabled) return;
    void initializeLenses();
  }, [aiEnabled, lensesEnabled, initializeLenses]);

  // Custom IMAP folders of the selected account. Account-specific by design,
  // so the unified view clears the list (fetchFolders(null)).
  const { folders, fetchFolders, createFolder, renameFolder, deleteFolder } = useFolderStore();
  const folderAccountId = activeAccount?.id ?? null;
  useEffect(() => {
    void fetchFolders(folderAccountId);
  }, [folderAccountId, fetchFolders]);
  // Re-fetch when a sync finishes — that's when new folders appear.
  const wasSyncing = useRef(isSyncing);
  useEffect(() => {
    if (wasSyncing.current && !isSyncing) {
      void fetchFolders(folderAccountId);
    }
    wasSyncing.current = isSyncing;
  }, [isSyncing, folderAccountId, fetchFolders]);

  // Folder management + drag-and-drop move targets (IMAP accounts only).
  const isImapAccount = activeAccount?.provider === 'imap';
  const { addLog } = useLogStore();
  // Persistent new-release link in the footer — unlike the update toast it
  // stays until the user actually upgrades (store self-heals post-upgrade).
  const availableUpdate = useUpdateStore((s) => s.available);
  const moveEmail = useEmailStore((s) => s.moveEmail);
  const [addingFolder, setAddingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');
  const [editingFolderId, setEditingFolderId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState('');
  const [confirmDeleteFolder, setConfirmDeleteFolder] = useState<Folder | null>(null);
  const [folderBusy, setFolderBusy] = useState(false);
  /** Folder id (or 'inbox') currently hovered by an email drag. */
  const [dragOverTarget, setDragOverTarget] = useState<string | null>(null);

  const submitCreateFolder = async () => {
    const name = newFolderName.trim();
    if (!name || !activeAccount || folderBusy) return;
    setFolderBusy(true);
    try {
      await createFolder(activeAccount.id, name);
      setAddingFolder(false);
      setNewFolderName('');
    } catch (e) {
      addLog('error', 'sync', errorText(e));
    } finally {
      setFolderBusy(false);
    }
  };

  const submitRenameFolder = async (folder: Folder) => {
    const name = editingName.trim();
    if (!activeAccount || folderBusy) return;
    if (!name || name === folderLabel(folder.displayName, folder.delimiter)) {
      setEditingFolderId(null);
      return;
    }
    setFolderBusy(true);
    try {
      const renamed = await renameFolder(activeAccount.id, folder.id, name);
      // Follow the rename when the user is looking at this folder.
      if (viewMode === `folder:${folder.serverPath}`) {
        onSetViewMode(`folder:${renamed.serverPath}`);
      }
      setEditingFolderId(null);
    } catch (e) {
      addLog('error', 'sync', errorText(e));
    } finally {
      setFolderBusy(false);
    }
  };

  const submitDeleteFolder = async () => {
    if (!confirmDeleteFolder || !activeAccount || folderBusy) return;
    setFolderBusy(true);
    try {
      await deleteFolder(activeAccount.id, confirmDeleteFolder.id);
      if (viewMode === `folder:${confirmDeleteFolder.serverPath}`) {
        onSetViewMode('inbox');
      }
      setConfirmDeleteFolder(null);
    } catch (e) {
      addLog('error', 'sync', errorText(e));
    } finally {
      setFolderBusy(false);
    }
  };

  /** dragover must preventDefault to become a legal drop target. */
  const handleEmailDragOver = (targetKey: string) => (e: React.DragEvent) => {
    if (!isImapAccount || !isEmailDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverTarget(targetKey);
  };

  const handleEmailDrop = (targetLabel: string, targetMailbox: MailboxView) => async (e: React.DragEvent) => {
    e.preventDefault();
    setDragOverTarget(null);
    if (!activeAccount || !isImapAccount) return;
    const payload = readEmailDragPayload(e.dataTransfer);
    if (!payload || payload.accountId !== activeAccount.id || payload.mailbox === targetMailbox) return;
    try {
      await moveEmail(activeAccount.id, payload.emailId, targetMailbox);
      addLog('success', 'sync', t('sidebar:folderActions.movedTo', { name: targetLabel }));
    } catch (err) {
      addLog('error', 'sync', errorText(err));
    }
  };

  const clearDragOver = (targetKey: string) => () => {
    setDragOverTarget((current) => (current === targetKey ? null : current));
  };

  return (
    <aside className="w-64 bg-gray-900 text-white flex flex-col">
      <div className="p-4 border-b border-gray-700">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-bold">{t('sidebar:appName')}</h1>
            <VersionLabel />
          </div>
          <div className="flex items-center gap-1">
            {(activeAccount || isUnifiedActive) && (
              <button
                onClick={onSync}
                disabled={isSyncing}
                className="p-2 text-gray-400 hover:text-white hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50"
                title={t('sidebar:accountActions.syncEmails')}
              >
                <svg
                  className={`w-4 h-4 ${isSyncing ? 'animate-spin' : ''}`}
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                  />
                </svg>
              </button>
            )}
          </div>
        </div>

        {/* Compose Button */}
        <button
          onClick={onCompose}
          className="mt-3 w-full flex items-center gap-2 px-3 py-2 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors"
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          <span>{t('sidebar:compose')}</span>
        </button>

        {/* Search Button */}
        <button
          onClick={onOpenSearch}
          className="mt-3 w-full flex items-center gap-2 px-3 py-2 text-sm text-gray-400 bg-gray-800 hover:bg-gray-700 rounded-lg transition-colors"
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
            />
          </svg>
          <span>{t('sidebar:search')}</span>
          {/* Rendered rather than translated: the shortcut is the same in every
              language, but differs per platform (⌘K on macOS, Ctrl+K elsewhere). */}
          <kbd className="ml-auto text-xs bg-gray-700 px-1.5 py-0.5 rounded">
            {formatShortcut(currentPlatform(), 'K')}
          </kbd>
        </button>

        {/* Give Feedback */}
        <FeedbackMenu onSelect={onGiveFeedback} />
      </div>

      <nav className="flex-1 p-4 space-y-6 overflow-y-auto">
        <section>
          <div className="flex items-center justify-between mb-2">
            <button
              onClick={() => setAccountsOpen((v) => !v)}
              className="flex items-center gap-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wider hover:text-gray-300"
            >
              <CollapseChevron open={accountsOpen} />
              {t('sidebar:accounts')}
            </button>
            <button
              onClick={onAddAccount}
              className="p-0.5 text-gray-500 hover:text-primary-400 hover:bg-gray-800 rounded transition-colors"
              title={t('sidebar:addAccount')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
              </svg>
            </button>
          </div>
          {accountsOpen &&
            (accounts.length === 0 ? (
              <p className="text-sm text-gray-500">{t('sidebar:noAccounts')}</p>
            ) : (
              <ul className="space-y-0.5">
                {accounts.length > 1 && (
                  <li>
                    <div
                      className={`flex items-center rounded-lg text-sm transition-colors ${
                        isUnifiedActive ? 'bg-primary-600 text-white' : 'text-gray-300 hover:bg-gray-800'
                      }`}
                    >
                      <button
                        onClick={() => onSelectAccount(ALL_ACCOUNTS_ID)}
                        className="flex-1 text-left px-3 py-1.5 min-w-0 flex items-center gap-1.5"
                      >
                        <svg
                          className="w-3.5 h-3.5 flex-shrink-0"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                          strokeWidth={2}
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"
                          />
                        </svg>
                        <span className="block truncate text-xs">{t('sidebar:allAccounts')}</span>
                      </button>
                    </div>
                  </li>
                )}
                {accounts.map((account, idx) => (
                  <AccountItem
                    key={account.id}
                    account={account}
                    isActive={activeAccount?.id === account.id}
                    isFirst={idx === 0}
                    isLast={idx === accounts.length - 1}
                    onSelect={() => onSelectAccount(account.id)}
                    onMoveUp={() => onMoveAccountUp(account.id)}
                    onMoveDown={() => onMoveAccountDown(account.id)}
                    onToggleEnabled={() => onToggleAccountEnabled(account.id, !account.enabled)}
                    onOpenSettings={() => onOpenAccountSettings(account.id)}
                  />
                ))}
              </ul>
            ))}
        </section>

        {/* Views */}
        <section>
          <button
            onClick={() => setViewsOpen((v) => !v)}
            className="flex items-center gap-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2 hover:text-gray-300"
          >
            <CollapseChevron open={viewsOpen} />
            {t('sidebar:views')}
          </button>
          {viewsOpen && (
            <ul className="space-y-1">
              <li>
                <button
                  onClick={() => onSetViewMode('inbox')}
                  onDragOver={handleEmailDragOver('inbox')}
                  onDragLeave={clearDragOver('inbox')}
                  onDrop={handleEmailDrop(t('sidebar:inbox'), 'inbox')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'inbox' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  } ${dragOverTarget === 'inbox' ? 'ring-1 ring-blue-400 bg-gray-800' : ''}`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"
                    />
                  </svg>
                  {t('sidebar:inbox')}
                </button>
              </li>
              <li>
                <button
                  onClick={() => onSetViewMode('attachments')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'attachments' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
                    />
                  </svg>
                  {t('sidebar:attachments')}
                </button>
              </li>
              <li>
                <button
                  onClick={() => onSetViewMode('drafts')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'drafts' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                    />
                  </svg>
                  {t('sidebar:drafts')}
                </button>
              </li>
              <li>
                <button
                  onClick={() => onSetViewMode('sent')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'sent' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8"
                    />
                  </svg>
                  {t('sidebar:sent')}
                </button>
              </li>
              {calendarEnabled && (
                <li>
                  <button
                    onClick={() => onSetViewMode('calendar')}
                    className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                      viewMode === 'calendar' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                    }`}
                  >
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"
                      />
                    </svg>
                    {t('sidebar:calendar')}
                  </button>
                </li>
              )}
            </ul>
          )}
        </section>

        {/* Other Views */}
        <section>
          <button
            onClick={() => setOtherViewsOpen((v) => !v)}
            className="flex items-center gap-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2 hover:text-gray-300"
          >
            <CollapseChevron open={otherViewsOpen} />
            {t('sidebar:otherViews')}
          </button>
          {otherViewsOpen && (
            <ul className="space-y-1">
              <li>
                <button
                  onClick={() => onSetViewMode('spam')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'spam' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                    />
                  </svg>
                  {t('sidebar:spam')}
                </button>
              </li>
              <li>
                <button
                  onClick={() => onSetViewMode('deleted')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'deleted' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6M1 7h22M8 7V4a1 1 0 011-1h6a1 1 0 011 1v3"
                    />
                  </svg>
                  {t('sidebar:deleted')}
                </button>
              </li>
              <li>
                <button
                  onClick={() => onSetViewMode('contacts')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'contacts' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z"
                    />
                  </svg>
                  {t('sidebar:contacts')}
                </button>
              </li>
              <li>
                <button
                  onClick={() => onSetViewMode('dashboard')}
                  className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                    viewMode === 'dashboard' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                  }`}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M9 17v-6h13M9 17H4a1 1 0 01-1-1V4a1 1 0 011-1h16a1 1 0 011 1v6M9 17v4m4-4v4m-8 0h16"
                    />
                  </svg>
                  {t('sidebar:dashboard')}
                </button>
              </li>
            </ul>
          )}
        </section>

        {/* Custom IMAP folders — account-specific, hidden in the unified view.
            IMAP accounts always get the section (with the "+" affordance) so
            the first folder can be created; other providers only when sync
            discovered folders. */}
        {activeAccount && (folders.length > 0 || isImapAccount) && (
          <section>
            <div className="flex items-center justify-between mb-2">
              <button
                onClick={() => setFoldersOpen((v) => !v)}
                className="flex items-center gap-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wider hover:text-gray-300"
              >
                <CollapseChevron open={foldersOpen} />
                {t('sidebar:folders')}
              </button>
              {isImapAccount && (
                <button
                  onClick={() => {
                    setFoldersOpen(true);
                    setAddingFolder(true);
                  }}
                  title={t('sidebar:folderActions.newFolder')}
                  className="p-1 rounded text-gray-400 hover:text-white hover:bg-gray-800 transition-colors"
                >
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                  </svg>
                </button>
              )}
            </div>
            {foldersOpen && (
              <ul className="space-y-1">
                {addingFolder && (
                  <li>
                    <div className="px-3 py-1.5">
                      <input
                        // biome-ignore lint/a11y/noAutofocus: input appears on explicit user action ("+")
                        autoFocus
                        value={newFolderName}
                        onChange={(e) => setNewFolderName(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') void submitCreateFolder();
                          if (e.key === 'Escape') {
                            setAddingFolder(false);
                            setNewFolderName('');
                          }
                        }}
                        placeholder={t('sidebar:folderActions.namePlaceholder')}
                        disabled={folderBusy}
                        className="w-full bg-gray-800 border border-gray-600 rounded px-2 py-1 text-sm text-white placeholder-gray-500 focus:outline-none focus:border-blue-500 disabled:opacity-50"
                      />
                    </div>
                  </li>
                )}
                {folders.map((folder) => {
                  const mode: ViewMode = `folder:${folder.serverPath}`;
                  const label = folderLabel(folder.displayName, folder.delimiter);
                  if (editingFolderId === folder.id) {
                    return (
                      <li key={folder.id}>
                        <div className="px-3 py-1.5">
                          <input
                            // biome-ignore lint/a11y/noAutofocus: input appears on explicit user action (rename)
                            autoFocus
                            value={editingName}
                            onChange={(e) => setEditingName(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === 'Enter') void submitRenameFolder(folder);
                              if (e.key === 'Escape') setEditingFolderId(null);
                            }}
                            disabled={folderBusy}
                            className="w-full bg-gray-800 border border-gray-600 rounded px-2 py-1 text-sm text-white focus:outline-none focus:border-blue-500 disabled:opacity-50"
                          />
                        </div>
                      </li>
                    );
                  }
                  return (
                    <li key={folder.id} className="relative group">
                      <button
                        onClick={() => onSetViewMode(mode)}
                        title={folder.displayName}
                        onDragOver={handleEmailDragOver(folder.id)}
                        onDragLeave={clearDragOver(folder.id)}
                        onDrop={handleEmailDrop(label, mode)}
                        className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                          viewMode === mode ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                        } ${dragOverTarget === folder.id ? 'ring-1 ring-blue-400 bg-gray-800' : ''}`}
                      >
                        <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
                          />
                        </svg>
                        <span className="truncate">{label}</span>
                      </button>
                      {isImapAccount && (
                        <div className="absolute right-1.5 top-1/2 -translate-y-1/2 hidden group-hover:flex items-center gap-0.5 bg-gray-800/95 rounded">
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              setEditingFolderId(folder.id);
                              setEditingName(label);
                            }}
                            title={t('sidebar:folderActions.rename')}
                            className="p-1 rounded hover:bg-gray-700 text-gray-400 hover:text-gray-200"
                          >
                            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                              />
                            </svg>
                          </button>
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              setConfirmDeleteFolder(folder);
                            }}
                            title={t('common:actions.delete')}
                            className="p-1 rounded hover:bg-gray-700 text-gray-400 hover:text-red-400"
                          >
                            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6M1 7h22M8 7V4a1 1 0 011-1h6a1 1 0 011 1v3"
                              />
                            </svg>
                          </button>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        )}

        {/* AI Features */}
        <section>
          <button
            onClick={() => setAiFeaturesOpen((v) => !v)}
            className="flex items-center gap-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2 hover:text-gray-300"
          >
            <CollapseChevron open={aiFeaturesOpen} />
            {t('sidebar:aiFeatures')}
          </button>
          {aiFeaturesOpen && (
            <ul className="space-y-1 mb-2">
              {aiEnabled && (
                <li>
                  {/* Navigates to the full-page chat view. The docked panel is
                      a separate surface with its own close button and is
                      reopened from the header's new-chat icon, so this entry
                      does not toggle it. */}
                  <button
                    onClick={onOpenChatView}
                    aria-pressed={viewMode === 'chat'}
                    title={t('chat:panel.expand')}
                    className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                      viewMode === 'chat' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                    }`}
                  >
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                      />
                    </svg>
                    {t('sidebar:chat')}
                  </button>
                </li>
              )}
              {aiEnabled && tasksEnabled && (
                <li>
                  <button
                    onClick={() => onSetViewMode('tasks')}
                    className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                      viewMode === 'tasks' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                    }`}
                  >
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4"
                      />
                    </svg>
                    <span className="flex-1">{t('sidebar:tasks')}</span>
                    {tasksBadge > 0 && (
                      <span
                        className={`px-1.5 py-0.5 rounded text-[10px] font-semibold ${
                          counts.overdue > 0 ? 'bg-red-600 text-white' : 'bg-gray-600 text-gray-100'
                        }`}
                      >
                        {tasksBadge}
                      </span>
                    )}
                  </button>
                </li>
              )}
              {aiEnabled && memoriesEnabled && (
                <li>
                  <button
                    onClick={() => onSetViewMode('memory')}
                    className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 ${
                      viewMode === 'memory' ? 'bg-gray-700 text-white' : 'text-gray-300 hover:bg-gray-800'
                    }`}
                  >
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                      />
                    </svg>
                    {t('sidebar:memory')}
                  </button>
                </li>
              )}
              {aiEnabled && lensesEnabled && (
                <li>
                  <div
                    className={`group flex items-center rounded-lg text-sm transition-colors ${
                      viewMode === 'lenses' && !activeLensId
                        ? 'bg-gray-700 text-white'
                        : 'text-gray-300 hover:bg-gray-800'
                    }`}
                  >
                    <button
                      onClick={() => onSetViewMode('lenses')}
                      className="flex-1 flex items-center gap-2 px-3 py-2 text-left min-w-0"
                    >
                      <svg className="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M3 10h18M3 6h18M3 14h18M3 18h18"
                        />
                      </svg>
                      <span className="flex-1">{t('sidebar:lenses')}</span>
                      {lenses.length > 0 && <span className="text-xs text-gray-500">{lenses.length}</span>}
                    </button>
                    {lenses.length > 0 && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          setLensesListOpen((v) => !v);
                        }}
                        className="px-2 py-2 text-gray-400 hover:text-gray-200 flex-shrink-0"
                        title={lensesListOpen ? t('sidebar:collapseLenses') : t('sidebar:expandLenses')}
                      >
                        <CollapseChevron open={lensesListOpen} />
                      </button>
                    )}
                  </div>
                  {lensesListOpen && lenses.length > 0 && (
                    <ul className="mt-0.5 ml-3 space-y-0.5 border-l border-gray-700 pl-2">
                      {lenses.map((l) => {
                        const isActive = viewMode === 'lenses' && activeLensId === l.id;
                        return (
                          <li key={l.id}>
                            <button
                              onClick={() => onSelectLens(l.id)}
                              className={`w-full text-left px-2 py-1 rounded text-xs truncate transition-colors ${
                                isActive
                                  ? 'bg-primary-600 text-white'
                                  : 'text-gray-400 hover:bg-gray-800 hover:text-gray-200'
                              }`}
                              title={l.name}
                            >
                              {l.name}
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </li>
              )}
            </ul>
          )}
        </section>

        <SmartFilters
          filters={smartFilters}
          activeFilter={activeFilter}
          isLoading={isLoadingFilters}
          onToggleFilter={onToggleFilter}
          onClearFilter={onClearFilter}
          onPinFilter={onPinFilter}
          onUnpinFilter={onUnpinFilter}
          onRemoveFilter={onRemoveFilter}
          onRefresh={onRefreshFilters}
          isPinned={isFilterPinned}
        />
      </nav>

      <div className="p-4 border-t border-gray-700">
        <div className="flex items-center justify-between">
          <div className="text-xs text-gray-500 truncate">
            {isUnifiedActive ? (
              <span>{t('sidebar:allAccountsFooter')}</span>
            ) : activeAccount ? (
              <span>{t('sidebar:signedInAs', { email: activeAccount.email })}</span>
            ) : (
              <span>{t('sidebar:noAccountSelected')}</span>
            )}
          </div>
          <button
            onClick={onOpenAppSettings}
            className="ml-2 flex-shrink-0 p-1.5 text-gray-500 hover:text-gray-300 hover:bg-gray-800 rounded transition-colors"
            title={t('sidebar:appSettings')}
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
              />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
          </button>
        </div>
        {isSyncing && <div className="mt-2 text-xs text-primary-400">{t('sidebar:syncingEmails')}</div>}
        {availableUpdate && (
          <button
            type="button"
            onClick={() => {
              void openExternal(availableUpdate.url).catch((e) => {
                addLog('error', 'system', `Failed to open release page: ${errorText(e)}`);
              });
            }}
            className="mt-2 block text-left text-xs text-primary-400 hover:text-primary-300 hover:underline truncate w-full"
            title={availableUpdate.url}
          >
            {t('sidebar:updateAvailable', { version: availableUpdate.version })}
          </button>
        )}
      </div>

      {/* Folder-delete confirmation — deleting removes the emails on the
          server too, so this is never a silent one-click action. */}
      {confirmDeleteFolder && (
        <div
          className="fixed inset-0 z-[60] bg-black/60 flex items-center justify-center"
          onClick={() => !folderBusy && setConfirmDeleteFolder(null)}
        >
          <div
            className="bg-[#2d2d2e] border border-gray-600 rounded-lg p-4 w-80 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="text-sm font-semibold text-gray-100 mb-1">{t('sidebar:folderActions.deleteTitle')}</p>
            <p className="text-xs text-gray-400 mb-4">
              {t('sidebar:folderActions.deleteConfirm', {
                name: folderLabel(confirmDeleteFolder.displayName, confirmDeleteFolder.delimiter),
              })}
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConfirmDeleteFolder(null)}
                disabled={folderBusy}
                className="px-3 py-1.5 text-xs rounded bg-gray-700 text-gray-200 hover:bg-gray-600 disabled:opacity-50"
              >
                {t('common:actions.cancel')}
              </button>
              <button
                onClick={() => void submitDeleteFolder()}
                disabled={folderBusy}
                className="px-3 py-1.5 text-xs rounded bg-red-600 text-white hover:bg-red-500 disabled:opacity-50"
              >
                {t('common:actions.delete')}
              </button>
            </div>
          </div>
        </div>
      )}
    </aside>
  );
}

function AccountItem({
  account,
  isActive,
  isFirst,
  isLast,
  onSelect,
  onMoveUp,
  onMoveDown,
  onToggleEnabled,
  onOpenSettings,
}: {
  account: Account;
  isActive: boolean;
  isFirst: boolean;
  isLast: boolean;
  onSelect: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onToggleEnabled: () => void;
  onOpenSettings: () => void;
}) {
  const { t } = useTranslation(['sidebar']);
  const [hovered, setHovered] = useState(false);

  return (
    <li onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)}>
      <div
        className={`flex items-center rounded-lg text-sm transition-colors ${
          isActive
            ? 'bg-primary-600 text-white'
            : account.enabled
              ? 'text-gray-300 hover:bg-gray-800'
              : 'text-gray-500 hover:bg-gray-800 opacity-50'
        }`}
      >
        <button onClick={onSelect} className="flex-1 text-left px-3 py-1.5 min-w-0">
          <span className="block truncate text-xs">
            {account.email}
            {!account.enabled && (
              <span className={`ml-1 ${isActive ? 'text-primary-200' : 'text-gray-500'}`}>
                {t('sidebar:accountDisabledSuffix')}
              </span>
            )}
          </span>
        </button>
        {hovered && (
          <div className="flex items-center gap-0.5 pr-1 flex-shrink-0">
            {!isFirst && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onMoveUp();
                }}
                className="p-0.5 rounded hover:bg-gray-700 text-gray-400 hover:text-gray-200"
                title={t('sidebar:accountActions.moveUp')}
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 15l7-7 7 7" />
                </svg>
              </button>
            )}
            {!isLast && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onMoveDown();
                }}
                className="p-0.5 rounded hover:bg-gray-700 text-gray-400 hover:text-gray-200"
                title={t('sidebar:accountActions.moveDown')}
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                </svg>
              </button>
            )}
            <button
              onClick={(e) => {
                e.stopPropagation();
                onToggleEnabled();
              }}
              className="p-0.5 rounded hover:bg-gray-700 text-gray-400 hover:text-gray-200"
              title={account.enabled ? 'Disable account' : 'Enable account'}
            >
              {account.enabled ? (
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                  />
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
                  />
                </svg>
              ) : (
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"
                  />
                </svg>
              )}
            </button>
            <button
              onClick={(e) => {
                e.stopPropagation();
                onOpenSettings();
              }}
              className="p-0.5 rounded hover:bg-gray-700 text-gray-400 hover:text-gray-200"
              title={t('sidebar:accountActions.openSettings')}
            >
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
                />
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                />
              </svg>
            </button>
          </div>
        )}
      </div>
    </li>
  );
}
