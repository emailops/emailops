// "Your response" row in the event detail.
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

const make = (over: Partial<CalendarEvent> = {}): CalendarEvent =>
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
    organizer: 'lead@example.com',
    attendees: [
      { email: 'lead@example.com', response: 'organizer' },
      { email: 'me@example.com', response: 'needsAction' },
    ],
    meetingLink: null,
    htmlLink: null,
    status: 'confirmed',
    recurringEventId: null,
    ...over,
  }) as unknown as CalendarEvent;

describe('EventDetailDialog — your response', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onResponded = vi.fn();
  const onAuthError = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    useAccountStore.setState({ accounts: [{ id: 'acc', email: 'me@example.com' } as Account] });
    vi.mocked(api.respondToCalendarEvent).mockReset();
    onResponded.mockReset();
    onAuthError.mockReset();
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
          onClose={() => {}}
          onDeleted={() => {}}
          onResponded={onResponded}
          onAuthError={onAuthError}
        />,
      ),
    );

  const choice = (name: string) => container.querySelector<HTMLButtonElement>(`button[data-response="${name}"]`);

  it('current_response_is_marked', async () => {
    const event = make({
      attendees: [
        { email: 'lead@example.com', response: 'organizer' },
        { email: 'me@example.com', response: 'declined' },
      ],
    });
    await render(event);
    expect(choice('declined')?.getAttribute('aria-pressed')).toBe('true');
    expect(choice('accepted')?.getAttribute('aria-pressed')).toBe('false');
  });

  it('single_event_responds_with_instance_scope', async () => {
    vi.mocked(api.respondToCalendarEvent).mockResolvedValue(undefined);
    const event = make();
    await render(event);
    expect(container.querySelector('input[type="radio"][name="event-response-scope"]')).toBeNull();
    await act(async () => choice('accepted')?.click());
    expect(api.respondToCalendarEvent).toHaveBeenCalledWith('acc', 'primary', 'e1', 'accepted', 'instance');
    expect(onResponded).toHaveBeenCalledWith(event, 'accepted', 'instance');
    expect(choice('accepted')?.getAttribute('aria-pressed')).toBe('true');
  });

  it('recurring_event_asks_for_scope_defaulting_to_instance', async () => {
    vi.mocked(api.respondToCalendarEvent).mockResolvedValue(undefined);
    await render(make({ recurringEventId: 'series-1' }));
    const radios = container.querySelectorAll<HTMLInputElement>('input[type="radio"][name="event-response-scope"]');
    expect(radios).toHaveLength(2);
    expect(radios[0].checked).toBe(true);
    await act(async () => radios[1].click());
    await act(async () => choice('tentative')?.click());
    expect(api.respondToCalendarEvent).toHaveBeenCalledWith('acc', 'primary', 'e1', 'tentative', 'series');
  });

  it('response_row_hidden_for_organizer', async () => {
    await render(make({ organizer: 'Me@example.com' }));
    expect(choice('accepted')).toBeNull();
  });

  it('response_row_hidden_on_read_only_calendars', async () => {
    await render(make(), true);
    expect(choice('accepted')).toBeNull();
  });

  it('error_is_shown_inline', async () => {
    vi.mocked(api.respondToCalendarEvent).mockRejectedValue(new Error('boom'));
    await render(make());
    await act(async () => choice('accepted')?.click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('calendar:detail.responseError');
    expect(onResponded).not.toHaveBeenCalled();
    expect(choice('accepted')?.disabled).toBe(false);
  });

  it('auth_error_is_delegated_to_the_parent', async () => {
    vi.mocked(api.respondToCalendarEvent).mockRejectedValue(new Error('token expired'));
    await render(make());
    await act(async () => choice('accepted')?.click());
    expect(onAuthError).toHaveBeenCalled();
  });
});
