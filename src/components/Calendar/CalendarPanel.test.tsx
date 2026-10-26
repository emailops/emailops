// The right-docked calendar panel: today's agenda for the account the user is
// in, a prompt to subscribe a feed when that account is IMAP without one, and
// a way to the full calendar.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '10:00', date: () => 'today', dateTime: () => 'today 10:00' }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(() => Promise.resolve(null)),
  setPref: vi.fn(() => Promise.resolve()),
  getCalendarEvents: vi.fn(() => Promise.resolve([])),
  getCalendars: vi.fn(() => Promise.resolve([])),
  syncCalendarNow: vi.fn(() => Promise.resolve(0)),
  setCalendarVisible: vi.fn(),
  reauthenticateAccount: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { ALL_ACCOUNTS_ID, useAccountStore } from '@/stores/accountStore';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import type { Account, CalendarEvent, CalendarFeed } from '@/types';
import { CalendarPanel } from './CalendarPanel';

function makeAccount(id: string, provider: string): Account {
  return {
    id,
    provider,
    email: `${id}@example.com`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
  } as Account;
}

function todayAt(hour: number): number {
  const d = new Date();
  d.setHours(hour, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}

function makeEvent(id: string, title: string, startHour: number): CalendarEvent {
  return {
    id,
    accountId: 'g',
    providerEventId: id,
    calendarId: 'primary',
    title,
    description: '',
    location: '',
    startTime: todayAt(startHour),
    endTime: todayAt(startHour) + 3600,
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

const FEED: CalendarFeed = { id: 'f1', accountId: 'i-feed', label: 'Work', host: 'calendar.proton.me', createdAt: 0 };

describe('CalendarPanel', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onOpenSettings = vi.fn();
  const onOpenFullCalendar = vi.fn();
  const onClose = vi.fn();

  beforeEach(() => {
    useAccountStore.setState({ activeAccountId: null });
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    onOpenSettings.mockClear();
    onOpenFullCalendar.mockClear();
    vi.mocked(api.getCalendarEvents).mockReset().mockResolvedValue([]);
    useCalendarIntegrationStore.setState({
      isLoaded: true,
      enabledIds: new Set(['g', 'i-feed']),
      feedsByAccount: { 'i-feed': [FEED], 'i-none': [] },
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const accounts = [makeAccount('g', 'gmail'), makeAccount('i-feed', 'imap'), makeAccount('i-none', 'imap')];

  const mount = (defaultAccountId: string) =>
    act(async () =>
      root.render(
        <CalendarPanel
          accounts={accounts}
          defaultAccountId={defaultAccountId}
          onClose={onClose}
          onOpenFullCalendar={onOpenFullCalendar}
          onOpenSettings={onOpenSettings}
        />,
      ),
    );

  it('prompts to set up a feed for an IMAP account without one, with a button to Settings', async () => {
    await mount('i-none');
    expect(container.textContent).toContain('calendar:panel.setupTitle');
    const button = [...container.querySelectorAll('button')].find((b) =>
      b.textContent?.includes('calendar:panel.setupButton'),
    );
    expect(button).toBeDefined();
    await act(async () => button?.click());
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    expect(api.getCalendarEvents).not.toHaveBeenCalled();
  });

  it("shows today's events of the current account on an hour-by-hour grid", async () => {
    vi.mocked(api.getCalendarEvents).mockResolvedValue([makeEvent('e2', 'Lunch', 13), makeEvent('e1', 'Standup', 9)]);
    await mount('g');
    // The day is a time grid (like Gmail's side panel), not a flat list: an
    // hour gutter plus one positioned block per event.
    expect(container.querySelector('[data-testid="calendar-panel"] [data-testid="time-grid"]')).not.toBeNull();
    const blocks = [...container.querySelectorAll('button[title]')]
      .map((b) => b.getAttribute('title') ?? '')
      .filter((t) => /^(Standup|Lunch) ·/.test(t));
    expect(blocks).toHaveLength(2);
    expect(api.syncCalendarNow).toHaveBeenCalledWith('g');
  });

  it('shows an empty grid for a quiet day and opens the full calendar from its header button', async () => {
    await mount('i-feed');
    expect(container.querySelector('[data-testid="time-grid"]')).not.toBeNull();
    expect(container.querySelector('button[title*=" · "]')).toBeNull();
    const expand = container.querySelector<HTMLButtonElement>('button[aria-label="calendar:panel.openFull"]');
    expect(expand).not.toBeNull();
    await act(async () => expand?.click());
    expect(onOpenFullCalendar).toHaveBeenCalledTimes(1);
  });

  it('in "All accounts" shows every calendar account\'s day, even when the first account has no feed', async () => {
    useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => [
      { ...makeEvent(`${accountId}-1`, `${accountId} sync`, 9), accountId },
    ]);
    await mount('i-none');
    expect(container.textContent).not.toContain('calendar:panel.setupTitle');
    const titles = [...container.querySelectorAll('button[title]')].map((b) => b.getAttribute('title') ?? '');
    expect(titles.some((t) => t.startsWith('g sync'))).toBe(true);
    expect(titles.some((t) => t.startsWith('i-feed sync'))).toBe(true);
  });

  it('prompts to set up the feed for the sidebar account even after "All accounts" was picked', async () => {
    vi.mocked(api.getPref).mockResolvedValueOnce(ALL_ACCOUNTS_ID);
    await mount('i-none');
    expect(container.textContent).toContain('calendar:panel.setupTitle');
  });
});
