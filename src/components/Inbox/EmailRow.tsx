import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { TagChips } from '@/components/common/TagChips';
import type { MailboxView } from '@/lib/api';
import * as api from '@/lib/api';
import { AVATAR_PALETTE, hashColorClass } from '@/lib/colors';
import { computeDropdownTop } from '@/lib/dropdownPosition';
import { writeEmailDragPayload } from '@/lib/emailDrag';
import { folderLabel } from '@/lib/folderDisplay';
import { useAccountStore } from '@/stores/accountStore';
import { useAiStore } from '@/stores/aiStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { useTagStore } from '@/stores/tagStore';
import type { Email, EmailCategory } from '@/types';

export interface RulePrefill {
  senderEmail: string;
  subject: string;
  senderName: string;
}

// Stable empty reference for the tag selector's missing-key fallback.
// zustand 5 dropped auto-shallow on selector results, so returning `|| []`
// inline produces a new array every render and trips React's
// useSyncExternalStore "getSnapshot should be cached" guard.
const EMPTY_TAGS: readonly string[] = Object.freeze([]);

interface EmailRowProps {
  email: Email;
  isSelected: boolean;
  onClick: () => void;
  onAddSenderFilter?: (senderEmail: string) => void;
  onBlockSender?: (senderEmail: string) => void;
  onCreateAttachmentRule?: (prefill: RulePrefill) => void;
  onCreateClassificationRule?: (prefill: RulePrefill) => void;
  onOpenInTab?: (email: Email) => void;
  /** Open a new chat session seeded with this email's cleaned thread. */
  onChatAboutThread?: (email: Email) => void;
  /** When true, render a single-line, Gmail-style compact row (used in full-width layout). */
  compact?: boolean;
  /** Unified ("All accounts") mode: colored left-edge bar identifying the
   *  email's account. Rendered absolutely so it never changes row height
   *  (the virtualizer depends on stable measured heights). */
  accountBadge?: { colorClass: string; label: string };
}

