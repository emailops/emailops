import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ClockIcon } from '@/components/common/MailIcons';
import { useFormatters } from '@/hooks/useFormatters';
import type { OutboxEntry } from '@/lib/api';
import { useOutboxStore } from '@/stores/outboxStore';
import type { Account } from '@/types';

interface ScheduledViewProps {
  /** Account to list, or `null` for every enabled account. */
  accountId: string | null;
  accounts: Account[];
}

/**
 * The Scheduled view: messages waiting in the local outbox for their send
 * time, and the ones that could not be sent, with Send now / Retry, Edit
 * (takes the message back into a composer) and Delete (its toast's Undo
 * reopens it). States that EmailOps must be open for them to go out.
 */
export function ScheduledView({ accountId, accounts }: ScheduledViewProps) {
  const { t } = useTranslation(['compose']);
  const entries = useOutboxStore((s) => s.entries);
  const fetchEntries = useOutboxStore((s) => s.fetchEntries);

  useEffect(() => {
    void fetchEntries(accountId);
  }, [accountId, fetchEntries]);

  const showAccount = accountId === null && accounts.length > 1;
  const accountEmail = (id: string) => accounts.find((a) => a.id === id)?.email ?? id;

  return (
    <div className="flex flex-col flex-1 overflow-hidden bg-white">
      <div className="px-6 py-4 border-b border-gray-200 flex-shrink-0">
        <h1 className="text-xl font-semibold text-gray-900">{t('compose:scheduled.title')}</h1>
        <p data-testid="scheduled-app-open-note" className="mt-1 text-xs text-gray-500">
          {t('compose:scheduled.appOpenNote')}
        </p>
      </div>
      {entries.length === 0 ? (
        <div className="flex flex-col items-center justify-center flex-1 text-center p-8">
          <ClockIcon className="h-12 w-12 text-gray-300 mb-3" />
          <p className="text-sm text-gray-500">{t('compose:scheduled.empty')}</p>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto divide-y divide-gray-100">
          {entries.map((entry) => (
            <ScheduledRow
              key={entry.id}
              entry={entry}
              accountLabel={showAccount ? accountEmail(entry.accountId) : null}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ScheduledRow({ entry, accountLabel }: { entry: OutboxEntry; accountLabel: string | null }) {
  const { t } = useTranslation(['compose']);
  const fmt = useFormatters();
  const edit = useOutboxStore((s) => s.edit);
  const remove = useOutboxStore((s) => s.remove);
  const sendNow = useOutboxStore((s) => s.sendNow);
  const failed = entry.status === 'failed';
  const sending = entry.status === 'sending';
  const recipients = [...entry.toAddresses, ...entry.ccAddresses].join(', ');
  const buttonClass =
    'px-2.5 py-1 text-xs font-medium rounded border border-gray-300 text-gray-700 hover:bg-gray-100 disabled:opacity-50';

  return (
    <div data-testid="scheduled-row" data-outbox-id={entry.id} className="px-6 py-4 flex items-start gap-4">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-gray-900 truncate">
            {entry.subject || t('compose:scheduled.noSubject')}
          </span>
          {entry.kind === 'reply' && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-600">
              {t('compose:scheduled.reply')}
            </span>
          )}
        </div>
        <div className="text-xs text-gray-500 truncate">{t('compose:scheduled.to', { recipients })}</div>
        {accountLabel && <div className="text-xs text-gray-400 truncate">{accountLabel}</div>}
        <div className="mt-1 flex items-center gap-2 text-xs">
          {failed ? (
            <span data-testid="scheduled-failed" className="text-red-600">
              {entry.failureKind === 'interrupted'
                ? t('compose:scheduled.interrupted')
                : `${t('compose:scheduled.failed')}${entry.lastError ? `: ${entry.lastError}` : ''}`}
            </span>
          ) : sending ? (
            <span className="text-gray-500">{t('compose:scheduled.sending')}</span>
          ) : (
            <span className="text-gray-600">
              {t('compose:scheduled.sendsAt', {
                time: fmt.dateTime(entry.sendAt, {
                  weekday: 'short',
                  day: 'numeric',
                  month: 'short',
                  hour: '2-digit',
                  minute: '2-digit',
                }),
              })}
            </span>
          )}
          {entry.attachmentCount > 0 && (
            <span className="text-gray-400">
              {t('compose:scheduled.attachments', { count: entry.attachmentCount })}
            </span>
          )}
        </div>
      </div>
      <div className="flex items-center gap-1.5 flex-shrink-0">
        <button
          type="button"
          data-testid="scheduled-send-now"
          disabled={sending}
          onClick={() => void sendNow(entry.id)}
          className={buttonClass}
        >
          {failed ? t('compose:scheduled.retry') : t('compose:scheduled.sendNow')}
        </button>
        <button
          type="button"
          data-testid="scheduled-edit"
          disabled={sending}
          onClick={() => void edit(entry.id)}
          className={buttonClass}
        >
          {t('compose:scheduled.edit')}
        </button>
        <button
          type="button"
          data-testid="scheduled-delete"
          disabled={sending}
          onClick={() => void remove(entry.id)}
          className={`${buttonClass} hover:text-red-600`}
        >
          {t('compose:scheduled.delete')}
        </button>
      </div>
    </div>
  );
}
