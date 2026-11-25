// Deleting from the event detail: only the organizer can cancel for everyone.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '10:00', date: () => '28/07/2026', dateTime: () => '28/07/2026 10:00' }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));
vi.mock('@/lib/api', () => ({
  deleteCalendarEvent: vi.fn(),
  respondToCalendarEvent: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { useAccountStore } from '@/stores/accountStore';
import type { Account, CalendarEvent } from '@/types';
import { EventDetailDialog } from './EventDetailDialog';

const make = (organizer: string, attendees: CalendarEvent['attendees']): CalendarEvent =>
  ({
    id: 'acc:primary:e1',
    accountId: 'acc',
    calendarId: 'primary',
    providerEventId: 'e1',
    title: 'Weekly sync',
    description: '',
    location: '',
    startTime: 1_785_312_000,
    endTime: 1_785_315_600,
    isAllDay: false,
    organizer,
    attendees,
    meetingLink: null,
    htmlLink: null,
    status: 'confirmed',
    recurringEventId: null,
  }) as unknown as CalendarEvent;

describe('EventDetailDialog — delete', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    useAccountStore.setState({ accounts: [{ id: 'acc', email: 'Me@example.com' } as Account] });
    vi.mocked(api.deleteCalendarEvent).mockReset();
    vi.mocked(api.deleteCalendarEvent).mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const openDeleteConfirm = async (event: CalendarEvent) => {
    await act(async () =>
      root.render(
        <EventDetailDialog
          event={event}
          accountId="acc"
          provider="outlook"
          readOnly={false}
          onClose={() => {}}
          onDeleted={() => {}}
          onResponded={() => {}}
          onAuthError={() => {}}
        />,
      ),
    );
    const byText = (text: string) =>
      [...container.querySelectorAll('button')].find((b) => b.textContent?.includes(text));
    await act(async () => byText('calendar:detail.delete')?.click());
    return () => act(async () => byText('calendar:detail.deleteButton')?.click());
  };

  const notifyCheckbox = () => container.querySelector('input[type="checkbox"]');

  it('a_meeting_i_was_invited_to_is_removed_without_notifying', async () => {
    const confirm = await openDeleteConfirm(
      make('lead@example.com', [
        { email: 'lead@example.com', response: 'organizer' },
        { email: 'me@example.com', response: 'accepted' },
      ]),
    );

    expect(notifyCheckbox()).toBeNull();
    expect(container.querySelector('#event-cancel-message')).toBeNull();
    await confirm();
    expect(api.deleteCalendarEvent).toHaveBeenCalledWith('acc', 'primary', 'e1', false, '', 'instance');
  });

  it('my_meeting_with_guests_offers_to_notify_them', async () => {
    const confirm = await openDeleteConfirm(
      make('me@example.com', [{ email: 'ana@example.com', response: 'accepted' }]),
    );

    expect(notifyCheckbox()).not.toBeNull();
    await confirm();
    expect(vi.mocked(api.deleteCalendarEvent).mock.calls[0]?.[3]).toBe(true);
  });
});