export function EmailRow({
  email,
  isSelected,
  onClick,
  onAddSenderFilter,
  onBlockSender,
  onCreateAttachmentRule,
  onCreateClassificationRule,
  onOpenInTab,
  onChatAboutThread,
  compact = false,
  accountBadge,
}: EmailRowProps) {
  const { t } = useTranslation(['inbox']);
  const receivedTime = formatReceptionTime(email.timestamp);
  const updateEmail = useEmailStore((s) => s.updateEmail);
  const deleteEmailFromStore = useEmailStore((s) => s.deleteEmail);
  // Hide classification chips when the master AI switch is off — the tags
  // remain in the DB (so toggling AI back on is lossless), but the user has
  // explicitly opted out of seeing AI-derived metadata in the inbox.
  const aiEnabled = useAiStore((s) => s.enabled);
  const storedTags = useTagStore((s) => s.tagsByEmail[email.id] || EMPTY_TAGS);
  // Exclude the company tag from the right-hand chip list — it's already
  // rendered as an uppercase prefix on the subject, so showing it again on
  // the tasks/tags side is just visual noise.
  const emailTags = aiEnabled ? storedTags.filter((t) => t.tagType !== 'company') : [];
  // Company tag is rendered as an uppercase chip prefix on the subject so the
  // user can scan which client/vendor a thread belongs to at a glance. We hide
  // it when the value contains '@' — that's the per-address shape produced by
  // `company_label_for` for personal-mail providers (gmail/outlook/yahoo/…),
  // where the address itself isn't a meaningful "company" badge. Only shown
  // when AI is enabled (consistent with the rest of the classification chips).
  const companyRaw = aiEnabled ? storedTags.find((t) => t.tagType === 'company')?.tagValue : undefined;
  const companyTag = companyRaw && !companyRaw.includes('@') ? companyRaw.toUpperCase() : undefined;
  const [isDeleting, setIsDeleting] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  /** 'move' shows the folder-picker page of the kebab menu. */
  const [menuView, setMenuView] = useState<'main' | 'move'>('main');
  const [menuPos, setMenuPos] = useState<{ top: number; right: number } | null>(null);
  const [copyMessage, setCopyMessage] = useState<string | null>(null);

  // Move-to-folder is IMAP-only and applies to inbox/custom-folder messages.
  // The folder store carries the *active* account's folders, so the picker is
  // only offered when they match this email's account (single-account views).
  const moveEmailFromStore = useEmailStore((s) => s.moveEmail);
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

  useEffect(() => {
    if (!copyMessage) return;
    const timeoutId = window.setTimeout(() => setCopyMessage(null), 2000);
    return () => window.clearTimeout(timeoutId);
  }, [copyMessage]);

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

  const kebabMenu = (
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
            {menuView === 'move' ? (
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
                        setCopyMessage(t('inbox:emailRow.moveFailed'));
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
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    onBlockSender?.(email.senderEmail);
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
                      setCopyMessage('Copied');
                    } catch {
                      setCopyMessage('Copy failed');
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
                    setCopyMessage('Downloading...');
                    try {
                      const updated = await api.redownloadEmail(email.id);
                      updateEmail(updated);
                      setCopyMessage('Downloaded');
                    } catch {
                      setCopyMessage('Download failed');
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
                  onClick={async (e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    setIsDeleting(true);
                    try {
                      const thread = await api.getThread(email.accountId, email.threadId);
                      for (const t of thread) {
                        await deleteEmailFromStore(t.id);
                      }
                    } catch {
                      setIsDeleting(false);
                    }
                  }}
                  disabled={isDeleting}
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

  // Unified-mode account indicator: absolutely positioned left-edge bar so it
  // adds zero height (virtualized rows must keep their measured height stable).
  const accountBar = accountBadge ? (
    <span
      className={`absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r ${accountBadge.colorClass}`}
      title={t('inbox:emailRow.accountTooltip', { email: accountBadge.label })}
      aria-hidden="true"
    />
  ) : null;

  if (compact) {
    return (
      <div
        role="button"
        tabIndex={0}
        className={`group relative hover:z-10 w-full text-left px-4 py-2 border-b border-gray-100 transition-colors cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-500 ${
          isSelected ? 'bg-primary-50/70 shadow-[inset_3px_0_0_0_theme(colors.primary.600)]' : 'hover:bg-gray-50'
        } ${!email.isRead && !isSelected ? 'bg-blue-50/40' : ''}`}
        onClick={onClick}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onClick();
          }
        }}
        draggable={canMove}
        onDragStart={(e) =>
          writeEmailDragPayload(e.dataTransfer, {
            emailId: email.id,
            accountId: email.accountId,
            mailbox: email.mailbox,
          })
        }
      >
        {accountBar}
        {/* Reserve a stable min-height so async tag/triage loading doesn't grow
            the row after measureElement has run — same fix as the non-compact
            branch, which prevents virtualizer translateY desync / overlap. */}
        <div className="flex items-center gap-3 min-w-0 min-h-[1.75rem]">
          <span
            className={`flex-shrink-0 w-1.5 h-1.5 rounded-full ${
              !email.isRead ? 'bg-primary-600 ring-2 ring-primary-100' : 'bg-transparent'
            }`}
            aria-hidden="true"
          />
          <Avatar name={email.sender} email={email.senderEmail} size="sm" />
          <span
            className={`text-sm truncate w-44 flex-shrink-0 ${
              email.isRead ? 'text-gray-700' : 'font-semibold text-gray-900'
            }`}
            title={email.sender}
          >
            {email.sender}
          </span>
          {email.category !== 'primary' && <CategoryBadge category={email.category} />}
          <div className="flex-1 min-w-0 flex items-baseline gap-2 text-sm">
            <span
              className={`truncate flex-shrink-0 max-w-[50%] ${
                email.isRead ? 'text-gray-800' : 'font-semibold text-gray-900'
              }`}
              title={
                companyTag ? `${companyTag} — ${email.subject || '(No subject)'}` : email.subject || '(No subject)'
              }
            >
              {companyTag && (
                <span className="inline-block rounded-full font-medium text-[11px] px-1.5 py-0 bg-slate-100 text-slate-700 mr-1.5 align-middle">
                  {companyTag}
                </span>
              )}
              {email.subject || '(No subject)'}
            </span>
            <span className="text-gray-500 truncate" title={email.snippet}>
              — {email.snippet}
            </span>
          </div>
          {/* Fixed height + nowrap so multi-tag rows can't wrap and grow the
              row past its measured height (virtualizer would lay subsequent
              rows on top of this one until ResizeObserver caught up). Excess
              chips are clipped horizontally — same trade-off Gmail makes. */}
          <div className="flex items-center gap-1 flex-shrink-0 h-6 max-w-[35%] overflow-hidden">
            {email.triageStatus && <TriageBadge status={email.triageStatus} />}
            {emailTags.length > 0 && <TagChips tags={emailTags} compact nowrap />}
          </div>
          <span className="text-xs text-gray-500 flex-shrink-0 w-24 text-right tabular-nums">{receivedTime}</span>
          {kebabMenu}
        </div>
        {copyMessage && <div className="mt-1 text-xs text-gray-500">{copyMessage}</div>}
      </div>
    );
  }

  return (
    <div
      role="button"
      tabIndex={0}
      className={`group relative hover:z-10 w-full text-left px-4 py-3 border-b border-gray-100 transition-colors cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-500 ${
        isSelected ? 'bg-primary-50/70 shadow-[inset_3px_0_0_0_theme(colors.primary.600)]' : 'hover:bg-gray-50'
      } ${!email.isRead && !isSelected ? 'bg-blue-50/50' : ''}`}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onClick();
        }
      }}
      draggable={canMove}
      onDragStart={(e) =>
        writeEmailDragPayload(e.dataTransfer, {
          emailId: email.id,
          accountId: email.accountId,
          mailbox: email.mailbox,
        })
      }
    >
      {accountBar}
      <div className="flex items-start gap-3">
        <Avatar name={email.sender} email={email.senderEmail} size="md" unread={!email.isRead} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className={`text-sm truncate ${email.isRead ? 'text-gray-700' : 'font-semibold text-gray-900'}`}>
              {email.sender}
            </span>
            {email.category !== 'primary' && <CategoryBadge category={email.category} />}
            <span className="ml-auto text-[11px] text-gray-500 flex-shrink-0 tabular-nums">{receivedTime}</span>
            {kebabMenu}
          </div>
          <h3
            className={`text-sm mt-0.5 truncate ${email.isRead ? 'text-gray-700' : 'font-semibold text-gray-900'}`}
            title={companyTag ? `${companyTag} — ${email.subject || '(No subject)'}` : undefined}
          >
            {companyTag && (
              <span className="inline-block rounded-full font-medium text-sm px-2 py-0.5 bg-slate-100 text-slate-700 mr-1.5 align-middle">
                {companyTag}
              </span>
            )}
            {email.subject || '(No subject)'}
          </h3>
          <p className="text-xs text-gray-500 mt-1 line-clamp-2 leading-relaxed">{email.snippet}</p>
          {/* Tag/triage row is always rendered with a *fixed* height (not min-h)
              and nowrap chips, so async tag loading (loadTags in Inbox.tsx) or a
              multi-chip email cannot change the row's measured height and desync
              the virtualizer's translateY offsets — which causes rows to visually
              overlap while the ResizeObserver catches up. Excess chips clip. */}
          <div className="mt-2 flex items-center gap-2 h-5 overflow-hidden">
            {email.triageStatus && <TriageBadge status={email.triageStatus} />}
            {emailTags.length > 0 && <TagChips tags={emailTags} compact nowrap />}
          </div>
        </div>
      </div>
      {copyMessage && <div className="mt-2 text-xs text-gray-500 pl-12">{copyMessage}</div>}
    </div>
  );
}

