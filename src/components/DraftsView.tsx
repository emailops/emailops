import { format } from 'date-fns';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import type { Account, Draft } from '@/types';

interface DraftsViewProps {
  accountId: string | null;
  accounts: Account[];
  onOpenComposeTab: (draft: Draft) => void;
}

export function DraftsView({ accountId, accounts, onOpenComposeTab }: DraftsViewProps) {
  const { t } = useTranslation(['compose']);
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const loadDrafts = useCallback(async () => {
    if (!accountId) {
      setDrafts([]);
      return;
    }
    setIsLoading(true);
    try {
      const result = await api.listDrafts(accountId);
      setDrafts(result);
    } catch (err) {
      console.error('Failed to load drafts:', err);
    } finally {
      setIsLoading(false);
    }
  }, [accountId]);

  useEffect(() => {
    void loadDrafts();
  }, [loadDrafts]);

  const handleDelete = async (draft: Draft) => {
    if (!accountId) return;
    setDeletingId(draft.id);
    try {
      await api.deleteDraft(draft.id, accountId);
      setDrafts((prev) => prev.filter((d) => d.id !== draft.id));
    } catch (err) {
      console.error('Failed to delete draft:', err);
    } finally {
      setDeletingId(null);
    }
  };

  const getAccountEmail = (accountId: string) => accounts.find((a) => a.id === accountId)?.email ?? accountId;

  return (
    <div className="flex flex-col flex-1 overflow-hidden bg-white">
      <div className="px-6 py-4 border-b border-gray-200 flex-shrink-0 flex items-center justify-between">
        <h1 className="text-xl font-semibold text-gray-900">{t('compose:drafts.title')}</h1>
        {drafts.length > 0 && (
          <span className="text-sm text-gray-500">
            {drafts.length} draft{drafts.length !== 1 ? 's' : ''}
          </span>
        )}
      </div>

      {isLoading ? (
        <div className="flex items-center justify-center flex-1">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600" />
        </div>
      ) : !accountId ? (
        <div className="flex items-center justify-center flex-1 text-sm text-gray-500">
          {t('compose:drafts.selectAccount')}
        </div>
      ) : drafts.length === 0 ? (
        <div className="flex flex-col items-center justify-center flex-1 text-center p-8">
          <svg className="h-12 w-12 text-gray-300 mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1}
              d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
            />
          </svg>
          <p className="text-sm text-gray-500">{t('compose:drafts.emptyLong')}</p>
          <p className="text-xs text-gray-400 mt-1">{t('compose:drafts.emptyHint')}</p>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto divide-y divide-gray-100">
          {drafts.map((draft) => (
            <div key={draft.id} className="px-6 py-4 hover:bg-gray-50 transition-colors flex items-start gap-4">
              <div className="flex-1 min-w-0">
                <div className="flex items-baseline gap-2 mb-1">
                  <span className="text-sm font-medium text-gray-900 truncate">{draft.subject || '(no subject)'}</span>
                  <span className="text-xs text-gray-400 flex-shrink-0">
                    {format(new Date(draft.updatedAt * 1000), 'MMM d, yyyy')}
                  </span>
                </div>
                {draft.toAddresses.length > 0 && (
                  <div className="text-xs text-gray-500 mb-1">To: {draft.toAddresses.join(', ')}</div>
                )}
                <div className="text-sm text-gray-400 truncate">{draft.body.slice(0, 120) || '(empty)'}</div>
                <div className="text-xs text-gray-400 mt-1">From: {getAccountEmail(draft.accountId)}</div>
              </div>
              <div className="flex items-center gap-1 flex-shrink-0">
                <button
                  type="button"
                  onClick={() => onOpenComposeTab(draft)}
                  className="px-3 py-1.5 text-xs font-medium text-primary-600 hover:text-primary-700 hover:bg-primary-50 rounded-lg transition-colors"
                  title={t('compose:drafts.continueEditing')}
                >
                  Edit
                </button>
                <button
                  type="button"
                  onClick={() => handleDelete(draft)}
                  disabled={deletingId === draft.id}
                  className="p-1.5 text-gray-400 hover:text-red-500 hover:bg-red-50 rounded-lg transition-colors disabled:opacity-50"
                  title={t('compose:drafts.delete')}
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                    />
                  </svg>
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
