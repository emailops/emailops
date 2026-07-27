import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  setPref: vi.fn(),
}));

import * as api from '@/lib/api';
import type { Account } from '@/types';
import {
  calendarCapableAccounts,
  calendarDisabledCurrentAccount,
  calendarEnabledAccounts,
  calendarEnabledPrefKey,
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

describe('calendar integration pure helpers', () => {
  it('capable accounts are enabled Gmail/Outlook only', () => {
    const accounts = [
      makeAccount('g', 'gmail'),
      makeAccount('o', 'outlook'),
      makeAccount('i', 'imap'),
      makeAccount('g-off', 'gmail', false),
    ];
    expect(calendarCapableAccounts(accounts).map((a) => a.id)).toEqual(['g', 'o']);
  });

  it('enabled accounts are the capable ones the user opted in', () => {
    const accounts = [makeAccount('g', 'gmail'), makeAccount('o', 'outlook'), makeAccount('i', 'imap')];
    // A stray pref for an IMAP account must not resurrect it.
    const enabledIds = new Set(['g', 'i']);
    expect(calendarEnabledAccounts(accounts, enabledIds).map((a) => a.id)).toEqual(['g']);
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
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'g-off')?.id).toBe('g-off');
  });

  it('returns null when the current account already has calendar enabled', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'g-on')).toBeNull();
  });

  it('returns null for IMAP accounts (no calendar support to enable)', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'i')).toBeNull();
  });

  it('returns null for disabled accounts', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'g-dis')).toBeNull();
  });

  it('returns null for an unknown or missing current account', () => {
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, 'nope')).toBeNull();
    expect(calendarDisabledCurrentAccount(accounts, enabledIds, null)).toBeNull();
  });
});

describe('calendarIntegrationStore', () => {
  beforeEach(() => {
    useCalendarIntegrationStore.setState({ enabledIds: new Set(), isLoaded: false });
    vi.mocked(api.getPref).mockReset();
    vi.mocked(api.setPref).mockReset();
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
