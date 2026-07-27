import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';

/**
 * Per-account calendar integration (default ON — only an explicit "false"
 * pref disables, written by the user's Settings → Calendar toggle or by the
 * backend when the account never granted calendar permission). The pref key
 * mirrors the backend's `db::calendar::calendar_enabled_pref_key` — the
 * backend gates calendar sync, meeting notifications, and the chat calendar
 * tool on the same pref, so this store only controls UI visibility.
 */
export function calendarEnabledPrefKey(accountId: string): string {
  return `calendar.enabled:${accountId}`;
}

/** Accounts that CAN have a calendar: enabled Gmail/Outlook — never IMAP. */
export function calendarCapableAccounts(accounts: Account[]): Account[] {
  return accounts.filter((a) => a.enabled && a.provider !== 'imap');
}

/** Capable accounts the user opted into calendar integration. A stray pref
 *  for an incapable account (e.g. IMAP) never resurrects it. */
export function calendarEnabledAccounts(accounts: Account[], enabledIds: ReadonlySet<string>): Account[] {
  return calendarCapableAccounts(accounts).filter((a) => enabledIds.has(a.id));
}

/** The current account when it could have a calendar but the integration is
 *  switched off (user toggle or permission-denied auto-disable) — the calendar
 *  view offers an inline "Enable calendar" banner for it instead of silently
 *  falling back to another account. */
export function calendarDisabledCurrentAccount(
  accounts: Account[],
  enabledIds: ReadonlySet<string>,
  currentAccountId: string | null,
): Account | null {
  if (!currentAccountId) return null;
  const account = calendarCapableAccounts(accounts).find((a) => a.id === currentAccountId);
  return account && !enabledIds.has(account.id) ? account : null;
}

interface CalendarIntegrationStore {
  /** Ids of accounts with calendar integration switched on. */
  enabledIds: Set<string>;
  /** False until the first `loadForAccounts` resolves — gate probing UI
   *  (e.g. the invite card) on this so nothing flashes before prefs load. */
  isLoaded: boolean;
  loadForAccounts: (accounts: Account[]) => Promise<void>;
  setEnabled: (accountId: string, enabled: boolean) => Promise<void>;
  /** Apply a backend-initiated change (the `calendar-integration-changed`
   *  event, e.g. permission-denied auto-disable) — no pref write, the backend
   *  already persisted it. */
  applyBackendChange: (accountId: string, enabled: boolean) => void;
}

let currentLoadId = 0;

export const useCalendarIntegrationStore = create<CalendarIntegrationStore>((set, get) => ({
  enabledIds: new Set<string>(),
  isLoaded: false,

  loadForAccounts: async (accounts) => {
    const loadId = ++currentLoadId;
    const capable = calendarCapableAccounts(accounts);
    const flags = await Promise.all(
      capable.map(async (account) => {
        try {
          // Default ON: only an explicit "false" disables.
          return (await api.getPref(calendarEnabledPrefKey(account.id))) !== 'false';
        } catch (e) {
          // A failed read counts as disabled — surface it, don't hide features silently.
          useLogStore
            .getState()
            .addLog('error', 'system', `Failed to load calendar pref for ${account.email}: ${errorText(e)}`);
          return false;
        }
      }),
    );
    if (loadId !== currentLoadId) return; // a newer load superseded this one
    set({
      enabledIds: new Set(capable.filter((_, i) => flags[i]).map((a) => a.id)),
      isLoaded: true,
    });
  },

  setEnabled: async (accountId, enabled) => {
    const previous = get().enabledIds;
    const next = new Set(previous);
    if (enabled) {
      next.add(accountId);
    } else {
      next.delete(accountId);
    }
    set({ enabledIds: next });
    try {
      await api.setPref(calendarEnabledPrefKey(accountId), enabled ? 'true' : 'false');
    } catch (e) {
      set({ enabledIds: previous }); // revert the optimistic flip
      throw e;
    }
  },

  applyBackendChange: (accountId, enabled) => {
    set((state) => {
      const next = new Set(state.enabledIds);
      if (enabled) {
        next.add(accountId);
      } else {
        next.delete(accountId);
      }
      return { enabledIds: next };
    });
  },
}));
