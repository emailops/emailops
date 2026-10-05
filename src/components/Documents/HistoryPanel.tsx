import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { CellConflict } from '@/lib/sheetConflicts';
import { columnLabel } from '@/lib/sheetModel';
import { useLogStore } from '@/stores/logStore';
import { useSharedDocsStore } from '@/stores/sharedDocsStore';
import type { DocVersion } from '@/types';

interface HistoryPanelProps {
  accountId: string;
  docId: string;
  /** This account's own address: its versions read "You". */
  me: string;
  selectedId: number | null;
  onSelect: (version: DocVersion | null) => void;
  onClose: () => void;
  /** A sheet's cells changed by two people at once (none for a text document). */
  conflicts?: CellConflict[];
}

/**
 * The right-hand panel listing a document's versions, newest first: who made
 * each change and when. Selecting one shows it read-only in place of the
 * editor; history is for looking back, nothing here changes the document.
 */
export function HistoryPanel({
  accountId,
  docId,
  me,
  selectedId,
  onSelect,
  onClose,
  conflicts = [],
}: HistoryPanelProps) {
  const { t } = useTranslation(['documents']);
  const fmt = useFormatters();
  const addLog = useLogStore((s) => s.addLog);
  // Changes that arrive while the panel is open add a version: reload then.
  const change = useSharedDocsStore((s) => s.changes[docId] ?? 0);
  const [versions, setVersions] = useState<DocVersion[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listSharedDocVersions(accountId, docId)
      .then((v) => {
        if (!cancelled) setVersions(v);
      })
      .catch((err) => {
        const msg = errorText(err);
        if (!cancelled) setError(msg);
        addLog('error', 'sync', `The document history could not be loaded: ${msg}`);
      });
    return () => {
      cancelled = true;
    };
  }, [accountId, docId, change, addLog]);

  return (
    <aside
      className="w-72 flex-shrink-0 border-l border-gray-700 flex flex-col min-h-0"
      data-testid="shared-doc-history"
    >
      <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-700">
        <h3 className="flex-1 text-sm font-semibold text-gray-200">{t('documents:history.title')}</h3>
        <button
          type="button"
          onClick={onClose}
          title={t('documents:history.close')}
          aria-label={t('documents:history.close')}
          className="p-1 rounded text-gray-400 hover:text-white hover:bg-gray-700"
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
      {error && <p className="m-3 p-2 rounded bg-red-900/30 border border-red-800 text-red-300 text-xs">{error}</p>}
      {conflicts.length > 0 && (
        <section className="border-b border-gray-700 px-4 py-2">
          <h4 className="mb-1 text-[11px] font-semibold uppercase text-gray-500">{t('documents:conflicts.title')}</h4>
          <ul className="flex flex-col gap-1">
            {conflicts.map((c) => (
              <li key={c.id} data-testid="shared-doc-conflict" className="text-xs text-gray-300">
                {t('documents:conflicts.item', {
                  cell: `${columnLabel(c.col)}${c.row + 1}`,
                  lost: c.lost,
                  kept: c.kept,
                })}
                <span className={`ml-1 ${c.resolved ? 'text-gray-500' : 'text-amber-300'}`}>
                  {c.resolved ? t('documents:conflicts.resolved') : t('documents:conflicts.pending')}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <ul className="flex-1 overflow-y-auto py-1">
        {versions?.length === 0 && <li className="px-4 py-3 text-xs text-gray-500">{t('documents:history.empty')}</li>}
        {versions?.map((v) => (
          <li key={v.id} className={selectedId === v.id ? 'bg-gray-700/60' : 'hover:bg-gray-800'}>
            <button
              type="button"
              data-testid={`shared-doc-version-${v.id}`}
              onClick={() => onSelect(selectedId === v.id ? null : v)}
              className="w-full text-left px-4 py-2"
            >
              <span className="block text-sm text-gray-200">{fmt.dateTime(v.createdAt)}</span>
              <span className="block text-xs text-gray-400 truncate">
                {v.author === me ? t('documents:history.you') : v.author}
              </span>
              <span className="block text-[11px] text-gray-500">
                {v.origin === 'remote' ? t('documents:history.remote') : t('documents:history.local')}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}
