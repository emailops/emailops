import { type FormEvent, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { errorText } from '@/lib/errors';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';

interface CalendarFeedsSectionProps {
  /** Enabled IMAP accounts — the only ones whose calendar comes from feeds. */
  accounts: Account[];
}

/**
 * Settings → Calendar block for ICS feeds: one card per IMAP account with its
 * subscribed feeds (label + host, never the URL) and a form to add one. The
 * backend validates and fetches the URL, so a bad link fails here with its
 * message instead of silently producing an empty calendar.
 */
export function CalendarFeedsSection({ accounts }: CalendarFeedsSectionProps) {
  const { t } = useTranslation(['settings']);
  if (accounts.length === 0) return null;
  return (
    <section>
      <h3 className="text-sm font-semibold text-gray-700 mb-1">{t('settings:calendar.feedsLabel')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:calendar.feedsDesc')}</p>
      <div className="space-y-3">
        {accounts.map((account) => (
          <AccountFeeds key={account.id} account={account} />
        ))}
      </div>
      <p className="text-xs text-gray-500 mt-2">{t('settings:calendar.protonHelp')}</p>
    </section>
  );
}

function AccountFeeds({ account }: { account: Account }) {
  const { t } = useTranslation(['settings']);
  const addLog = useLogStore((s) => s.addLog);
  const feeds = useCalendarIntegrationStore((s) => s.feedsByAccount[account.id] ?? []);
  const addFeed = useCalendarIntegrationStore((s) => s.addFeed);
  const removeFeed = useCalendarIntegrationStore((s) => s.removeFeed);
  const [label, setLabel] = useState('');
  const [url, setUrl] = useState('');
  const [isAdding, setIsAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const trimmedUrl = url.trim();
    if (!trimmedUrl || isAdding) return;
    setIsAdding(true);
    setError(null);
    addFeed(account.id, label.trim(), trimmedUrl)
      .then((feed) => {
        setLabel('');
        setUrl('');
        addLog('success', 'sync', `Calendar feed "${feed.label}" added`);
      })
      .catch((e) => {
        const msg = errorText(e);
        setError(msg);
        addLog('error', 'sync', `Adding the calendar feed failed: ${msg}`);
      })
      .finally(() => setIsAdding(false));
  };

  const remove = (feedId: string, feedLabel: string) => {
    setError(null);
    removeFeed(account.id, feedId).catch((e) => {
      const msg = errorText(e);
      setError(msg);
      addLog('error', 'sync', `Removing the calendar feed "${feedLabel}" failed: ${msg}`);
    });
  };

  return (
    <div data-testid={`calendar-feeds-${account.id}`} className="rounded-lg border border-gray-200 bg-white px-4 py-3">
      <span className="text-sm text-gray-900 block truncate">{account.email}</span>
      {feeds.length === 0 ? (
        <p className="text-xs text-gray-500 mt-1">{t('settings:calendar.noFeeds')}</p>
      ) : (
        <ul className="mt-2 divide-y divide-gray-200">
          {feeds.map((feed) => (
            <li key={feed.id} className="flex items-center justify-between gap-3 py-1.5">
              <div className="min-w-0">
                <span className="text-sm text-gray-800 block truncate">{feed.label}</span>
                <span className="text-xs text-gray-500">{feed.host}</span>
              </div>
              <button
                type="button"
                onClick={() => remove(feed.id, feed.label)}
                aria-label={t('settings:calendar.removeFeedAria', { label: feed.label })}
                className="text-xs text-gray-500 hover:text-red-700 transition-colors flex-shrink-0"
              >
                {t('settings:calendar.removeFeed')}
              </button>
            </li>
          ))}
        </ul>
      )}
      <form onSubmit={submit} className="mt-3 flex flex-col gap-2 sm:flex-row">
        <input
          name="label"
          type="text"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder={t('settings:calendar.feedLabelPlaceholder')}
          className="sm:w-40 rounded border border-gray-300 bg-white px-2 py-1.5 text-sm text-gray-900 focus:outline-none focus:ring-1 focus:ring-primary-600"
        />
        <input
          name="url"
          type="url"
          required
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t('settings:calendar.feedUrlPlaceholder')}
          className="flex-1 rounded border border-gray-300 bg-white px-2 py-1.5 text-sm text-gray-900 focus:outline-none focus:ring-1 focus:ring-primary-600"
        />
        <button
          type="submit"
          disabled={isAdding || url.trim() === ''}
          className="px-3 py-1.5 rounded bg-primary-600 hover:bg-primary-700 text-white text-sm font-medium transition-colors disabled:opacity-50 flex-shrink-0"
        >
          {isAdding ? t('settings:calendar.addingFeed') : t('settings:calendar.addFeed')}
        </button>
      </form>
      {error && <p className="mt-2 text-xs text-red-700">{error}</p>}
    </div>
  );
}