/** Deterministic color from a seed string so the same sender always renders with
 *  the same avatar color across the app. Shared hash lives in `@/lib/colors`. */
function avatarColor(seed: string): string {
  return hashColorClass(seed, AVATAR_PALETTE);
}

/** Strip leading non-letter chars (e.g. quotes, < ) so initials come from the
 *  actual name. Falls back to "?" for empty/unparseable input. */
function avatarInitial(name: string, fallback: string): string {
  const source = name?.trim() || fallback?.trim() || '';
  const match = source.match(/[\p{L}\p{N}]/u);
  return (match?.[0] ?? '?').toUpperCase();
}

interface AvatarProps {
  name: string;
  email: string;
  size: 'sm' | 'md';
  unread?: boolean;
}

function Avatar({ name, email, size, unread }: AvatarProps) {
  const color = avatarColor(email || name);
  const initial = avatarInitial(name, email);
  const sizeClasses = size === 'md' ? 'w-9 h-9 text-sm' : 'w-6 h-6 text-[11px]';
  return (
    <div
      className={`relative flex-shrink-0 ${sizeClasses} rounded-full ${color} text-white font-semibold flex items-center justify-center select-none`}
      aria-hidden="true"
    >
      <span>{initial}</span>
      {unread && size === 'md' && (
        <span className="absolute -top-0.5 -right-0.5 w-2.5 h-2.5 bg-primary-600 rounded-full ring-2 ring-white" />
      )}
    </div>
  );
}

function CategoryBadge({ category }: { category: EmailCategory }) {
  const config: Record<EmailCategory, { label: string; color: string }> = {
    primary: { label: 'Primary', color: 'bg-blue-100 text-blue-700' },
    social: { label: 'Social', color: 'bg-pink-100 text-pink-700' },
    updates: { label: 'Updates', color: 'bg-yellow-100 text-yellow-700' },
    forums: { label: 'Forums', color: 'bg-purple-100 text-purple-700' },
    promotions: { label: 'Promo', color: 'bg-green-100 text-green-700' },
  };

  const { label, color } = config[category] || config.primary;

  return <span className={`inline-block px-1.5 py-0.5 text-[10px] rounded ${color}`}>{label}</span>;
}

function TriageBadge({ status }: { status: Email['triageStatus'] }) {
  const config = {
    action_needed: { label: 'Action Needed', color: 'bg-red-100 text-red-800' },
    fyi: { label: 'FYI', color: 'bg-yellow-100 text-yellow-800' },
    low_priority: { label: 'Low Priority', color: 'bg-gray-100 text-gray-600' },
  };

  if (!status) return null;
  const { label, color } = config[status];

  return <span className={`inline-block px-2 py-0.5 text-xs rounded-full ${color}`}>{label}</span>;
}

/** Format a unix timestamp (seconds) as HH:MM for today's emails or DD/MM/YYYY for older ones. */
function formatReceptionTime(timestampSec: number): string {
  const d = new Date(timestampSec * 1000);
  const now = new Date();
  const isToday =
    d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  if (isToday) {
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  }
  return `${String(d.getDate()).padStart(2, '0')}/${String(d.getMonth() + 1).padStart(2, '0')}/${d.getFullYear()}`;
}
