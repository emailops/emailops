import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getDashboardStats, getQueueState, getStorageStats } from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Account, AccountDashboard, AllQueuesState, StorageStats } from '@/types';
import { AccountPanel } from './AccountPanel';
import { QueuePanel } from './QueuePanel';
import { StorageCard } from './StorageCard';

const DASHBOARD_POLL_MS = 5_000;
const QUEUE_POLL_MS = 1_500;

interface DashboardProps {
  accounts: Account[];
  onOpenAccountSettings: (accountId: string) => void;
}

export function Dashboard({ accounts, onOpenAccountSettings }: DashboardProps) {
  const { t } = useTranslation(['dashboard']);
  const [stats, setStats] = useState<AccountDashboard[] | null>(null);
  const [queues, setQueues] = useState<AllQueuesState | null>(null);
  const [storage, setStorage] = useState<StorageStats | null>(null);
  const [storageErr, setStorageErr] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const refreshStats = useCallback(async () => {
    try {
      const s = await getDashboardStats();
      setStats(s);
      setErr(null);
    } catch (e) {
      setErr(errorText(e));
    }
  }, []);

  const refreshQueues = useCallback(async () => {
    try {
      const q = await getQueueState();
      setQueues(q);
    } catch {
      // queue state errors are not fatal — leave the previous snapshot
    }
  }, []);

  // Initial load. Storage stats are computed once on mount: they walk
  // the filesystem (cheap) but the result rarely changes during a single
  // dashboard session, so polling would just spam disk reads.
  useEffect(() => {
    void refreshStats();
    void refreshQueues();
    getStorageStats()
      .then((s) => {
        setStorage(s);
        setStorageErr(null);
      })
      .catch((e) => setStorageErr(errorText(e)));
  }, [refreshStats, refreshQueues]);

  // Polling
  useEffect(() => {
    const statsT = setInterval(() => {
      void refreshStats();
    }, DASHBOARD_POLL_MS);
    const queueT = setInterval(() => {
      void refreshQueues();
    }, QUEUE_POLL_MS);
    return () => {
      clearInterval(statsT);
      clearInterval(queueT);
    };
  }, [refreshStats, refreshQueues]);

  // Re-fetch when an app-log event fires (sync done, embeddings done, etc.)
  useEffect(() => {
    const unlisten = listen('app-log', () => {
      void refreshStats();
    });
    return () => {
      unlisten.then((u) => u()).catch(() => undefined);
    };
  }, [refreshStats]);

  return (
    <div className="flex-1 overflow-auto bg-gray-950 text-gray-200 p-6">
      <div className="max-w-6xl mx-auto space-y-6">
        <header>
          <h1 className="text-2xl font-semibold text-white">{t('dashboard:title')}</h1>
          <p className="text-sm text-gray-400">
            Per-account sync, classification, and AI processing status. Polls every {DASHBOARD_POLL_MS / 1000}s.
          </p>
        </header>

        {err && (
          <div className="border border-red-800 bg-red-950 text-red-200 text-sm rounded p-3">
            Failed to load dashboard: {err}
          </div>
        )}

        <section>
          <h2 className="text-sm font-semibold text-gray-300 mb-2 uppercase tracking-wider">
            {t('dashboard:queues.title')}
          </h2>
          {queues ? (
            <QueuePanel state={queues} accounts={accounts} />
          ) : (
            <div className="text-xs text-gray-500 italic">{t('dashboard:queues.loading')}</div>
          )}
        </section>

        <section>
          <h2 className="text-sm font-semibold text-gray-300 mb-2 uppercase tracking-wider">
            {t('dashboard:storage.title')}
          </h2>
          <StorageCard stats={storage} error={storageErr} />
        </section>

        <section>
          <h2 className="text-sm font-semibold text-gray-300 mb-2 uppercase tracking-wider">
            {t('dashboard:accounts.title')}
          </h2>
          {stats === null ? (
            <div className="text-xs text-gray-500 italic">{t('dashboard:loading')}</div>
          ) : stats.length === 0 ? (
            <div className="text-xs text-gray-500 italic">{t('dashboard:noAccounts')}</div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {stats.map((s) => (
                <AccountPanel
                  key={s.account.id}
                  data={s}
                  onRefreshed={refreshStats}
                  onOpenSettings={() => onOpenAccountSettings(s.account.id)}
                />
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
