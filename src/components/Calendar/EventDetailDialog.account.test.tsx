// In "All accounts" one grid mixes several accounts' events, so the detail
// names the account an event belongs to.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '10:00', date: () => '28/07/2026', dateTime: () => '28/07/2026 10:00' }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));
vi.mock('@/lib/api', () => ({ deleteCalendarEvent: vi.fn(), currentPlatform: vi.fn(() => '') }));

import type { CalendarEvent } from '@/types';
import { EventDetailDialog } from './EventDetailDialog';

const EVENT = {
  id: 'acc:primary:e1',
  accountId: 'acc',
  calendarId: 'primary',
  title: 'Standup',
  description: '',
  location: '',
  startTime: 1_785_312_000,
  endTime: 1_785_315_600,
  isAllDay: false,
  organizer: '',
  attendees: [],
  meetingLink: null,
  htmlLink: null,
  status: 'confirmed',
  recurringEventId: null,
} as unknown as CalendarEvent;

describe('EventDetailDialog — account', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const render = (accountEmail?: string) =>
    act(async () =>
      root.render(
        <EventDetailDialog
          event={EVENT}
          accountId="acc"
          accountEmail={accountEmail}
          provider="gmail"
          onClose={() => {}}
          onDeleted={() => {}}
          onAuthError={() => {}}
        />,
      ),
    );

  it('names the account when given one', async () => {
    await render('work@mail.example');
    expect(container.textContent).toContain('calendar:detail.account');
    expect(container.textContent).toContain('work@mail.example');
  });

  it('shows no account row for a single-account calendar', async () => {
    await render();
    expect(container.textContent).not.toContain('calendar:detail.account');
  });
});
