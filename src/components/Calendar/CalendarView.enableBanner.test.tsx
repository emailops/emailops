// When the user opens the calendar while "in" an account whose calendar
// integration is switched off (e.g. the backend auto-disabled it on a
// permission failure), the view must not silently fall back to another
// account: it shows a centered banner naming the account with an inline
// "Enable calendar" button that flips the pref back on and selects it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    // Surface interpolated email so tests can assert the banner names the account.
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
  syncCalendarNow: vi.fn(),
  reauthenticateAccount: vi.fn(),
}));

import * as api from '@/lib/api';
import { calendarEnabledPrefKey, useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import type { Account, CalendarEvent } from '@/types';
import { CalendarView } from './CalendarView';

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

const gmailOn = makeAccount('g-on', 'gmail');
const gmailOff = makeAccount('g-off', 'gmail');
const gmailOther = makeAccount('g-other', 'gmail');

function makeEvent(id: string, accountId: string, title: string): CalendarEvent {
  const now = Math.floor(Date.now() / 1000);
  return {
    id,
    accountId,
    providerEventId: id,
    calendarId: 'cal',
    title,
    description: '',
    location: '',
    startTime: now,
    endTime: now + 3600,
    isAllDay: false,
    timezone: '',
    organizer: '',
    attendees: [],
    meetingLink: null,
    meetingPlatform: null,
    status: 'confirmed',
    htmlLink: null,
  } as unknown as CalendarEvent;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.stubGlobal('requestAnimationFrame', () => 0);
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.mocked(api.getPref).mockResolvedValue(null);
  vi.mocked(api.setPref).mockResolvedValue();
  vi.mocked(api.getCalendarEvents).mockResolvedValue([]);
  vi.mocked(api.syncCalendarNow).mockResolvedValue(0);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.mocked(api.getPref).mockReset();
  vi.mocked(api.setPref).mockReset();
  vi.mocked(api.getCalendarEvents).mockReset();
  vi.mocked(api.syncCalendarNow).mockReset();
});

async function mount(accounts: Account[], enabledIds: string[], defaultAccountId: string | null) {
  useCalendarIntegrationStore.setState({ enabledIds: new Set(enabledIds), isLoaded: true });
  await act(async () => {
    root.render(<CalendarView accounts={accounts} defaultAccountId={defaultAccountId} />);
  });
  await act(async () => {}); // flush pref/event loads
}

function enableButton(): HTMLButtonElement | null {
  return (
    [...container.querySelectorAll('button')].find((b) => b.textContent === 'calendar:enableCalendarButton') ?? null
  );
}

function overlay(): HTMLElement | null {
  const el = container.querySelector('.backdrop-blur-sm');
  return el instanceof HTMLElement ? el : null;
}

