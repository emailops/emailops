import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import type { MailboxView } from '@/lib/api';
import * as api from '@/lib/api';
import { bulkDelete, bulkMove, canBulkMove } from '@/lib/bulkActions';
import { errorText } from '@/lib/errors';
import { folderLabel } from '@/lib/folderDisplay';
import { useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { useLogStore } from '@/stores/logStore';
import type { Email } from '@/types';

interface MultiSelectBarProps {
  /** The selected emails, in selection order. */
  emails: Email[];
  /** Every visible email is selected (the ✓ button then deselects them). */
  allSelected: boolean;
  /** ✓: select every visible email, or deselect them all when they already are. */
  onToggleAll: () => void;
  onClear: () => void;
  /** Receives the bar's delete action, so the Delete key can trigger it. */
  registerDelete?: (run: (() => void) | null) => void;
}

/**
 * Shown above the list while two or more emails are selected: the count and
 * the actions that apply to all of them. Delete moves each thread to the
 * Trash (same as the row menu); Move is offered when the selection belongs to
 * one IMAP account and sits in the inbox or a folder (same rule as the row's
 * "Move to folder").
 */
export function MultiSelectBar({ emails, allSelected, onToggleAll, onClear, registerDelete }: MultiSelectBarProps) {
  const { t } = useTranslation(['inbox']);
  const addLog = useLogStore((s) => s.addLog);
  const deleteEmail = useEmailStore((s) => s.deleteEmail);
  const moveEmail = useEmailStore((s) => s.moveEmail);
  const accounts = useAccountStore((s) => s.accounts);
  const { folders, accountId: foldersAccountId } = useFolderStore();
  const [busy, setBusy] = useState<'delete' | 'move' | null>(null);

  const count = emails.length;
  const provider = (accountId: string) => accounts.find((a) => a.id === accountId)?.provider;
  const movable = canBulkMove(emails, provider) && foldersAccountId === emails[0]?.accountId;
  const moveTargets: { value: MailboxView; label: string }[] = movable
    ? [
        { value: 'inbox' as MailboxView, label: t('inbox:emailRow.moveToInbox') },
        ...folders.map((f) => ({
          value: `folder:${f.serverPath}` as MailboxView,
          label: folderLabel(f.displayName, f.delimiter),
        })),
      ]
    : [];

  const deps = { getThread: api.getThread, deleteEmail, moveEmail };

  const report = (failed: { error: string }[]) => {
    if (failed.length > 0) {
      addLog('error', 'sync', t('inbox:multiSelect.failed', { count: failed.length, error: failed[0].error }));
    }
  };

  const handleDelete = async () => {
    setBusy('delete');
    try {
      const result = await bulkDelete(emails, deps);
      if (result.done > 0) addLog('success', 'sync', t('inbox:multiSelect.deleted', { count: result.done }));
      report(result.failed);
      onClear();
    } catch (err) {
      addLog('error', 'sync', errorText(err));
    } finally {
      setBusy(null);
    }
  };

  // The Delete key runs the same action as the button (once, while not busy).
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const handleDeleteRef = useRef(handleDelete);
  handleDeleteRef.current = handleDelete;
  useEffect(() => {
    registerDelete?.(() => {
      if (busyRef.current === null) void handleDeleteRef.current();
    });
    return () => registerDelete?.(null);
  }, [registerDelete]);

  const handleMove = async (choice: string) => {
    const target = moveTargets.find((m) => m.value === choice);
    if (!target) return;
    const name = target.label;
    setBusy('move');
    try {
      const result = await bulkMove(emails, target.value, deps);
      if (result.done > 0) addLog('success', 'sync', t('inbox:multiSelect.moved', { count: result.done, name }));
      report(result.failed);
      onClear();
    } catch (err) {
      addLog('error', 'sync', errorText(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    // One line at any column width: labels never wrap, the count truncates
    // first, and the secondary actions are icon buttons (with tooltips).
    <div
      role="toolbar"
      aria-label={t('inbox:multiSelect.selected', { count })}
      title={t('inbox:multiSelect.hint')}
      className="flex items-center gap-1 px-3 py-1.5 border-b border-primary-200 bg-primary-50 text-xs whitespace-nowrap"
    >
      <span className="font-medium text-primary-800 truncate min-w-0">
        {count === 0 ? t('inbox:multiSelect.noneSelected') : t('inbox:multiSelect.selected', { count })}
      </span>
      <div className="flex-1" />
      <button
        type="button"
        onClick={() => void handleDelete()}
        disabled={busy !== null || count === 0}
        className="flex-shrink-0 px-2 py-1 rounded text-red-700 hover:bg-red-100 disabled:opacity-50"
      >
        {busy === 'delete' ? t('inbox:multiSelect.deleting') : t('inbox:multiSelect.delete')}
      </button>
      {movable ? (
        <div className="flex-shrink-0">
          <Select
            value=""
            options={[
              { value: '', label: busy === 'move' ? t('inbox:multiSelect.moving') : t('inbox:multiSelect.move') },
              ...moveTargets,
            ]}
            onChange={(v) => void handleMove(v)}
            ariaLabel={t('inbox:multiSelect.move')}
            size="xs"
            variant="light"
            align="right"
            disabled={busy !== null || count === 0}
          />
        </div>
      ) : (
        <span className="flex-shrink-0 px-1 text-gray-400" title={t('inbox:multiSelect.moveUnavailable')}>
          {t('inbox:multiSelect.move')}
        </span>
      )}
      <button
        type="button"
        onClick={onToggleAll}
        disabled={busy !== null}
        aria-pressed={allSelected}
        title={allSelected ? t('inbox:multiSelect.deselectAll') : t('inbox:multiSelect.selectAll')}
        aria-label={allSelected ? t('inbox:multiSelect.deselectAll') : t('inbox:multiSelect.selectAll')}
        className={`flex-shrink-0 p-1 rounded disabled:opacity-50 ${
          allSelected ? 'bg-primary-600 text-white hover:bg-primary-700' : 'text-gray-600 hover:bg-gray-100'
        }`}
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4"
          />
        </svg>
      </button>
      <button
        type="button"
        onClick={onClear}
        disabled={busy !== null}
        title={t('inbox:multiSelect.clear')}
        aria-label={t('inbox:multiSelect.clear')}
        className="flex-shrink-0 p-1 rounded text-gray-600 hover:bg-gray-100 disabled:opacity-50"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
