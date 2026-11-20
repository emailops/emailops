// "All accounts" shows every calendar account's events on one grid, and a
// failing account is named without hiding the others.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) =>
      opts && typeof opts.email === 'string' ? `${key}|${opts.email}` : key,
    i18n: { language: 'en' },
  }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '00:00', date: () => '', dateTime: () => '' }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  setPref: vi.fn(),
  getCalendarEvents: vi.fn(),
  getCalendars: vi.fn(),
  syncCalendarNow: vi.fn(),
  reauthenticateAccount: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { ALL_ACCOUNTS_ID, useAccountStore } from '@/stores/accountStore';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import type { Account, CalendarEvent } from '@/types';
import { CalendarView } from './CalendarView';

function makeAccount(id: string): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@mail.example`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
    color: 'blue',
  };
}

function makeEvent(accountId: string, title: string): CalendarEvent {
  const tenAm = new Date();
  tenAm.setHours(10, 0, 0, 0);
  const start = Math.floor(tenAm.getTime() / 1000);
  return {
    id: `${accountId}-${title}`,
    accountId,
    providerEventId: title,
    calendarId: 'primary',
    title,
    description: '',
    location: '',
    startTime: start,
    endTime: start + 3600,
    isAllDay: false,
    attendees: [],
    status: 'confirmed',
  } as unknown as CalendarEvent;
}

const work = makeAccount('work');
const home = makeAccount('home');

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useAccountStore.setState({ activeAccountId: ALL_ACCOUNTS_ID });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.stubGlobal('requestAnimationFrame', () => 0);
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.mocked(api.getPref).mockResolvedValue(null);
  vi.mocked(api.setPref).mockResolvedValue();
  vi.mocked(api.getCalendars).mockResolvedValue([]);
  vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) => [
    makeEvent(accountId, `${accountId} meeting`),
  ]);
  vi.mocked(api.syncCalendarNow).mockResolvedValue(0);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

async function mount() {
  useCalendarIntegrationStore.setState({ enabledIds: new Set(['work', 'home']), isLoaded: true, feedsByAccount: {} });
  await act(async () => {
    root.render(<CalendarView accounts={[work, home]} defaultAccountId="work" onOpenSettings={() => {}} />);
  });
  await act(async () => {});
}

describe('CalendarView — All accounts', () => {
  it("shows every account's events together when the sidebar is on All accounts", async () => {
    await mount();
    expect(container.textContent).toContain('work meeting');
    expect(container.textContent).toContain('home meeting');
    expect(container.querySelector('[aria-label="calendar:selectAccount"]')?.textContent).toContain(
      'calendar:allAccounts',
    );
  });

  it('names the account that needs re-auth and keeps the others shown', async () => {
    vi.mocked(api.syncCalendarNow).mockImplementation((accountId: string) =>
      accountId === 'home' ? Promise.reject(new Error('Authentication required')) : Promise.resolve(0),
    );
    await mount();
    expect(container.textContent).toContain('calendar:reauthNeededAccount|home@mail.example');
    expect(container.textContent).toContain('work meeting');
  });
});

describe('CalendarView — Import .ics button', () => {
  const calendar = (accountId: string, accessRole: string, feedHost: string | null) =>
    ({
      id: `${accountId}-cal`,
      accountId,
      providerCalendarId: 'primary',
      name: 'Calendar',
      color: '',
      isPrimary: true,
      accessRole,
      isVisible: true,
      sortOrder: 0,
      ownerEmail: null,
      feedHost,
    }) as never;

  // An IMAP account whose calendar is a subscribed Proton feed (read only).
  const proton: Account = { ...makeAccount('proton'), provider: 'imap' };
  const protonFeeds = { proton: [{ id: 'f1' }] } as never;

  it('import_button_hidden_without_writable_accounts', async () => {
    vi.mocked(api.getCalendars).mockResolvedValue([calendar('proton', 'reader', 'calendar.proton.me')]);
    useAccountStore.setState({ activeAccountId: 'proton' });
    useCalendarIntegrationStore.setState({
      enabledIds: new Set(['proton']),
      isLoaded: true,
      feedsByAccount: protonFeeds,
    });
    await act(async () => {
      root.render(<CalendarView accounts={[proton]} defaultAccountId="proton" onOpenSettings={() => {}} />);
    });
    await act(async () => {});
    expect(container.textContent).toContain('common:actions.refresh');
    expect(container.textContent).not.toContain('calendar:import.button');
  });

  it('shows the import button when an account can take events', async () => {
    vi.mocked(api.getCalendars).mockResolvedValue([calendar('work', 'owner', null)]);
    await mount();
    expect(container.textContent).toContain('calendar:import.button');
  });

  it('shows the import button on a feed account when a Gmail account has a calendar', async () => {
    vi.mocked(api.getCalendars).mockResolvedValue([calendar('proton', 'reader', 'calendar.proton.me')]);
    useAccountStore.setState({ activeAccountId: 'proton' });
    useCalendarIntegrationStore.setState({
      enabledIds: new Set(['proton', 'work']),
      isLoaded: true,
      feedsByAccount: protonFeeds,
    });
    await act(async () => {
      root.render(<CalendarView accounts={[proton, work]} defaultAccountId="proton" onOpenSettings={() => {}} />);
    });
    await act(async () => {});
    expect(container.querySelector('[aria-label="calendar:selectAccount"]')?.textContent).toContain(
      'proton@mail.example',
    );
    expect(container.textContent).toContain('calendar:import.button');
  });
});