describe('CalendarView enable-calendar banner', () => {
  it('shows the banner naming the current account when its calendar is disabled', async () => {
    await mount([gmailOn, gmailOff], ['g-on'], 'g-off');

    expect(container.textContent).toContain('calendar:integrationDisabled|g-off@example.com');
    expect(enableButton()).not.toBeNull();
  });

  it('renders the message as a centered overlay that freezes the calendar behind it', async () => {
    await mount([gmailOn, gmailOff], ['g-on'], 'g-off');

    const el = overlay();
    expect(el).not.toBeNull();
    // Full-cover frost over the calendar, centering its card both ways.
    for (const cls of ['absolute', 'inset-0', 'items-center', 'justify-center']) {
      expect(el?.className).toContain(cls);
    }
    // Message and button live inside the overlay, not in a top bar.
    expect(el?.textContent).toContain('calendar:integrationDisabled|g-off@example.com');
    expect(el?.querySelector('button')?.textContent).toBe('calendar:enableCalendarButton');
  });

  it('shows the banner even when no account has calendar enabled (empty state)', async () => {
    await mount([gmailOff], [], 'g-off');

    expect(container.textContent).toContain('calendar:integrationDisabled|g-off@example.com');
    expect(enableButton()).not.toBeNull();
    // The generic "enable it in Settings" hint still renders underneath.
    expect(container.textContent).toContain('calendar:noAccount');
  });

  it('does not show the banner when the current account has calendar enabled', async () => {
    await mount([gmailOn, gmailOff], ['g-on'], 'g-on');

    expect(container.textContent).not.toContain('calendar:integrationDisabled');
    expect(enableButton()).toBeNull();
  });

  it('does not show the banner for a current account with no calendar support (IMAP)', async () => {
    await mount([gmailOn, makeAccount('i', 'imap')], ['g-on'], 'i');

    expect(container.textContent).not.toContain('calendar:integrationDisabled');
  });

  it('clicking Enable turns the integration on and selects the account', async () => {
    await mount([gmailOn, gmailOff], ['g-on'], 'g-off');

    const button = enableButton();
    expect(button).not.toBeNull();
    await act(async () => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    await act(async () => {});

    expect(api.setPref).toHaveBeenCalledWith(calendarEnabledPrefKey('g-off'), 'true');
    expect(useCalendarIntegrationStore.getState().enabledIds.has('g-off')).toBe(true);
    // The view switches to the freshly enabled account…
    expect(api.setPref).toHaveBeenCalledWith('calendar_selected_account', 'g-off');
    // …and the banner is gone.
    expect(container.textContent).not.toContain('calendar:integrationDisabled');
  });
});

describe('CalendarView selector follows the current account', () => {
  function accountSelect(): HTMLSelectElement {
    const el = container.querySelector('select');
    if (!(el instanceof HTMLSelectElement)) throw new Error('account selector not rendered');
    return el;
  }

  it('opening the view switches the selector to the account the user is in', async () => {
    // A different account was persisted from a previous visit…
    vi.mocked(api.getPref).mockImplementation(async (key: string) =>
      key === 'calendar_selected_account' ? 'g-on' : null,
    );
    await mount([gmailOn, gmailOther], ['g-on', 'g-other'], 'g-other');

    // …but the view opens on the account the user is currently in.
    expect(accountSelect().value).toBe('g-other');
  });

  it('keeps the persisted selection when the current account has no enabled calendar', async () => {
    vi.mocked(api.getPref).mockImplementation(async (key: string) =>
      key === 'calendar_selected_account' ? 'g-on' : null,
    );
    await mount([gmailOn, gmailOther, makeAccount('i', 'imap')], ['g-on', 'g-other'], 'i');

    expect(accountSelect().value).toBe('g-on');
  });

  it('a late sync for the previously selected account cannot overwrite the events', async () => {
    // The persisted account's sync hangs (slow provider); the auto-switched
    // account's sync completes immediately.
    let resolveOldSync: (n: number) => void = () => {};
    vi.mocked(api.syncCalendarNow).mockImplementation((accountId: string) =>
      accountId === 'g-on'
        ? new Promise<number>((resolve) => {
            resolveOldSync = resolve;
          })
        : Promise.resolve(0),
    );
    vi.mocked(api.getCalendarEvents).mockImplementation(async (accountId: string) =>
      accountId === 'g-on'
        ? [makeEvent('e-on', 'g-on', 'stale-account-event')]
        : [makeEvent('e-other', 'g-other', 'current-account-event')],
    );
    vi.mocked(api.getPref).mockImplementation(async (key: string) =>
      key === 'calendar_selected_account' ? 'g-on' : null,
    );

    await mount([gmailOn, gmailOther], ['g-on', 'g-other'], 'g-other');
    expect(accountSelect().value).toBe('g-other');
    expect(container.textContent).toContain('current-account-event');

    // The stale sync finishes AFTER the view moved on to the other account.
    await act(async () => {
      resolveOldSync(0);
    });
    await act(async () => {});

    expect(container.textContent).toContain('current-account-event');
    expect(container.textContent).not.toContain('stale-account-event');
  });

  it('a manual selector change sticks after the auto-switch', async () => {
    await mount([gmailOn, gmailOther], ['g-on', 'g-other'], 'g-other');
    expect(accountSelect().value).toBe('g-other');

    await act(async () => {
      accountSelect().value = 'g-on';
      accountSelect().dispatchEvent(new Event('change', { bubbles: true }));
    });

    expect(accountSelect().value).toBe('g-on');
    expect(api.setPref).toHaveBeenCalledWith('calendar_selected_account', 'g-on');
  });
});
