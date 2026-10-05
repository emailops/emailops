// Settings → Junk → Blocked senders: every sender the user blocked, per
// account, each with Unblock — the reachable inverse of "Block sender".

import { format } from 'date-fns';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useAccountStore } from '@/stores/accountStore';
import { useSenderStore } from '@/stores/senderStore';

export function BlockedSendersSettings() {
  const { t } = useTranslation(['settings']);
  const blocked = useSenderStore((s) => s.blocked);
  const loadBlocked = useSenderStore((s) => s.loadBlocked);
  const openDialog = useSenderStore((s) => s.openDialog);
  const accounts = useAccountStore((s) => s.accounts);

  useEffect(() => {
    void loadBlocked();
  }, [loadBlocked]);

  const accountEmail = (id: string) => accounts.find((a) => a.id === id)?.email ?? id;
  const showAccount = new Set(blocked.map((b) => b.accountId)).size > 1 || accounts.length > 1;

  return (
    <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3" data-testid="blocked-senders">
      <h3 className="text-sm font-semibold text-gray-300">{t('settings:junk.blocked.title')}</h3>
      <p className="mt-1 text-xs text-gray-500">{t('settings:junk.blocked.desc')}</p>
      {blocked.length === 0 ? (
        <p className="mt-3 text-xs text-gray-500">{t('settings:junk.blocked.empty')}</p>
      ) : (
        <ul className="mt-3 divide-y divide-gray-700">
          {blocked.map((b) => (
            <li key={`${b.accountId}:${b.address}`} className="flex items-center gap-3 py-2 text-sm">
              <div className="min-w-0 flex-1">
                <div className="truncate text-gray-200">{b.address}</div>
                <div className="truncate text-xs text-gray-500">
                  {showAccount && <span>{accountEmail(b.accountId)} · </span>}
                  {t('settings:junk.blocked.since', { date: format(new Date(b.createdAt * 1000), 'PP') })}
                </div>
              </div>
              <button
                type="button"
                className="rounded px-2 py-1 text-xs text-gray-300 hover:bg-gray-700"
                onClick={() => openDialog({ type: 'unblock', accountId: b.accountId, address: b.address })}
              >
                {t('settings:junk.blocked.unblock')}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
