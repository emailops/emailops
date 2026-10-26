// The calendar data hook feeds both the full view and the day-agenda panel.
// Its one subtle invariant: a sync that finishes after the user switched
// account must not reload the old account's events over the new one's.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getCalendarEvents: vi.fn(),
  getCalendars: vi.fn(() => Promise.resolve([])),
  syncCalendarNow: vi.fn(),
  setCalendarVisible: vi.fn(),
  reauthenticateAccount: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import type { CalendarEvent } from '@/types';
import { useCalendarData } from './useCalendarData';

function makeEvent(id: string, accountId: string): CalendarEvent {
  return {
    id,
    accountId,
    providerEventId: id,
    calendarId: 'primary',
    title: id,
    description: '',
    location: '',
    startTime: 1_000,
    endTime: 2_000,
    isAllDay: false,
    timezone: '',
    organizer: '',
    attendees: [],
    meetingLink: null,
    meetingPlatform: null,
    status: 'confirmed',
    htmlLink: null,
    notifiedAt: null,
    recurringEventId: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

let latest: ReturnType<typeof useCalendarData> | null = null;
function Probe({ accountId, accountIds }: { accountId?: string | null; accountIds?: string[] }) {
  latest = useCalendarData(accountIds ?? (accountId ? [accountId] : []), 0, 10_000);
  return <div data-testid="titles">{latest.events.map((e) => e.title).join(',')}</div>;
}

describe('useCalendarData', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    latest = null;
    vi.mocked(api.getCalendarEvents).mockReset();
    vi.mocked(api.syncCalendarNow).mockReset();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('syncs on open and shows the account events', async () => {
    vi.mocked(api.syncCalendarNow).mockResolvedValue(1);
    vi.mocked(api.getCalendarEvents).mockResolvedValue([makeEvent('a1', 'a')]);

    await act(async () => root.render(<Probe accountId="a" />));

    expect(api.syncCalendarNow).toHaveBeenCalledWith('a');
    expect(container.textContent).toBe('a1');
    expect(latest?.isSyncing).toBe(false);
  });

  it('a sync finishing after an account switch does not overwrite the new account', async () => {
    let finishSlowSync: (n: number) => void = () => {};
    vi.mocked(api.syncCalendarNow).mockImplementation((accountId: string) =>
      accountId === 'a'
        ? new Promise<number>((resolve) => {
            finishSlowSync = resolve;
          })
        : Promise.resolve(0),
    );
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => [
      makeEvent(`${accountId}-event`, accountId),
    ]);

    await act(async () => root.render(<Probe accountId="a" />));
    await act(async () => root.render(<Probe accountId="b" />));
    expect(container.textContent).toBe('b-event');

    await act(async () => {
      finishSlowSync(5);
    });

    expect(container.textContent).toBe('b-event');
    expect(latest?.isSyncing).toBe(false);
  });

  it('an auth failure raises the re-auth flag instead of the error text', async () => {
    vi.mocked(api.syncCalendarNow).mockRejectedValue(new Error('Authentication required for account a'));
    vi.mocked(api.getCalendarEvents).mockResolvedValue([]);

    await act(async () => root.render(<Probe accountId="a" />));

    expect(latest?.reauthAccountIds).toEqual(['a']);
    expect(latest?.error).toBeNull();
  });

  it('"All accounts" syncs and shows every account\'s events together', async () => {
    vi.mocked(api.syncCalendarNow).mockResolvedValue(1);
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => [
      makeEvent(`${accountId}-event`, accountId),
    ]);

    await act(async () => root.render(<Probe accountIds={['a', 'b']} />));

    expect(api.syncCalendarNow).toHaveBeenCalledWith('a');
    expect(api.syncCalendarNow).toHaveBeenCalledWith('b');
    expect(container.textContent).toBe('a-event,b-event');
  });

  it('one account needing re-auth is named, and the others still show', async () => {
    vi.mocked(api.syncCalendarNow).mockImplementation((accountId: string) =>
      accountId === 'a' ? Promise.reject(new Error('Authentication required for account a')) : Promise.resolve(1),
    );
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => [
      makeEvent(`${accountId}-event`, accountId),
    ]);

    await act(async () => root.render(<Probe accountIds={['a', 'b']} />));

    expect(latest?.reauthAccountIds).toEqual(['a']);
    expect(container.textContent).toBe('a-event,b-event');
  });

  it('one account failing to sync is reported against that account', async () => {
    vi.mocked(api.syncCalendarNow).mockImplementation((accountId: string) =>
      accountId === 'b' ? Promise.reject(new Error('provider down')) : Promise.resolve(1),
    );
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => [
      makeEvent(`${accountId}-event`, accountId),
    ]);

    await act(async () => root.render(<Probe accountIds={['a', 'b']} />));

    expect(latest?.syncErrors).toEqual([{ accountId: 'b', message: 'provider down' }]);
    expect(container.textContent).toBe('a-event,b-event');
  });

  it("an account whose events fail to load does not hide the others'", async () => {
    vi.mocked(api.syncCalendarNow).mockResolvedValue(1);
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => {
      if (accountId === 'a') throw new Error('db busy');
      return [makeEvent(`${accountId}-event`, accountId)];
    });

    await act(async () => root.render(<Probe accountIds={['a', 'b']} />));

    expect(container.textContent).toBe('b-event');
    expect(latest?.error).toBe('db busy');
  });
});
