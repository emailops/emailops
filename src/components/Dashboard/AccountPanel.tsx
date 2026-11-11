import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import { refreshServerTotal } from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { AccountDashboard } from '@/types';
import { ProgressBar } from './ProgressBar';

interface AccountPanelProps {
  data: AccountDashboard;
  onRefreshed: () => void;
  onOpenSettings: () => void;
}

const PROVIDER_LABEL: Record<string, string> = {
  gmail: 'GMAIL',
  imap: 'IMAP',
  outlook: 'OUTLOOK',
};

export function AccountPanel({ data, onRefreshed, onOpenSettings }: AccountPanelProps) {
  const { t } = useTranslation(['common', 'dashboard']);
  const fmt = useFormatters();
  const [refreshing, setRefreshing] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const handleRefresh = async () => {
    setRefreshing(true);
    setErr(null);
    try {
      await refreshServerTotal(data.account.id);
      onRefreshed();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setRefreshing(false);
    }
  };

  const provider = PROVIDER_LABEL[data.account.provider] ?? data.account.provider.toUpperCase();
  const enabled = data.account.enabled;
  const syncStatus = data.sync.status;

  return (
    <div className={`border border-gray-200 bg-white rounded-lg p-4 space-y-3 ${enabled ? '' : 'opacity-60'}`}>
      {/* Header */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          {/* An account without a name stores an empty one; show the address. */}
          <div className="text-sm font-semibold text-gray-900 truncate">{data.account.name || data.account.email}</div>
          <div className="text-xs text-gray-500 truncate">{data.account.email}</div>
        </div>
        <div className="flex items-start gap-2 shrink-0">
          <div className="flex flex-col items-end gap-1">
            <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-gray-100 text-gray-700 border border-gray-200">
              {provider}
            </span>
            <span
              className={`text-[10px] font-mono px-1.5 py-0.5 rounded ${
                syncStatus === 'syncing'
                  ? 'bg-blue-100 text-blue-700'
                  : syncStatus === 'error'
                    ? 'bg-red-100 text-red-700'
                    : 'bg-gray-100 text-gray-500'
              }`}
              title={data.sync.error ?? undefined}
            >
              {syncStatus}
            </span>
          </div>
          <button
            type="button"
            onClick={onOpenSettings}
            title={t('dashboard:accounts.openSettings')}
            aria-label={t('dashboard:accounts.openSettings')}
            className="p-1 rounded text-gray-500 hover:text-gray-700 hover:bg-gray-100 transition-colors"
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
      </div>

      {/* Sync stats */}
      <div className="grid grid-cols-2 gap-2 text-xs font-mono text-gray-800">
        <div>
          <div className="text-gray-500">{t('dashboard:accounts.syncedSince')}</div>
          <div>{fmt.date(data.syncedSince)}</div>
        </div>
        <div>
          <div className="text-gray-500">{t('dashboard:accounts.lastSync')}</div>
          <div>{fmt.dateTime(data.sync.lastSyncAt)}</div>
        </div>
        <div className="col-span-2">
          <div className="text-gray-500">{t('dashboard:accounts.localServer')}</div>
          <div className="flex items-center gap-2">
            <span>
              {fmt.number(data.syncedCount)} / {data.serverTotal != null ? fmt.number(data.serverTotal) : '—'}
            </span>
            <button
              type="button"
              onClick={handleRefresh}
              disabled={refreshing}
              className="px-2 py-0.5 text-[10px] rounded border border-gray-300 hover:bg-gray-100 text-gray-800 disabled:opacity-50"
              title={
                data.serverTotalFetchedAt
                  ? t('dashboard:accounts.lastFetched', { time: fmt.dateTime(data.serverTotalFetchedAt) })
                  : t('dashboard:accounts.fetchFromProvider')
              }
            >
              {refreshing ? t('dashboard:accounts.refreshing') : t('dashboard:accounts.refreshTotal')}
            </button>
          </div>
          {err && <div className="text-red-600 mt-1">{err}</div>}
        </div>
        <div className="col-span-2">
          <div className="text-gray-500">{t('dashboard:accounts.sent')}</div>
          <div>{fmt.number(data.sentCount)}</div>
        </div>
      </div>

      {/* Categories */}
      {data.categoryCounts.length > 0 && (
        <div className="text-xs font-mono">
          <div className="text-gray-500 mb-1">{t('dashboard:accounts.categories')}</div>
          <ul className="grid grid-cols-2 gap-x-3 gap-y-0.5 text-gray-800">
            {data.categoryCounts.map((c) => (
              <li key={c.category} className="flex justify-between">
                <span className="text-gray-700">{c.category || '(none)'}</span>
                <span>{fmt.number(c.count)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Progress bars */}
      <div className="space-y-2 pt-1">
        <ProgressBar
          label={t('dashboard:accounts.classified')}
          numerator={data.classifiedCount}
          denominator={data.classifiedEligible}
          color="bg-blue-500"
        />
        <ProgressBar
          label={t('dashboard:accounts.memoriesAnalyzed')}
          numerator={data.memoryAnalyzedCount}
          denominator={data.memoryEligible}
          color="bg-purple-500"
        />
        <ProgressBar
          label={t('dashboard:accounts.tasksAnalyzed')}
          numerator={data.taskAnalyzedCount}
          denominator={data.taskEligible}
          color="bg-emerald-500"
        />
        <ProgressBar
          label={t('dashboard:accounts.embeddings')}
          numerator={data.embeddedCount}
          denominator={data.embeddedEligible}
          color="bg-amber-500"
        />
        <ProgressBar
          label={t('dashboard:accounts.junkScored')}
          numerator={data.junkScoredCount}
          denominator={data.syncedCount}
          color="bg-gray-400"
        />
      </div>

      {/* What the junk detector found. Severity-ordered, and phishing is
          separated from the other two because it is a security finding rather
          than a tidiness one. Hidden entirely when nothing was flagged — an
          all-zero row is noise. */}
      {data.junkPhishingCount + data.junkSpamCount + data.junkGraymailCount > 0 && (
        <div className="flex flex-wrap gap-2 pt-2 text-xs">
          {data.junkPhishingCount > 0 && (
            <span className="rounded-full bg-red-100 text-red-800 px-2 py-0.5">
              {t('dashboard:accounts.junkPhishing', { count: data.junkPhishingCount })}
            </span>
          )}
          {data.junkSpamCount > 0 && (
            <span className="rounded-full bg-orange-100 text-orange-800 px-2 py-0.5">
              {t('dashboard:accounts.junkSpam', { count: data.junkSpamCount })}
            </span>
          )}
          {data.junkGraymailCount > 0 && (
            <span className="rounded-full bg-gray-100 text-gray-600 px-2 py-0.5">
              {t('dashboard:accounts.junkGraymail', { count: data.junkGraymailCount })}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
