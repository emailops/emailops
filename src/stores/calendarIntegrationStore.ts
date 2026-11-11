import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Account, CalendarFeed } from '@/types';

/**
 * Per-account calendar integration (default ON — only an explicit "false"
 * pref disables, written by the user's Settings → Calendar toggle or by the
 * backend when the account never granted calendar permission). The pref key
 * mirrors the backend's `db::calendar::calendar_enabled_pref_key` — the
 * backend gates calendar sync, meeting notifications, and the chat calendar
 * tool on the same pref, so this store only controls UI visibility.
 *
 * Which accounts CAN have a calendar mirrors the backend's
 * `services::calendar::capability`: Gmail / Outlook always, an IMAP account
 * once it subscribed to at least one ICS feed (`feedsByAccount`).
 */
export function calendarEnabledPrefKey(accountId: string): string {
  return `calendar.enabled:${accountId}`;
}

/** Subscribed ICS feeds per account id (IMAP accounts only; `[]` when none). */
export type FeedsByAccount = Readonly<Record<string, readonly CalendarFeed[]>>;

function hasFeed(feeds: FeedsByAccount, accountId: string): boolean {
  return (feeds[accountId]?.length ?? 0) > 0;
}

/** Accounts that CAN have a calendar: enabled Gmail/Outlook, or an enabled
 *  IMAP account with at least one feed. */
export function calendarCapableAccounts(accounts: Account[], feeds: FeedsByAccount): Account[] {
  return accounts.filter((a) => a.enabled && (a.provider !== 'imap' || hasFeed(feeds, a.id)));
}

/** Capable accounts the user opted into calendar integration. A stray pref
 *  for an incapable account (e.g. IMAP without feeds) never resurrects it. */
export function calendarEnabledAccounts(
  accounts: Account[],
  enabledIds: ReadonlySet<string>,
  feeds: FeedsByAccount,
): Account[] {
  return calendarCapableAccounts(accounts, feeds).filter((a) => enabledIds.has(a.id));
}

/** The current account when it could have a calendar but the integration is
 *  switched off (user toggle or permission-denied auto-disable) — the calendar
 *  view offers an inline "Enable calendar" banner for it instead of silently
 *  falling back to another account. */
export function calendarDisabledCurrentAccount(
  accounts: Account[],
  enabledIds: ReadonlySet<string>,
  currentAccountId: string | null,
  feeds: FeedsByAccount,
): Account | null {
  if (!currentAccountId) return null;
  const account = calendarCapableAccounts(accounts, feeds).find((a) => a.id === currentAccountId);
  return account && !enabledIds.has(account.id) ? account : null;
}

/** What the calendar panel shows for one account: its agenda (`ready`), a
 *  "subscribe to a feed" prompt (`setup`: IMAP without feeds), the "enable
 *  calendar" banner (`disabled`), or nothing (`none`: no usable account). */
export type CalendarPanelState = 'ready' | 'setup' | 'disabled' | 'none';

export function calendarPanelState(
  account: Account | null,
  enabledIds: ReadonlySet<string>,
  feeds: FeedsByAccount,
): CalendarPanelState {
  if (!account || !account.enabled) return 'none';
  if (account.provider === 'imap' && !hasFeed(feeds, account.id)) return 'setup';
  return enabledIds.has(account.id) ? 'ready' : 'disabled';
}

/** Pure reducer: `feed` appended to its account's list. */
export function feedsAfterAdd(feeds: FeedsByAccount, feed: CalendarFeed): FeedsByAccount {
  return { ...feeds, [feed.accountId]: [...(feeds[feed.accountId] ?? []), feed] };
}

/** Pure reducer: `feedId` removed from `accountId`'s list. */
export function feedsAfterRemove(feeds: FeedsByAccount, accountId: string, feedId: string): FeedsByAccount {
  const current = feeds[accountId];
  if (!current?.some((f) => f.id === feedId)) return feeds;
  return { ...feeds, [accountId]: current.filter((f) => f.id !== feedId) };
}

interface CalendarIntegrationStore {
  /** Ids of accounts with calendar integration switched on. */
  enabledIds: Set<string>;
  /** Subscribed ICS feeds per IMAP account id. */
  feedsByAccount: FeedsByAccount;
  /** False until the first `loadForAccounts` resolves — gate probing UI
   *  (e.g. the invite card) on this so nothing flashes before prefs load. */
  isLoaded: boolean;
  loadForAccounts: (accounts: Account[]) => Promise<void>;
  setEnabled: (accountId: string, enabled: boolean) => Promise<void>;
  /** Apply a backend-initiated change (the `calendar-integration-changed`
   *  event, e.g. permission-denied auto-disable) — no pref write, the backend
   *  already persisted it. */
  applyBackendChange: (accountId: string, enabled: boolean) => void;
  /** Subscribe an IMAP account to a feed. The account becomes capable, so its
   *  opt-in pref is read (default ON) right after. */
  addFeed: (accountId: string, label: string, url: string) => Promise<CalendarFeed>;
  removeFeed: (accountId: string, feedId: string) => Promise<void>;
}

let currentLoadId = 0;

async function readEnabledPref(account: Pick<Account, 'id' | 'email'>): Promise<boolean> {
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
}

export const useCalendarIntegrationStore = create<CalendarIntegrationStore>((set, get) => ({
  enabledIds: new Set<string>(),
  feedsByAccount: {},
  isLoaded: false,

  loadForAccounts: async (accounts) => {
    const loadId = ++currentLoadId;
    // Feeds first: they decide which IMAP accounts are capable at all.
    const imapAccounts = accounts.filter((a) => a.enabled && a.provider === 'imap');
    const feedLists = await Promise.all(
      imapAccounts.map(async (account) => {
        try {
          return await api.listCalendarFeeds(account.id);
        } catch (e) {
          useLogStore
            .getState()
            .addLog('error', 'system', `Failed to load calendar feeds for ${account.email}: ${errorText(e)}`);
          return [] as CalendarFeed[];
        }
      }),
    );
    const feeds: FeedsByAccount = Object.fromEntries(imapAccounts.map((a, i) => [a.id, feedLists[i]]));
    const capable = calendarCapableAccounts(accounts, feeds);
    const flags = await Promise.all(capable.map(readEnabledPref));
    if (loadId !== currentLoadId) return; // a newer load superseded this one
    set({
      enabledIds: new Set(capable.filter((_, i) => flags[i]).map((a) => a.id)),
      feedsByAccount: feeds,
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

  addFeed: async (accountId, label, url) => {
    const feed = await api.addCalendarFeed(accountId, label, url);
    const enabled = await readEnabledPref({ id: accountId, email: accountId });
    set((state) => {
      const next = new Set(state.enabledIds);
      if (enabled) {
        next.add(accountId);
      } else {
        next.delete(accountId);
      }
      return { feedsByAccount: feedsAfterAdd(state.feedsByAccount, feed), enabledIds: next };
    });
    return feed;
  },

  removeFeed: async (accountId, feedId) => {
    await api.removeCalendarFeed(accountId, feedId);
    set((state) => ({ feedsByAccount: feedsAfterRemove(state.feedsByAccount, accountId, feedId) }));
  },
}));
