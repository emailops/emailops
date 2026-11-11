import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  setPref: vi.fn(),
  listCalendarFeeds: vi.fn(async () => []),
  addCalendarFeed: vi.fn(),
  removeCalendarFeed: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import type { Account, CalendarFeed } from '@/types';
import {
  calendarCapableAccounts,
  calendarDisabledCurrentAccount,
  calendarEnabledAccounts,
  calendarEnabledPrefKey,
  calendarPanelState,
  feedsAfterAdd,
  feedsAfterRemove,
  useCalendarIntegrationStore,
} from './calendarIntegrationStore';

function makeAccount(id: string, provider: string, enabled = true): Account {
  return {
    id,
    provider,
    email: `${id}@example.com`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled,
    syncFromTimestamp: null,
  } as Account;
}

function makeFeed(id: string, accountId: string): CalendarFeed {
  return { id, accountId, label: id, host: 'calendar.proton.me', createdAt: 0 };
}

const NO_FEEDS = {};
const FEEDS = { 'i-feed': [makeFeed('f1', 'i-feed')] };

describe('calendar integration pure helpers', () => {
  it('capable accounts are enabled Gmail/Outlook, plus IMAP accounts with a feed', () => {
    const accounts = [
      makeAccount('g', 'gmail'),
      makeAccount('o', 'outlook'),
      makeAccount('i', 'imap'),
      makeAccount('i-feed', 'imap'),
      makeAccount('g-off', 'gmail', false),
    ];
    expect(calendarCapableAccounts(accounts, NO_FEEDS).map((a) => a.id)).toEqual(['g', 'o']);
    expect(calendarCapableAccounts(accounts, FEEDS).map((a) => a.id)).toEqual(['g', 'o', 'i-feed']);
  });

  it('enabled accounts are the capable ones the user opted in', () => {
    const accounts = [makeAccount('g', 'gmail'), makeAccount('o', 'outlook'), makeAccount('i', 'imap')];
    // A stray pref for an IMAP account without feeds must not resurrect it.
    const enabledIds = new Set(['g', 'i']);
    expect(calendarEnabledAccounts(accounts, enabledIds, NO_FEEDS).map((a) => a.id)).toEqual(['g']);
  });

  it('pref key matches the backend composite key', () => {
    expect(calendarEnabledPrefKey('acc-1')).toBe('calendar.enabled:acc-1');
  });
});

describe('calendarDisabledCurrentAccount', () => {
  const accounts = [
    makeAccount('g-on', 'gmail'),
    makeAccount('g-off', 'gmail'),
    makeAccount('i', 'imap'),
    makeAccount('g-dis', 'gmail', false),
  ];
  const enabledIds = new Set(['g-on']);

  it('returns the current account when it is capable but calendar-disabled', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'g-off', NO_FEEDS)?.id).toBe('g-off');
  });

  it('returns null when the current account already has calendar enabled', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'g-on', NO_FEEDS)).toBeNull();
  });

  it('returns null for IMAP accounts without a feed (nothing to enable yet)', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'i', NO_FEEDS)).toBeNull();
  });

  it('returns null for disabled accounts', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'g-dis', NO_FEEDS)).toBeNull();
  });

  it('returns null for an unknown or missing current account', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'nope', NO_FEEDS)).toBeNull();
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, null, NO_FEEDS)).toBeNull();
  });
});

describe('calendarPanelState', () => {
  const enabledIds = new Set(['g-on', 'i-feed']);

  it('is ready for an enabled account with a calendar', () => {
    expect(calendarPanelState(makeAccount('g-on', 'gmail'), enabledIds, FEEDS)).toBe('ready');
    expect(calendarPanelState(makeAccount('i-feed', 'imap'), enabledIds, FEEDS)).toBe('ready');
  });

  it('asks for setup on an IMAP account without feeds', () => {
    expect(calendarPanelState(makeAccount('i', 'imap'), enabledIds, FEEDS)).toBe('setup');
  });

  it('is disabled when the integration is switched off', () => {
    expect(calendarPanelState(makeAccount('g-off', 'gmail'), enabledIds, FEEDS)).toBe('disabled');
  });

  it('is none without an account or for a disabled account', () => {
    expect(calendarPanelState(null, enabledIds, FEEDS)).toBe('none');
    expect(calendarPanelState(makeAccount('g-on', 'gmail', false), enabledIds, FEEDS)).toBe('none');
  });
});

