// How far the email index is from done, and how fast it is being filled.
// The backend counts the backlog the way the indexing pipeline picks emails
// and measures the pace over its last two minutes, once there is enough to
// measure (`get_indexing_status`); this only shows it, refreshed while
// Settings is open.

import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';

const REFRESH_MS = 5000;

export function IndexingStatus() {
  const { t, i18n } = useTranslation(['common', 'settings']);
  const [status, setStatus] = useState<api.IndexingStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    const load = async () => {
      try {
        const next = await api.getIndexingStatus();
        if (stale) return;
        setStatus(next);
        setError(null);
      } catch (e) {
        if (!stale) setError(errorText(e));
      }
    };
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => {
      stale = true;
      clearInterval(timer);
    };
  }, []);

  // One decimal below ten a minute, so a slow server never reads as zero.
  const rate = (perMinute: number) =>
    new Intl.NumberFormat(i18n.language, { maximumFractionDigits: perMinute < 10 ? 1 : 0 }).format(perMinute);

  return (
    <section>
      <h3 className="text-sm font-medium text-gray-700 mb-1">{t('settings:ai.indexing.title')}</h3>
      {error && <p className="text-xs text-red-600">{t('settings:ai.indexing.loadFailed', { error })}</p>}
      {status && status.backlog === 0 && <p className="text-xs text-gray-600">{t('settings:ai.indexing.upToDate')}</p>}
      {status && status.backlog > 0 && (
        <>
          <p className="text-xs text-gray-600">{t('settings:ai.indexing.backlog', { count: status.backlog })}</p>
          {/* No pace in a run's first seconds either, so none is shown rather
              than a claim that nothing is being indexed. */}
          {status.messagesPerMinute != null && (
            <p className="text-xs text-gray-600">
              {t('settings:ai.indexing.rate', { rate: rate(status.messagesPerMinute) })}
            </p>
          )}
        </>
      )}
    </section>
  );
}
