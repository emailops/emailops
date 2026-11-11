// Events of a subscribed feed cannot be deleted from here: the dialog hides
// its delete action instead of offering one the backend would reject.
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

const EVENT: CalendarEvent = {
  id: 'acc:feed-1:e1',
  accountId: 'acc',
  providerEventId: 'e1',
  calendarId: 'feed-1',
  title: 'Standup',
  description: '',
  location: '',
  startTime: 1_785_312_000,
  endTime: 1_785_315_600,
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

describe('EventDetailDialog — read-only calendars', () => {
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

  const deleteButton = () =>
    [...container.querySelectorAll('button')].find((b) => b.textContent?.includes('calendar:detail.delete'));

  const render = (readOnly: boolean) =>
    act(async () =>
      root.render(
        <EventDetailDialog
          event={EVENT}
          accountId="acc"
          provider="imap"
          readOnly={readOnly}
          onClose={() => {}}
          onDeleted={() => {}}
          onAuthError={() => {}}
        />,
      ),
    );

  it('offers delete on a writable calendar', async () => {
    await render(false);
    expect(deleteButton()).toBeDefined();
  });

  it('hides delete on a read-only calendar but still shows the event', async () => {
    await render(true);
    expect(deleteButton()).toBeUndefined();
    expect(container.textContent).toContain('Standup');
  });
});
