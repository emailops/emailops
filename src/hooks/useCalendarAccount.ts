import { useCallback, useEffect, useMemo, useState } from 'react';
import * as api from '@/lib/api';
import { resolveCalendarAccountId } from '@/lib/calendarGrid';
import { errorText } from '@/lib/errors';
import { ALL_ACCOUNTS_ID, isUnifiedMode, useAccountStore } from '@/stores/accountStore';
import {
  calendarDisabledCurrentAccount,
  calendarEnabledAccounts,
  useCalendarIntegrationStore,
} from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';

/** Pref key for the calendar's own persisted account selection. */
const ACCOUNT_PREF_KEY = 'calendar_selected_account';

/**
 * The sidebar account (or `ALL_ACCOUNTS_ID`) the calendar last snapped to. Module-level on purpose:
 * the docked agenda and the full view are separate mounts of this hook, and
 * a manual pick in one must survive opening the other — only a *change* of
 * the sidebar account snaps the selection again.
 */
let appliedDefaultAccountId: string | null = null;

/** Test seam: forget the last applied sidebar account between tests. */
export function resetCalendarAccountDefaultForTests(): void {
  appliedDefaultAccountId = null;
}

/**
 * Which account the calendar surfaces (full view, day-agenda panel) show: one
 * account, or `ALL_ACCOUNTS_ID` for every calendar account together (offered
 * once two have a calendar). Follows the sidebar — the account the user is
 * "in" (`defaultAccountId`) when it has an enabled calendar, "All accounts"
 * when the sidebar is there; a manual pick in the selector wins afterwards,
 * until the sidebar selection changes again. Persisted under
 * `calendar_selected_account`.
 */
export function useCalendarAccount(accounts: Account[], defaultAccountId: string | null) {
  const addLog = useLogStore((s) => s.addLog);
  const calendarIntegrationIds = useCalendarIntegrationStore((s) => s.enabledIds);
  const integrationLoaded = useCalendarIntegrationStore((s) => s.isLoaded);
  const setIntegrationEnabled = useCalendarIntegrationStore((s) => s.setEnabled);
  const sidebarIsUnified = useAccountStore((s) => isUnifiedMode(s.activeAccountId));
  const feedsByAccount = useCalendarIntegrationStore((s) => s.feedsByAccount);

  // Only offer accounts whose calendar integration the user enabled in
  // Settings → Calendar (an IMAP account counts once it has an ICS feed).
  const calendarAccounts = useMemo(
    () => calendarEnabledAccounts(accounts, calendarIntegrationIds, feedsByAccount),
    [accounts, calendarIntegrationIds, feedsByAccount],
  );

  // The account the user is "in" when its calendar integration is switched
  // off — instead of silently falling back to another account, the surface
  // offers to enable it right there. Gated on the pref load so the banner
  // never flashes before the enabled set is known. In "All accounts" the user
  // is in no single account (`defaultAccountId` is just the first one).
  const disabledCurrentAccount = useMemo(
    () =>
      integrationLoaded && !sidebarIsUnified
        ? calendarDisabledCurrentAccount(accounts, calendarIntegrationIds, defaultAccountId, feedsByAccount)
        : null,
    [integrationLoaded, sidebarIsUnified, accounts, calendarIntegrationIds, defaultAccountId, feedsByAccount],
  );

  // Persisted account selection: load once, then resolve against the enabled
  // account set (falling back to the effective account when the stored one is
  // gone or disabled).
  const [persistedAccountId, setPersistedAccountId] = useState<string | null>(null);
  const [prefLoaded, setPrefLoaded] = useState(false);
  useEffect(() => {
    let cancelled = false;
    api
      .getPref(ACCOUNT_PREF_KEY)
      .then((raw) => {
        if (cancelled) return;
        setPersistedAccountId(raw);
        setPrefLoaded(true);
      })
      .catch((err) => {
        if (cancelled) return;
        addLog('error', 'system', `Failed to load calendar account pref: ${err}`);
        setPrefLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [addLog]);

  // Gated on both loads so the override is never undone by the async pref
  // read landing late.
  const followAccountId = sidebarIsUnified ? ALL_ACCOUNTS_ID : defaultAccountId;
  useEffect(() => {
    if (!prefLoaded || !integrationLoaded || !followAccountId) return;
    if (appliedDefaultAccountId === followAccountId) return;
    const followable =
      followAccountId === ALL_ACCOUNTS_ID
        ? calendarAccounts.length >= 2
        : calendarAccounts.some((a) => a.id === followAccountId);
    if (!followable) return;
    appliedDefaultAccountId = followAccountId;
    setPersistedAccountId(followAccountId);
  }, [prefLoaded, integrationLoaded, followAccountId, calendarAccounts]);

  const selectedAccountId = useMemo(
    () => (prefLoaded ? resolveCalendarAccountId(calendarAccounts, persistedAccountId, defaultAccountId) : null),
    [prefLoaded, calendarAccounts, persistedAccountId, defaultAccountId],
  );
  const isAllAccounts = selectedAccountId === ALL_ACCOUNTS_ID;

  const selectedAccount = useMemo(
    () => calendarAccounts.find((a) => a.id === selectedAccountId) ?? null,
    [calendarAccounts, selectedAccountId],
  );

  const handleSelectAccount = useCallback(
    (id: string) => {
      setPersistedAccountId(id);
      api.setPref(ACCOUNT_PREF_KEY, id).catch((err) => {
        addLog('error', 'system', `Failed to save calendar account pref: ${err}`);
      });
    },
    [addLog],
  );

  // Inline enable from the banner: flip the integration pref back on, then
  // select the account — the data hook's selection effect triggers the first
  // sync. Rethrows after logging so the caller can show the message.
  const [isEnabling, setIsEnabling] = useState(false);
  const enableCurrentCalendar = useCallback(async () => {
    if (!disabledCurrentAccount) return;
    setIsEnabling(true);
    try {
      await setIntegrationEnabled(disabledCurrentAccount.id, true);
      addLog('info', 'sync', `Calendar integration enabled for ${disabledCurrentAccount.email} — syncing…`);
      handleSelectAccount(disabledCurrentAccount.id);
    } catch (e) {
      addLog('error', 'system', `Failed to enable calendar integration: ${errorText(e)}`);
      throw e;
    } finally {
      setIsEnabling(false);
    }
  }, [disabledCurrentAccount, setIntegrationEnabled, handleSelectAccount, addLog]);

  return {
    calendarAccounts,
    disabledCurrentAccount,
    selectedAccountId,
    selectedAccount,
    isAllAccounts,
    handleSelectAccount,
    enableCurrentCalendar,
    isEnabling,
  };
}
