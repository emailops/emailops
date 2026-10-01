import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { ArchiveIcon, ClockIcon, InboxIcon, StarIcon } from '@/components/common/MailIcons';
import { SnoozeOptions } from '@/components/Inbox/SnoozePicker';
import type { MailboxView } from '@/lib/api';
import * as api from '@/lib/api';
import { computeDropdownTop } from '@/lib/dropdownPosition';
import { folderLabel } from '@/lib/folderDisplay';
import { useAccountStore } from '@/stores/accountStore';
import { isSnoozed, threadRefOf, useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { useSenderStore } from '@/stores/senderStore';
import type { Email } from '@/types';

export interface RulePrefill {
  senderEmail: string;
  subject: string;
  senderName: string;
}

export interface EmailActionsMenuProps {
  email: Email;
  onAddSenderFilter?: (senderEmail: string) => void;
  onHideSenderFromFilters?: (senderEmail: string) => void;
  onCreateAttachmentRule?: (prefill: RulePrefill) => void;
  onCreateClassificationRule?: (prefill: RulePrefill) => void;
  onOpenInTab?: (email: Email) => void;
  /** Open a new chat session seeded with this email's cleaned thread. */
  onChatAboutThread?: (email: Email) => void;
  /** Short outcome text ("Copied", "Download failed") for the host to show
   *  next to the row; null clears it. */
  onStatus: (message: string | null) => void;
}

/**
 * Where an IMAP message may be moved to. Move-to-folder is IMAP-only and
 * applies to inbox/custom-folder messages. The folder store carries the
 * *active* account's folders, so the picker is only offered when they match
 * this email's account (single-account views).
 */
export function useMoveTargets(email: Email): {
  canMove: boolean;
  moveTargets: { label: string; mailbox: MailboxView }[];
} {
  const { t } = useTranslation(['inbox']);
  // Move-to-folder is IMAP-only and applies to inbox/custom-folder messages.
  // The folder store carries the *active* account's folders, so the picker is
  // only offered when they match this email's account (single-account views).
  const emailAccount = useAccountStore((s) => s.accounts.find((a) => a.id === email.accountId));
  const { folders: accountFolders, accountId: foldersAccountId } = useFolderStore();
  const canMove =
    emailAccount?.provider === 'imap' && (email.mailbox === 'inbox' || email.mailbox.startsWith('folder:'));
  const moveTargets: { label: string; mailbox: MailboxView }[] =
    canMove && foldersAccountId === email.accountId
      ? [
          ...(email.mailbox !== 'inbox'
            ? [{ label: t('inbox:emailRow.moveToInbox'), mailbox: 'inbox' as MailboxView }]
            : []),
          ...accountFolders
            .filter((f) => `folder:${f.serverPath}` !== email.mailbox)
            .map((f) => ({
              label: folderLabel(f.displayName, f.delimiter),
              mailbox: `folder:${f.serverPath}` as MailboxView,
            })),
        ]
      : [];
  return { canMove, moveTargets };
}

/**
 * The ⋮ menu of an email row: chat about the thread, open in a tab, sender
 * filters, rules, copy id, redownload, move, delete. Extracted from `EmailRow`
 * so the tag board's cards offer the same actions as the inbox rather than a
 * different subset.
 */
export function EmailActionsMenu({
  email,
  onAddSenderFilter,
  onHideSenderFromFilters,
  onCreateAttachmentRule,
  onCreateClassificationRule,
  onOpenInTab,
  onChatAboutThread,
  onStatus,
}: EmailActionsMenuProps) {
  const { t } = useTranslation(['inbox']);
  const updateEmail = useEmailStore((s) => s.updateEmail);
  const deleteThreads = useEmailStore((s) => s.deleteThreads);
  const moveEmailFromStore = useEmailStore((s) => s.moveEmail);
  const setThreadsRead = useEmailStore((s) => s.setThreadsRead);
  const setThreadsStarred = useEmailStore((s) => s.setThreadsStarred);
  const archiveThreads = useEmailStore((s) => s.archiveThreads);
  const moveThreadsToInbox = useEmailStore((s) => s.moveThreadsToInbox);
  const snoozeThreads = useEmailStore((s) => s.snoozeThreads);
  const unsnoozeThreads = useEmailStore((s) => s.unsnoozeThreads);
  const snoozed = useEmailStore((s) => isSnoozed(s.snoozes, email));
  const openSenderDialog = useSenderStore((s) => s.openDialog);
  const { moveTargets } = useMoveTargets(email);
  const [menuOpen, setMenuOpen] = useState(false);
  /** 'move' shows the folder-picker page of the menu, 'snooze' the snooze
   *  times. */
  const [menuView, setMenuView] = useState<'main' | 'move' | 'snooze'>('main');
  const [menuPos, setMenuPos] = useState<{ top: number; right: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuDropdownRef = useRef<HTMLDivElement>(null);
  const menuBtnRef = useRef<HTMLButtonElement>(null);

  // Close menu on outside click
  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      const inButton = menuRef.current?.contains(target);
      const inDropdown = menuDropdownRef.current?.contains(target);
      if (!inButton && !inDropdown) {
        setMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [menuOpen]);

  // The menu first renders below the button (estimate set on click), then —
  // before paint — is measured and repositioned so it never clips past the
  // bottom of the window: flipped above the button, or clamped as a last
  // resort. Height isn't known until the portal is in the DOM because most
  // menu items are conditional on the handler props.
  // biome-ignore lint/correctness/useExhaustiveDependencies: menuView isn't read in the callback, but switching between the main and move pages changes the menu's height and requires a re-measure
  useLayoutEffect(() => {
    if (!menuOpen) return;
    const btnRect = menuBtnRef.current?.getBoundingClientRect();
    const menuHeight = menuDropdownRef.current?.getBoundingClientRect().height;
    if (!btnRect || !menuHeight) return;
    const top = computeDropdownTop({
      anchorTop: btnRect.top,
      anchorBottom: btnRect.bottom,
      menuHeight,
      viewportHeight: window.innerHeight,
    });
    setMenuPos((pos) => (pos && pos.top !== top ? { ...pos, top } : pos));
  }, [menuOpen, menuView]);

  return (
    <div ref={menuRef} className="relative flex-shrink-0">
      <button
        ref={menuBtnRef}
        onClick={(e) => {
          e.stopPropagation();
          if (!menuOpen && menuBtnRef.current) {
            const rect = menuBtnRef.current.getBoundingClientRect();
            setMenuPos({ top: rect.bottom + 4, right: window.innerWidth - rect.right });
          }
          setMenuView('main');
          setMenuOpen(!menuOpen);
        }}
        className="p-1 text-gray-300 hover:text-gray-500 rounded hover:bg-gray-100 transition-colors"
        title={t('inbox:emailRow.moreActions')}
        aria-label={t('inbox:emailRow.moreActions')}
      >
        <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 20 20">
          <path d="M10 6a2 2 0 110-4 2 2 0 010 4zM10 12a2 2 0 110-4 2 2 0 010 4zM10 18a2 2 0 110-4 2 2 0 010 4z" />
        </svg>
      </button>
      {menuOpen &&
        menuPos &&
        createPortal(
          <div
            ref={menuDropdownRef}
            className="fixed w-56 bg-white rounded-lg shadow-lg border border-gray-200 py-1 z-[100] max-h-[calc(100vh-8px)] overflow-y-auto"
            style={{ top: menuPos.top, right: menuPos.right }}
          >
            {menuView === 'snooze' ? (
              <>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuView('main');
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-500 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                  </svg>
                  {t('inbox:snooze.back')}
                </button>
                <div className="border-t border-gray-100 my-1" />
                <SnoozeOptions
                  onPick={(until) => {
                    setMenuOpen(false);
                    void snoozeThreads([threadRefOf(email)], until);
                  }}
                />
              </>
            ) : menuView === 'move' ? (
              <>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuView('main');
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-500 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                  </svg>
                  {t('inbox:emailRow.moveToFolder')}
                </button>
                <div className="border-t border-gray-100 my-1" />
                {moveTargets.map((target) => (
                  <button
                    key={target.mailbox}
                    onClick={async (e) => {
                      e.stopPropagation();
                      setMenuOpen(false);
                      try {
                        await moveEmailFromStore(email.accountId, email.id, target.mailbox);
                      } catch {
                        onStatus(t('inbox:emailRow.moveFailed'));
                      }
                    }}
                    title={target.label}
                    className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                  >
                    <svg
                      className="w-4 h-4 text-gray-400 shrink-0"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      {target.mailbox === 'inbox' ? (
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"
                        />
                      ) : (
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
                        />
                      )}
                    </svg>
                    <span className="truncate">{target.label}</span>
                  </button>
                ))}
              </>
            ) : (
              <>
                {onChatAboutThread && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onChatAboutThread(email);
                      setMenuOpen(false);
                    }}
                    className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                  >
                    <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                      />
                    </svg>
                    {t('inbox:emailRow.chatAboutThread')}
                  </button>
                )}
                {onOpenInTab && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpenInTab(email);
                      setMenuOpen(false);
                    }}
                    className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                  >
                    <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                      />
                    </svg>
                    {t('inbox:emailRow.openInNewTab')}
                  </button>
                )}
                {(onChatAboutThread || onOpenInTab) && <div className="border-t border-gray-100 my-1" />}
                <ThreadActionItems
                  email={email}
                  onPick={(run) => {
                    setMenuOpen(false);
                    void run();
                  }}
                  onRead={(read) => setThreadsRead([threadRefOf(email)], read)}
                  onStar={(starred) => setThreadsStarred([threadRefOf(email)], starred)}
                  onArchive={() => archiveThreads([threadRefOf(email)])}
                  onMoveToInbox={() => moveThreadsToInbox([threadRefOf(email)])}
                  snoozed={snoozed}
                  onSnooze={() => setMenuView('snooze')}
                  onUnsnooze={() => unsnoozeThreads([threadRefOf(email)])}
                />
                <div className="border-t border-gray-100 my-1" />
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    onAddSenderFilter?.(email.senderEmail);
                    setMenuOpen(false);
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z"
                    />
                  </svg>
                  {t('inbox:emailRow.addSenderFilter')}
                  <span className="ml-auto text-xs text-gray-400 truncate max-w-[120px]">{email.senderEmail}</span>
                </button>
                {onHideSenderFromFilters && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onHideSenderFromFilters(email.senderEmail);
                      setMenuOpen(false);
                    }}
                    className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                  >
                    <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"
                      />
                    </svg>
                    {t('inbox:emailRow.hideFromFilters')}
                  </button>
                )}
                {!email.isSent && (
                  <button
                    data-testid="menu-block-sender"
                    onClick={(e) => {
                      e.stopPropagation();
                      openSenderDialog({ type: 'block', accountId: email.accountId, address: email.senderEmail });
                      setMenuOpen(false);
                    }}
                    className="w-full text-left px-3 py-2 text-sm text-red-600 hover:bg-red-50 flex items-center gap-2"
                  >
                    <svg className="w-4 h-4 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636"
                      />
                    </svg>
                    {t('inbox:emailRow.blockSender')}
                    <span className="ml-auto text-xs text-red-400 truncate max-w-[120px]">{email.senderEmail}</span>
                  </button>
                )}
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    onCreateAttachmentRule?.({
                      senderEmail: email.senderEmail,
                      subject: email.subject,
                      senderName: email.sender,
                    });
                    setMenuOpen(false);
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
                    />
                  </svg>
                  {t('inbox:emailRow.createAttachmentRule')}
                </button>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    onCreateClassificationRule?.({
                      senderEmail: email.senderEmail,
                      subject: email.subject,
                      senderName: email.sender,
                    });
                    setMenuOpen(false);
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M7 7h.01M7 3h5c.512 0 1.024.195 1.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.994 1.994 0 013 12V7a4 4 0 014-4z"
                    />
                  </svg>
                  {t('inbox:emailRow.createClassificationRule')}
                </button>
                <button
                  onClick={async (e) => {
                    e.stopPropagation();
                    try {
                      await navigator.clipboard.writeText(email.id);
                      onStatus('Copied');
                    } catch {
                      onStatus('Copy failed');
                    }
                    setMenuOpen(false);
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                    />
                  </svg>
                  {t('inbox:emailRow.copyEmailId')}
                  <span className="ml-auto text-xs text-gray-400 truncate max-w-[120px]">
                    {email.id.slice(0, 12)}...
                  </span>
                </button>
                <button
                  onClick={async (e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    onStatus('Downloading...');
                    try {
                      const updated = await api.redownloadEmail(email.id);
                      updateEmail(updated);
                      onStatus('Downloaded');
                    } catch {
                      onStatus('Download failed');
                    }
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
                    />
                  </svg>
                  {t('inbox:emailRow.redownloadEmail')}
                </button>
                {moveTargets.length > 0 && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenuView('move');
                    }}
                    className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
                  >
                    <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
                      />
                    </svg>
                    {t('inbox:emailRow.moveToFolder')}
                    <svg
                      className="w-3 h-3 ml-auto text-gray-400"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                    </svg>
                  </button>
                )}
                <div className="border-t border-gray-100 my-1" />
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    // The row leaves at once; the provider call waits out the
                    // undo window, and a refusal brings the row back with a
                    // toast (emailStore.deleteThreads).
                    void deleteThreads([threadRefOf(email)]);
                  }}
                  className="w-full text-left px-3 py-2 text-sm text-red-600 hover:bg-red-50 flex items-center gap-2 disabled:opacity-50"
                >
                  <svg className="w-4 h-4 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                    />
                  </svg>
                  {t('inbox:emailRow.deleteThread')}
                </button>
              </>
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}

