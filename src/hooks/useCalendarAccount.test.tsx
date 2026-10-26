// The calendar's account selection follows the sidebar selection whenever a
// calendar surface opens; a manual pick in the selector lasts while that
// surface stays open, or until the sidebar selection changes.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const prefs = vi.hoisted(() => ({ stored: {} as Record<string, string> }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async (key: string) => prefs.stored[key] ?? null),
  setPref: vi.fn(async (key: string, value: string) => {
    prefs.stored[key] = value;
  }),
  currentPlatform: vi.fn(() => ''),
}));

import { ALL_ACCOUNTS_ID, useAccountStore } from '@/stores/accountStore';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import type { Account } from '@/types';
import { useCalendarAccount } from './useCalendarAccount';

function makeAccount(id: string): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@example.com`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
  } as Account;
}

const accounts = [makeAccount('g1'), makeAccount('g2'), makeAccount('g3')];

let latest: ReturnType<typeof useCalendarAccount> | null = null;
function Probe({ defaultAccountId }: { defaultAccountId: string | null }) {
  latest = useCalendarAccount(accounts, defaultAccountId);
  return <span>{latest.selectedAccountId ?? ''}</span>;
}

describe('useCalendarAccount', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    prefs.stored = {};
    useAccountStore.setState({ activeAccountId: 'g1' });
    useCalendarIntegrationStore.setState({
      isLoaded: true,
      enabledIds: new Set(['g1', 'g2', 'g3']),
      feedsByAccount: {},
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('opens on the sidebar account', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    expect(container.textContent).toBe('g1');
  });

  it('reopening the calendar shows the sidebar account again, not an earlier manual pick', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount(ALL_ACCOUNTS_ID));
    expect(container.textContent).toBe(ALL_ACCOUNTS_ID);

    // The calendar closes and opens again: a fresh mount of the hook.
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Probe defaultAccountId="g1" />));

    expect(container.textContent).toBe('g1');
  });

  it('keeps a manual pick while the calendar stays open', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount('g2'));
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    expect(container.textContent).toBe('g2');
  });

  it('follows the sidebar account again once it changes', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount('g2'));
    await act(async () => root.render(<Probe defaultAccountId="g3" />));
    expect(container.textContent).toBe('g3');
  });

  it('follows the sidebar into "All accounts"', async () => {
    useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    expect(container.textContent).toBe(ALL_ACCOUNTS_ID);
    expect(latest?.isAllAccounts).toBe(true);
    expect(latest?.selectedAccount).toBeNull();
  });

  it('can be switched to "All accounts" from the selector', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount(ALL_ACCOUNTS_ID));
    expect(container.textContent).toBe(ALL_ACCOUNTS_ID);
  });

  it('shows the single account when only one has a calendar, even in "All accounts"', async () => {
    useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
    useCalendarIntegrationStore.setState({ enabledIds: new Set(['g2']) });
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    expect(container.textContent).toBe('g2');
  });

  it('does not offer to enable the first account\'s calendar when the sidebar is on "All accounts"', async () => {
    useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
    useCalendarIntegrationStore.setState({ enabledIds: new Set(['g2', 'g3']) });
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    expect(latest?.disabledCurrentAccount).toBeNull();
  });

  it('maximizing hands the pick over to the full view', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount('g2'));
    act(() => latest?.handOffSelection());

    // The agenda closes and the full view opens: a fresh mount of the hook.
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Probe defaultAccountId="g1" />));

    expect(container.textContent).toBe('g2');
  });

  it('a handed-over pick gives way to a sidebar account changed in between', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount('g2'));
    act(() => latest?.handOffSelection());

    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Probe defaultAccountId="g3" />));

    expect(container.textContent).toBe('g3');
  });

  it('a hand-over is used once: opening again later follows the sidebar', async () => {
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    await act(async () => latest?.handleSelectAccount('g2'));
    act(() => latest?.handOffSelection());
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Probe defaultAccountId="g1" />));
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Probe defaultAccountId="g1" />));

    expect(container.textContent).toBe('g1');
  });
});
