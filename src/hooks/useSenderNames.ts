import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useAccountStore } from '@/stores/accountStore';
import { finishedSyncing, useSenderNameStore } from '@/stores/senderNameStore';
import type { Email } from '@/types';

/**
 * Names for the recipients of the user's own messages in `emails`: loads the
 * sender-name map of every account on screen, reloads one when its sync ends,
 * and returns a lookup by account and lowercased address.
 */
export function useSenderNames(emails: Email[]): (accountId: string, address: string) => string | undefined {
  const namesByAccount = useSenderNameStore((s) => s.namesByAccount);
  const ensureLoaded = useSenderNameStore((s) => s.ensureLoaded);
  const reload = useSenderNameStore((s) => s.reload);
  const syncingAccountIds = useAccountStore((s) => s.syncingAccountIds);

  // A string key, so a new page of the same accounts does not re-run the effect.
  const accountKey = useMemo(() => [...new Set(emails.map((e) => e.accountId))].sort().join('\n'), [emails]);
  useEffect(() => {
    for (const accountId of accountKey.split('\n')) {
      if (accountId) void ensureLoaded(accountId);
    }
  }, [accountKey, ensureLoaded]);

  // A sync ends once, when its account leaves the set — not on every batch.
  const syncingBefore = useRef(syncingAccountIds);
  useEffect(() => {
    for (const accountId of finishedSyncing(syncingBefore.current, syncingAccountIds)) void reload(accountId);
    syncingBefore.current = syncingAccountIds;
  }, [syncingAccountIds, reload]);

  return useCallback((accountId, address) => namesByAccount[accountId]?.[address], [namesByAccount]);
}
