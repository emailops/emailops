// On a phone the calendar header's five controls (Today, prev/next, the
// month/week/day segmented toggle, Refresh, the account selector) wrapped onto
// three rows and, with the calendar legend underneath, ate roughly a third of
// the screen before a single hour of the day was visible. Everything but
// navigation moves behind one overflow menu.

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
vi.mock('@/hooks/useResponsiveLayout', () => ({
  useResponsiveLayout: vi.fn(() => ({ isStacked: false, isMobile: false })),
}));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  setPref: vi.fn(),
  getCalendarEvents: vi.fn(),
  getCalendars: vi.fn(),
  setCalendarVisible: vi.fn(),
  syncCalendarNow: vi.fn(),
  reauthenticateAccount: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import { useResponsiveLayout } from '@/hooks/useResponsiveLayout';
import * as api from '@/lib/api';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import type { Account, Calendar } from '@/types';
import { CalendarView } from './CalendarView';

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

function makeCalendar(providerCalendarId: string, name: string): Calendar {
  return {
    id: providerCalendarId,
    accountId: 'a1',
    providerCalendarId,
    name,
    color: '',
    isPrimary: false,
    accessRole: 'owner',
    isVisible: true,
    sortOrder: 0,
    createdAt: 0,
    updatedAt: 0,
  };
}

const account = makeAccount('a1');
const twoCalendars = [makeCalendar('work', 'Work calendar'), makeCalendar('team', 'Team calendar')];

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
  vi.mocked(api.getCalendars).mockResolvedValue(twoCalendars);
  vi.mocked(api.syncCalendarNow).mockResolvedValue(0);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

async function mount(isStacked: boolean) {
  vi.mocked(useResponsiveLayout).mockReturnValue({ isStacked, isMobile: isStacked });
  useCalendarIntegrationStore.setState({ enabledIds: new Set(['a1']), isLoaded: true });
  await act(async () => {
    root.render(<CalendarView accounts={[account]} defaultAccountId="a1" />);
  });
  await act(async () => {}); // flush pref / event / calendar loads
}

function buttonWithText(text: string): HTMLButtonElement | null {
  return [...container.querySelectorAll('button')].find((b) => b.textContent === text) ?? null;
}

function menuToggle(): HTMLButtonElement | null {
  return container.querySelector<HTMLButtonElement>('button[aria-label="calendar:moreOptions"]');
}

describe('CalendarView header on a phone', () => {
  it('keeps every control inline on a desktop', async () => {
    await mount(false);

    expect(menuToggle()).toBeNull();
    expect(buttonWithText('calendar:viewModes.month')).not.toBeNull();
    expect(buttonWithText('common:actions.refresh')).not.toBeNull();
    expect(container.textContent).toContain('Work calendar');
  });

  it('collapses view modes, refresh and the account selector behind one menu', async () => {
    await mount(true);

    expect(buttonWithText('calendar:viewModes.month')).toBeNull();
    expect(buttonWithText('calendar:viewModes.week')).toBeNull();
    expect(buttonWithText('calendar:viewModes.day')).toBeNull();
    expect(buttonWithText('common:actions.refresh')).toBeNull();
    expect(menuToggle()).not.toBeNull();
  });

  it('leaves date navigation on the bar, where it is used constantly', async () => {
    await mount(true);

    expect(buttonWithText('calendar:today')).not.toBeNull();
    expect(container.querySelector('button[title="calendar:previous"]')).not.toBeNull();
    expect(container.querySelector('button[title="calendar:next"]')).not.toBeNull();
  });

  it('hides the calendar-name legend, reclaiming a whole row', async () => {
    await mount(true);

    expect(container.textContent).not.toContain('Work calendar');
    expect(container.textContent).not.toContain('Team calendar');
  });

  it('reveals the collapsed controls — including the calendar toggles — when the menu is opened', async () => {
    // Hiding the legend must not cost the user the ability to show/hide a
    // calendar; the toggles move into the menu rather than disappearing.
    await mount(true);

    await act(async () => {
      menuToggle()?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(buttonWithText('calendar:viewModes.month')).not.toBeNull();
    expect(buttonWithText('common:actions.refresh')).not.toBeNull();
    expect(container.textContent).toContain('Work calendar');
    expect(container.textContent).toContain('Team calendar');
  });

  it('closes the menu once a view mode is chosen', async () => {
    await mount(true);

    await act(async () => {
      menuToggle()?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    await act(async () => {
      buttonWithText('calendar:viewModes.month')?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(buttonWithText('calendar:viewModes.month')).toBeNull();
  });
});
