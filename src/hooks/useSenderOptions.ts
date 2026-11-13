import { useEffect, useMemo, useState } from 'react';
import * as api from '@/lib/api';
import { orderSendersByUsage } from '@/lib/composeSender';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Account, AccountSentCount } from '@/types';

/**
 * Options for a composer's From list: the enabled accounts, the one with the
 * most sent mail first. Counted once when the composer opens; until then (or
 * if the count fails) the accounts keep their order. Only the order changes:
 * the composer's selected sender is its own state.
 */
export function useSenderOptions(accounts: Account[]): { value: string; label: string }[] {
  const [counts, setCounts] = useState<AccountSentCount[]>([]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await api.getSentCounts();
        if (!cancelled) setCounts(loaded);
      } catch (err) {
        useLogStore.getState().addLog('debug', 'system', `Could not order senders by usage: ${errorText(err)}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return useMemo(
    () => orderSendersByUsage(accounts, counts).map((a) => ({ value: a.id, label: a.email })),
    [accounts, counts],
  );
}
