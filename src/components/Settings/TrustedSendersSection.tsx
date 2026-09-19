// The senders allowed to load remote content automatically, with a way to take
// that back.
//
// `addTrustedSender` has always been wired, to the "Trust sender" button on the
// blocked-images banner. `listTrustedSenders` and `removeTrustedSender` were
// implemented, registered and wrapped in api.ts with no caller anywhere — so a
// single click, sitting next to "Show images", granted permanent permission to
// fetch remote content (tracking pixels included) from that address, and
// nothing in the app would show you the grant or let you undo it.
//
// Grants are per account, and shown per account for the same reason
// `JunkSettings` reports its counts that way: a list that silently described
// whichever mailbox the rest of the app happened to have selected would be
// unreadable on a multi-account install.

import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';

export function TrustedSendersSection() {
  const { t } = useTranslation(['common', 'settings']);
  const accounts = useAccountStore((s) => s.accounts);
  const addLog = useLogStore((s) => s.addLog);
  const [sendersByAccount, setSendersByAccount] = useState<Record<string, string[]>>({});
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const entries = await Promise.all(
        accounts.map(async (account) => {
          try {
            return [account.id, await api.listTrustedSenders(account.id)] as const;
          } catch (e) {
            addLog('error', 'system', `Failed to list trusted senders for ${account.email}: ${errorText(e)}`);
            return [account.id, []] as const;
          }
        }),
      );
      if (cancelled) return;
      setSendersByAccount(Object.fromEntries(entries));
      setIsLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [accounts, addLog]);

  const handleRevoke = useCallback(
    async (accountId: string, sender: string) => {
      try {
        await api.removeTrustedSender(accountId, sender);
        // Only drop the row once the backend confirms. Removing it optimistically
        // would tell the user the grant is gone while it is still loading images.
        setSendersByAccount((prev) => ({
          ...prev,
          [accountId]: (prev[accountId] ?? []).filter((s) => s !== sender),
        }));
        addLog('success', 'system', `Revoked remote-image trust for ${sender}.`);
      } catch (e) {
        addLog('error', 'system', `Failed to revoke trust for ${sender}: ${errorText(e)}`);
      }
    },
    [addLog],
  );

  if (isLoading) {
    return <p className="text-xs text-gray-500">{t('settings:privacy.trustedSenders.loading')}</p>;
  }

  return (
    <div className="rounded-lg border border-gray-700 bg-[#1f1f20] divide-y divide-gray-700">
      {accounts.map((account) => {
        const senders = sendersByAccount[account.id] ?? [];
        return (
          <div key={account.id} className="px-4 py-3">
            <p className="text-xs font-medium text-gray-400 mb-2">{account.email}</p>
            {senders.length === 0 ? (
              <p className="text-xs text-gray-500">{t('settings:privacy.trustedSenders.none')}</p>
            ) : (
              <ul className="space-y-1">
                {senders.map((sender) => (
                  <li key={sender} className="flex items-center justify-between gap-3">
                    <span className="text-sm text-gray-200 truncate font-mono">{sender}</span>
                    <button
                      type="button"
                      data-sender={sender}
                      onClick={() => handleRevoke(account.id, sender)}
                      className="text-xs text-primary-400 hover:text-primary-300 transition-colors flex-shrink-0"
                    >
                      {t('settings:privacy.trustedSenders.revoke')}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        );
      })}
    </div>
  );
}