interface ThreadActionItemsProps {
  email: Email;
  /** Close the menu and run the chosen action. */
  onPick: (run: () => Promise<void>) => void;
  onRead: (read: boolean) => Promise<void>;
  onStar: (starred: boolean) => Promise<void>;
  onArchive: () => Promise<void>;
  onMoveToInbox: () => Promise<void>;
  /** The row's conversation is snoozed (the Snoozed view lists it). */
  snoozed: boolean;
  /** Open the snooze-times page of the menu. */
  onSnooze: () => void;
  onUnsnooze: () => Promise<void>;
}

/** Read/unread, star, snooze (or unsnooze) and archive (or its inverse, move
 *  to inbox) for the row's conversation. Errors are reported by the store
 *  (toast + log). */
function ThreadActionItems({
  email,
  onPick,
  onRead,
  onStar,
  onArchive,
  onMoveToInbox,
  snoozed,
  onSnooze,
  onUnsnooze,
}: ThreadActionItemsProps) {
  const { t } = useTranslation(['inbox']);
  const itemClass = 'w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2';
  return (
    <>
      <button
        onClick={(e) => {
          e.stopPropagation();
          onPick(() => onRead(!email.isRead));
        }}
        className={itemClass}
      >
        <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
          />
        </svg>
        {email.isRead ? t('inbox:emailRow.markAsUnread') : t('inbox:emailRow.markAsRead')}
      </button>
      <button
        onClick={(e) => {
          e.stopPropagation();
          onPick(() => onStar(!email.isStarred));
        }}
        className={itemClass}
      >
        <StarIcon filled={email.isStarred} className="w-4 h-4 text-gray-400" />
        {email.isStarred ? t('inbox:emailRow.unstar') : t('inbox:emailRow.star')}
      </button>
      {snoozed ? (
        <button
          data-testid="row-unsnooze"
          onClick={(e) => {
            e.stopPropagation();
            onPick(onUnsnooze);
          }}
          className={itemClass}
        >
          <ClockIcon className="w-4 h-4 text-gray-400" />
          {t('inbox:snooze.unsnooze')}
        </button>
      ) : (
        email.mailbox === 'inbox' && (
          <button
            data-testid="row-snooze"
            onClick={(e) => {
              e.stopPropagation();
              onSnooze();
            }}
            className={itemClass}
          >
            <ClockIcon className="w-4 h-4 text-gray-400" />
            {t('inbox:snooze.button')}
          </button>
        )
      )}
      {email.mailbox === 'inbox' && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onPick(onArchive);
          }}
          className={itemClass}
        >
          <ArchiveIcon className="w-4 h-4 text-gray-400" />
          {t('inbox:emailRow.archive')}
        </button>
      )}
      {email.mailbox === 'archive' && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onPick(onMoveToInbox);
          }}
          className={itemClass}
        >
          <InboxIcon className="w-4 h-4 text-gray-400" />
          {t('inbox:emailRow.moveArchivedToInbox')}
        </button>
      )}
    </>
  );
}
