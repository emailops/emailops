// Reading-pane sender controls: the "Unsubscribe" link next to the sender
// (when the message offers a way out) and the banner on mail from a sender the
// user blocked or unsubscribed from. The confirmations live in `SenderDialogs`.

import { format } from 'date-fns';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { statusKey, useSenderStore } from '@/stores/senderStore';
import type { SenderStatus } from '@/types';

/** The message's sender facts, fetched once per message. */
export function useSenderStatus(accountId: string, emailId: string): SenderStatus | undefined {
  const status = useSenderStore((s) => s.statusByEmail[statusKey(accountId, emailId)]);
  const loadStatus = useSenderStore((s) => s.loadStatus);
  useEffect(() => {
    void loadStatus(accountId, emailId);
  }, [accountId, emailId, loadStatus]);
  return status;
}

interface SenderControlProps {
  accountId: string;
  emailId: string;
  senderName: string;
}

/** "Unsubscribe" next to the sender, or "Unsubscribed" once asked. */
export function UnsubscribeButton({ accountId, emailId, senderName }: SenderControlProps) {
  const { t } = useTranslation(['inbox']);
  const status = useSenderStatus(accountId, emailId);
  const openDialog = useSenderStore((s) => s.openDialog);
  if (!status?.unsubscribe) return null;
  if (status.unsubscribedAt !== null) {
    return (
      <span className="text-xs text-gray-400" data-testid="unsubscribed-label">
        {t('inbox:sender.unsubscribed')}
      </span>
    );
  }
  return (
    <button
      type="button"
      data-testid="unsubscribe-button"
      onClick={(e) => {
        e.stopPropagation();
        openDialog({ type: 'unsubscribe', accountId, emailId, senderName });
      }}
      className="text-xs text-gray-500 underline hover:text-gray-800"
    >
      {t('inbox:sender.unsubscribe')}
    </button>
  );
}

/** "You blocked this sender · Unblock", and when the user unsubscribed. */
export function SenderBanner({ accountId, emailId }: Omit<SenderControlProps, 'senderName'>) {
  const { t } = useTranslation(['inbox']);
  const status = useSenderStatus(accountId, emailId);
  const openDialog = useSenderStore((s) => s.openDialog);
  if (!status || (!status.blocked && status.unsubscribedAt === null)) return null;
  return (
    <div className="mb-3 rounded border border-slate-200 bg-slate-50 px-3 py-2 text-xs text-slate-700">
      {status.blocked && (
        <div className="flex items-center gap-2" data-testid="blocked-sender-banner">
          <span>{t('inbox:sender.blockedBanner')}</span>
          <button
            type="button"
            className="ml-auto font-medium underline hover:no-underline"
            onClick={() => openDialog({ type: 'unblock', accountId, address: status.address })}
          >
            {t('inbox:sender.unblockAction')}
          </button>
        </div>
      )}
      {status.unsubscribedAt !== null && (
        <div data-testid="unsubscribed-banner">
          {t('inbox:sender.unsubscribedBanner', { date: format(new Date(status.unsubscribedAt * 1000), 'PP') })}
        </div>
      )}
    </div>
  );
}
