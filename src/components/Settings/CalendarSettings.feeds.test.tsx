// IMAP accounts get their calendar from subscribed ICS feeds (Proton Calendar
// "share via link"): Settings → Calendar lists them and lets the user add or
// remove one. The feed URL is only ever sent to the backend, never shown back.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts?.label ? `${key}:${opts.label}` : key),
  }),
}));

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getPref: vi.fn(() => Promise.resolve(null)),
  setPref: vi.fn(() => Promise.resolve()),
  getCalendars: vi.fn(() => Promise.resolve([])),
  syncCalendarNow: vi.fn(() => Promise.resolve(0)),
  listCalendarFeeds: vi.fn(() => Promise.resolve([])),
  addCalendarFeed: vi.fn(),
  removeCalendarFeed: vi.fn(() => Promise.resolve()),
}));

import * as api from '@/lib/api';
import { useAccountStore } from '@/stores/accountStore';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import type { Account, CalendarFeed } from '@/types';
import { CalendarSettings } from './CalendarSettings';

function account(id: string, provider: string): Account {
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

const WORK: CalendarFeed = { id: 'f1', accountId: 'i', label: 'Work', host: 'calendar.proton.me', createdAt: 0 };

describe('CalendarSettings — ICS feeds for IMAP accounts', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.mocked(api.addCalendarFeed).mockReset();
    vi.mocked(api.removeCalendarFeed).mockClear();
    useAccountStore.setState({ accounts: [account('g', 'gmail'), account('i', 'imap')] });
    useCalendarIntegrationStore.setState({ isLoaded: true, enabledIds: new Set(['g']), feedsByAccount: { i: [WORK] } });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const feedsSection = () => container.querySelector<HTMLElement>('[data-testid="calendar-feeds-i"]');
  const labelInput = () => feedsSection()?.querySelector<HTMLInputElement>('input[name="label"]');
  const urlInput = () => feedsSection()?.querySelector<HTMLInputElement>('input[name="url"]');
  const addButton = () => feedsSection()?.querySelector<HTMLButtonElement>('button[type="submit"]');
  const setValue = (input: HTMLInputElement, value: string) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  };

  it('lists the IMAP account feeds (label and host) and no feeds block for Gmail', async () => {
    await act(async () => root.render(<CalendarSettings />));
    const section = feedsSection();
    expect(section?.textContent).toContain('Work');
    expect(section?.textContent).toContain('calendar.proton.me');
    expect(container.querySelector('[data-testid="calendar-feeds-g"]')).toBeNull();
  });

  it('adds a feed with the trimmed label and URL and shows it', async () => {
    const added: CalendarFeed = { id: 'f2', accountId: 'i', label: 'Family', host: 'calendar.proton.me', createdAt: 1 };
    vi.mocked(api.addCalendarFeed).mockResolvedValue(added);
    await act(async () => root.render(<CalendarSettings />));

    await act(async () => {
      setValue(labelInput() as HTMLInputElement, '  Family ');
      setValue(urlInput() as HTMLInputElement, ' https://calendar.proton.me/api/calendar/v1/url/x/calendar.ics ');
    });
    await act(async () => addButton()?.click());

    expect(api.addCalendarFeed).toHaveBeenCalledWith(
      'i',
      'Family',
      'https://calendar.proton.me/api/calendar/v1/url/x/calendar.ics',
    );
    expect(feedsSection()?.textContent).toContain('Family');
    expect(urlInput()?.value).toBe('');
  });

  it('shows the backend error inline and keeps what was typed when adding fails', async () => {
    vi.mocked(api.addCalendarFeed).mockRejectedValue(new Error('calendar.proton.me did not return a calendar'));
    await act(async () => root.render(<CalendarSettings />));

    await act(async () => setValue(urlInput() as HTMLInputElement, 'https://calendar.proton.me/nope'));
    await act(async () => addButton()?.click());

    expect(feedsSection()?.textContent).toContain('did not return a calendar');
    expect(urlInput()?.value).toBe('https://calendar.proton.me/nope');
  });

  it('removes a feed through the backend', async () => {
    await act(async () => root.render(<CalendarSettings />));
    const remove = feedsSection()?.querySelector<HTMLButtonElement>(
      'button[aria-label="settings:calendar.removeFeedAria:Work"]',
    );
    expect(remove).not.toBeNull();

    await act(async () => remove?.click());

    expect(api.removeCalendarFeed).toHaveBeenCalledWith('i', 'f1');
    expect(feedsSection()?.textContent).not.toContain('Work');
  });
});