describe('feeds reducers', () => {
  it('feedsAfterAdd appends to the account without touching other accounts', () => {
    const next = feedsAfterAdd(FEEDS, makeFeed('f2', 'i-feed'));
    expect(next['i-feed'].map((f) => f.id)).toEqual(['f1', 'f2']);
    expect(feedsAfterAdd(FEEDS, makeFeed('f9', 'other')).other.map((f) => f.id)).toEqual(['f9']);
    expect(FEEDS['i-feed']).toHaveLength(1); // input untouched
  });

  it('feedsAfterRemove drops only that feed', () => {
    const two = feedsAfterAdd(FEEDS, makeFeed('f2', 'i-feed'));
    expect(feedsAfterRemove(two, 'i-feed', 'f1')['i-feed'].map((f) => f.id)).toEqual(['f2']);
    expect(feedsAfterRemove(two, 'i-feed', 'nope')).toEqual(two);
  });
});

describe('calendarIntegrationStore', () => {
  beforeEach(() => {
    useCalendarIntegrationStore.setState({ enabledIds: new Set(), isLoaded: false, feedsByAccount: {} });
    vi.mocked(api.getPref).mockReset();
    vi.mocked(api.setPref).mockReset();
    vi.mocked(api.listCalendarFeeds).mockReset().mockResolvedValue([]);
    vi.mocked(api.addCalendarFeed).mockReset();
    vi.mocked(api.removeCalendarFeed).mockReset();
  });

  it('loads feeds for IMAP accounts and probes the pref only for accounts with a calendar', async () => {
    vi.mocked(api.listCalendarFeeds).mockImplementation(async (id: string) =>
      id === 'i-feed' ? [makeFeed('f1', 'i-feed')] : [],
    );
    vi.mocked(api.getPref).mockResolvedValue(null);

    await useCalendarIntegrationStore
      .getState()
      .loadForAccounts([makeAccount('g1', 'gmail'), makeAccount('i-feed', 'imap'), makeAccount('i-none', 'imap')]);

    const { enabledIds, feedsByAccount } = useCalendarIntegrationStore.getState();
    expect([...enabledIds]).toEqual(['g1', 'i-feed']);
    expect(feedsByAccount['i-feed'].map((f) => f.id)).toEqual(['f1']);
    expect(feedsByAccount['i-none']).toEqual([]);
    const feedCalls = vi.mocked(api.listCalendarFeeds).mock.calls.map(([id]) => id);
    expect(feedCalls).toEqual(['i-feed', 'i-none']); // never for Gmail
    const probedKeys = vi.mocked(api.getPref).mock.calls.map(([key]) => key);
    expect(probedKeys).not.toContain(calendarEnabledPrefKey('i-none'));
  });

  it('a feeds load failure counts as no feeds but still finishes the load', async () => {
    vi.mocked(api.listCalendarFeeds).mockRejectedValue(new Error('db closed'));
    vi.mocked(api.getPref).mockResolvedValue(null);

    await useCalendarIntegrationStore.getState().loadForAccounts([makeAccount('i', 'imap')]);

    const { isLoaded, feedsByAccount, enabledIds } = useCalendarIntegrationStore.getState();
    expect(isLoaded).toBe(true);
    expect(feedsByAccount.i).toEqual([]);
    expect(enabledIds.size).toBe(0);
  });

  it('addFeed stores the new feed and switches the account on by default', async () => {
    vi.mocked(api.addCalendarFeed).mockResolvedValue(makeFeed('f1', 'i'));
    vi.mocked(api.getPref).mockResolvedValue(null);

    const feed = await useCalendarIntegrationStore.getState().addFeed('i', 'Work', 'https://x/y.ics');

    expect(api.addCalendarFeed).toHaveBeenCalledWith('i', 'Work', 'https://x/y.ics');
    expect(feed.id).toBe('f1');
    const { feedsByAccount, enabledIds } = useCalendarIntegrationStore.getState();
    expect(feedsByAccount.i.map((f) => f.id)).toEqual(['f1']);
    expect(enabledIds.has('i')).toBe(true);
  });

  it('addFeed leaves the store untouched and rethrows when the backend rejects', async () => {
    vi.mocked(api.addCalendarFeed).mockRejectedValue(new Error('not a calendar'));

    await expect(useCalendarIntegrationStore.getState().addFeed('i', '', 'https://x')).rejects.toThrow(
      'not a calendar',
    );
    expect(useCalendarIntegrationStore.getState().feedsByAccount.i).toBeUndefined();
  });

  it('removeFeed drops the feed after the backend confirms', async () => {
    useCalendarIntegrationStore.setState({ feedsByAccount: { i: [makeFeed('f1', 'i'), makeFeed('f2', 'i')] } });
    vi.mocked(api.removeCalendarFeed).mockResolvedValue();

    await useCalendarIntegrationStore.getState().removeFeed('i', 'f1');

    expect(api.removeCalendarFeed).toHaveBeenCalledWith('i', 'f1');
    expect(useCalendarIntegrationStore.getState().feedsByAccount.i.map((f) => f.id)).toEqual(['f2']);
  });

  it('capable accounts are enabled by default; only an explicit "false" pref disables', async () => {
    vi.mocked(api.getPref).mockImplementation(async (key: string) =>
      key === calendarEnabledPrefKey('g2') ? 'false' : null,
    );

    await useCalendarIntegrationStore
      .getState()
      .loadForAccounts([makeAccount('g1', 'gmail'), makeAccount('g2', 'gmail'), makeAccount('i', 'imap')]);

    const { enabledIds, isLoaded } = useCalendarIntegrationStore.getState();
    expect(isLoaded).toBe(true);
    expect([...enabledIds]).toEqual(['g1']);
    // IMAP accounts are never probed.
    const probedKeys = vi.mocked(api.getPref).mock.calls.map(([key]) => key);
    expect(probedKeys).not.toContain(calendarEnabledPrefKey('i'));
  });

  it('a pref read failure counts as disabled but still finishes the load', async () => {
    vi.mocked(api.getPref).mockRejectedValue(new Error('db closed'));

    await useCalendarIntegrationStore.getState().loadForAccounts([makeAccount('g1', 'gmail')]);

    const { enabledIds, isLoaded } = useCalendarIntegrationStore.getState();
    expect(isLoaded).toBe(true);
    expect(enabledIds.size).toBe(0);
  });

  it('applyBackendChange flips the flag without touching prefs (backend already wrote them)', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    await useCalendarIntegrationStore.getState().loadForAccounts([makeAccount('g1', 'gmail')]);
    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(true);

    useCalendarIntegrationStore.getState().applyBackendChange('g1', false);

    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(false);
    expect(api.setPref).not.toHaveBeenCalled();
  });

  it('setEnabled persists the pref and updates the set', async () => {
    vi.mocked(api.setPref).mockResolvedValue();

    await useCalendarIntegrationStore.getState().setEnabled('g1', true);
    expect(api.setPref).toHaveBeenCalledWith(calendarEnabledPrefKey('g1'), 'true');
    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(true);

    await useCalendarIntegrationStore.getState().setEnabled('g1', false);
    expect(api.setPref).toHaveBeenCalledWith(calendarEnabledPrefKey('g1'), 'false');
    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(false);
  });

  it('setEnabled reverts the optimistic flip and rethrows when persisting fails', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));

    await expect(useCalendarIntegrationStore.getState().setEnabled('g1', true)).rejects.toThrow('disk full');
    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(false);
  });

  it('ignores a stale load finishing after a newer one', async () => {
    let resolveSlow: (value: string | null) => void = () => {};
    vi.mocked(api.getPref).mockImplementationOnce(
      () =>
        new Promise<string | null>((resolve) => {
          resolveSlow = resolve;
        }),
    );
    const slowLoad = useCalendarIntegrationStore.getState().loadForAccounts([makeAccount('g1', 'gmail')]);

    vi.mocked(api.getPref).mockResolvedValueOnce('false');
    await useCalendarIntegrationStore.getState().loadForAccounts([makeAccount('g1', 'gmail')]);
    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(false);

    resolveSlow(null); // stale "enabled by default" answer lands late — must be dropped
    await slowLoad;

    expect(useCalendarIntegrationStore.getState().enabledIds.has('g1')).toBe(false);
  });
});
