import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArchiveIcon, InboxIcon, StarIcon } from '@/components/common/MailIcons';
import type { MailboxView } from '@/lib/api';
import { bulkAvailability, bulkMoveTargets } from '@/lib/bulkActions';
import { folderLabel } from '@/lib/folderDisplay';
import { useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { selectedThreads, useSelectionStore } from '@/stores/selectionStore';
import type { Email } from '@/types';

interface BulkToolbarProps {
  /** The rows the list shows, in order — what "select all" selects. */
  emails: Email[];
}

/**
 * Replaces the list header while rows are selected: the count, a header
 * checkbox (all loaded rows / none) and the actions over the selected
 * conversations. Archive and delete wait out the undo window (the store
 * shows the Undo toast); failures come back as one toast from the store.
 */
export function BulkToolbar({ emails }: BulkToolbarProps) {
  const { t } = useTranslation(['inbox']);
  const ids = useSelectionStore((s) => s.ids);
  const selectAll = useSelectionStore((s) => s.selectAll);
  const clear = useSelectionStore((s) => s.clear);
  const archiveThreads = useEmailStore((s) => s.archiveThreads);
  const deleteThreads = useEmailStore((s) => s.deleteThreads);
  const setThreadsRead = useEmailStore((s) => s.setThreadsRead);
  const setThreadsStarred = useEmailStore((s) => s.setThreadsStarred);
  const moveEmailsToMailbox = useEmailStore((s) => s.moveEmailsToMailbox);
  const accounts = useAccountStore((s) => s.accounts);
  const folders = useFolderStore((s) => s.folders);
  const foldersAccountId = useFolderStore((s) => s.accountId);

  const rows = useMemo(() => emails.filter((e) => ids.has(e.id)), [emails, ids]);
  if (rows.length === 0) return null;

  const threads = selectedThreads({ ids, anchor: null }, emails);
  const can = bulkAvailability(rows);
  const move = bulkMoveTargets(rows, accounts, folders, foldersAccountId);
  const allSelected = rows.length === emails.length;

  /** Archive, delete and move take the rows out of the list: the selection
   *  goes with them. Flag changes keep it, so another action can follow. */
  const leaving = (run: () => Promise<void>) => () => {
    clear();
    void run();
  };

  return (
    <div
      data-testid="bulk-toolbar"
      role="toolbar"
      aria-label={t('inbox:bulk.toolbarAria')}
      className="flex items-center gap-1 min-w-0 flex-wrap"
    >
      <SelectAllCheckbox
        checked={allSelected}
        indeterminate={!allSelected}
        label={t('inbox:bulk.selectAll')}
        onToggle={() => (allSelected ? clear() : selectAll(emails.map((e) => e.id)))}
      />
      <span className="text-sm font-medium text-gray-900 mr-2 whitespace-nowrap">
        {t('inbox:bulk.selected', { count: rows.length })}
      </span>
      {can.canArchive && (
        <ToolbarButton
          testId="bulk-archive"
          label={t('inbox:bulk.archive')}
          onClick={leaving(() => archiveThreads(threads))}
        >
          <ArchiveIcon className="w-4 h-4" />
        </ToolbarButton>
      )}
      <ToolbarButton
        testId="bulk-delete"
        label={t('inbox:bulk.delete')}
        onClick={leaving(() => deleteThreads(threads))}
        danger
      >
        <TrashIcon />
      </ToolbarButton>
      {can.canMarkRead && (
        <ToolbarButton
          testId="bulk-mark-read"
          label={t('inbox:bulk.markRead')}
          onClick={() => void setThreadsRead(threads, true)}
        >
          <EnvelopeOpenIcon />
        </ToolbarButton>
      )}
      {can.canMarkUnread && (
        <ToolbarButton
          testId="bulk-mark-unread"
          label={t('inbox:bulk.markUnread')}
          onClick={() => void setThreadsRead(threads, false)}
        >
          <EnvelopeIcon />
        </ToolbarButton>
      )}
      {can.canStar && (
        <ToolbarButton
          testId="bulk-star"
          label={t('inbox:bulk.star')}
          onClick={() => void setThreadsStarred(threads, true)}
        >
          <StarIcon filled={false} className="w-4 h-4" />
        </ToolbarButton>
      )}
      {can.canUnstar && (
        <ToolbarButton
          testId="bulk-unstar"
          label={t('inbox:bulk.unstar')}
          onClick={() => void setThreadsStarred(threads, false)}
        >
          <StarIcon filled className="w-4 h-4 text-amber-400" />
        </ToolbarButton>
      )}
      {move && move.mailboxes.length > 0 && (
        <MoveMenu
          mailboxes={move.mailboxes}
          labelOf={(mailbox) => {
            if (mailbox === 'inbox') return t('inbox:emailRow.moveToInbox');
            const folder = folders.find((f) => `folder:${f.serverPath}` === mailbox);
            return folder ? folderLabel(folder.displayName, folder.delimiter) : mailbox;
          }}
          onPick={(mailbox) =>
            leaving(() =>
              moveEmailsToMailbox(
                move.accountId,
                rows.map((e) => e.id),
                mailbox,
              ),
            )()
          }
        />
      )}
      <button
        type="button"
        data-testid="bulk-clear"
        onClick={clear}
        className="ml-auto px-2 py-1 text-xs font-medium text-gray-600 hover:text-gray-900 hover:bg-gray-100 rounded transition-colors whitespace-nowrap"
      >
        {t('inbox:bulk.clear')}
      </button>
    </div>
  );
}

function SelectAllCheckbox({
  checked,
  indeterminate,
  label,
  onToggle,
}: {
  checked: boolean;
  indeterminate: boolean;
  label: string;
  onToggle: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={ref}
      type="checkbox"
      data-testid="bulk-select-all"
      className="w-4 h-4 mr-1 rounded border-gray-300 text-primary-600 cursor-pointer"
      checked={checked}
      onChange={onToggle}
      title={label}
      aria-label={label}
    />
  );
}

function ToolbarButton({
  testId,
  label,
  onClick,
  danger = false,
  children,
}: {
  testId: string;
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      title={label}
      aria-label={label}
      className={`p-1.5 rounded transition-colors text-gray-500 hover:bg-gray-100 ${
        danger ? 'hover:text-red-600 hover:bg-red-50' : 'hover:text-gray-800'
      }`}
    >
      {children}
    </button>
  );
}

function MoveMenu({
  mailboxes,
  labelOf,
  onPick,
}: {
  mailboxes: MailboxView[];
  labelOf: (mailbox: MailboxView) => string;
  onPick: (mailbox: MailboxView) => void;
}) {
  const { t } = useTranslation(['inbox']);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <ToolbarButton testId="bulk-move" label={t('inbox:bulk.moveToFolder')} onClick={() => setOpen(!open)}>
        <FolderIcon />
      </ToolbarButton>
      {open && (
        <div className="absolute left-0 top-full mt-1 w-56 max-h-72 overflow-y-auto bg-white rounded-lg shadow-lg border border-gray-200 py-1 z-50">
          {mailboxes.map((mailbox) => (
            <button
              key={mailbox}
              type="button"
              data-testid={`bulk-move-${mailbox}`}
              onClick={() => {
                setOpen(false);
                onPick(mailbox);
              }}
              title={labelOf(mailbox)}
              className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2"
            >
              {mailbox === 'inbox' ? (
                <InboxIcon className="w-4 h-4 text-gray-400 shrink-0" />
              ) : (
                <FolderIcon className="w-4 h-4 text-gray-400 shrink-0" />
              )}
              <span className="truncate">{labelOf(mailbox)}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function TrashIcon() {
  return (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
      />
    </svg>
  );
}

function EnvelopeIcon() {
  return (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
      />
    </svg>
  );
}

function EnvelopeOpenIcon() {
  return (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M3 19v-8.93a2 2 0 01.89-1.664l7-4.666a2 2 0 012.22 0l7 4.666A2 2 0 0121 10.07V19M3 19a2 2 0 002 2h14a2 2 0 002-2M3 19l6.75-4.5M21 19l-6.75-4.5M3 10l6.75 4.5M21 10l-6.75 4.5m0 0l-1.14.76a2 2 0 01-2.22 0l-1.14-.76"
      />
    </svg>
  );
}

function FolderIcon({ className = 'w-4 h-4' }: { className?: string }) {
  return (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
      />
    </svg>
  );
}
