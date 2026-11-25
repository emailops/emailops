// Mail icons in the event detail: write to the organizer / to everyone else.
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

import { useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
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

describe('EventDetailDialog — contact icons', () => {
  let container: HTMLDivElement;
  let root: Root;
  const openComposeTab = vi.fn();
  const onClose = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    useAccountStore.setState({ accounts: [{ id: 'acc', email: 'Me@example.com' } as Account] });
    useEmailStore.setState({ openComposeTab });
    openComposeTab.mockReset();
    onClose.mockReset();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const render = (event: CalendarEvent, readOnly = false) =>
    act(async () =>
      root.render(
        <EventDetailDialog
          event={event}
          accountId="acc"
          provider="gmail"
          readOnly={readOnly}
          onClose={onClose}
          onDeleted={() => {}}
          onResponded={() => {}}
          onAuthError={() => {}}
        />,
      ),
    );

  const button = (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  const guests = [
    { email: 'me@example.com', response: 'needsAction' },
    { email: 'ana@example.com', response: 'accepted' },
  ];

  it('mail_organizer_opens_compose_with_title', async () => {
    await render(make('lead@example.com', guests));
    const btn = button('calendar:detail.mailOrganizer');
    expect(btn?.getAttribute('title')).toBe('calendar:detail.mailOrganizer');
    await act(async () => btn?.click());
    expect(openComposeTab).toHaveBeenCalledWith('acc', ['lead@example.com'], 'Weekly sync');
    expect(onClose).toHaveBeenCalled();
  });

  it('mail_participants_opens_compose_with_everyone_else', async () => {
    await render(make('lead@example.com', guests));
    await act(async () => button('calendar:detail.mailParticipants')?.click());
    expect(openComposeTab).toHaveBeenCalledWith('acc', ['lead@example.com', 'ana@example.com'], 'Weekly sync');
  });

  it('mail_participants_hidden_when_nobody_else', async () => {
    await render(make('me@example.com', []));
    expect(button('calendar:detail.mailOrganizer')).toBeNull();
    expect(button('calendar:detail.mailParticipants')).toBeNull();
  });

  it('mail_participants_reachable_when_only_the_organizer_is_listed', async () => {
    await render(make('lead@example.com', [{ email: 'lead@example.com', response: 'organizer' }]));
    await act(async () => button('calendar:detail.mailParticipants')?.click());
    expect(openComposeTab).toHaveBeenCalledWith('acc', ['lead@example.com'], 'Weekly sync');
  });

  it('icons_shown_on_read_only_calendars', async () => {
    await render(make('lead@example.com', guests), true);
    expect(button('calendar:detail.mailOrganizer')).not.toBeNull();
    expect(button('calendar:detail.mailParticipants')).not.toBeNull();
  });
});
