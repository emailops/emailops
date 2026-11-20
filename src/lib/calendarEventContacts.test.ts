import { describe, expect, it } from 'vitest';
import type { CalendarEvent } from '@/types';
import { canRespond, organizerRecipient, participantRecipients, selfResponse } from './calendarEventContacts';

const make = (organizer: string, attendees: { email: string; response: string }[]): CalendarEvent =>
  ({ organizer, attendees }) as unknown as CalendarEvent;

describe('calendarEventContacts', () => {
  it('organizer_is_null_when_it_is_me', () => {
    expect(organizerRecipient(make('Me@Example.com', []), 'me@example.com')).toBeNull();
    expect(organizerRecipient(make('', []), 'me@example.com')).toBeNull();
    expect(organizerRecipient(make('lead@example.com', []), 'me@example.com')).toBe('lead@example.com');
  });

  it('participants_include_organizer_exclude_me_and_dedupe', () => {
    const event = make('lead@example.com', [
      { email: 'ME@example.com', response: 'accepted' },
      { email: 'Lead@example.com', response: 'organizer' },
      { email: 'ana@example.com', response: 'needsAction' },
      { email: '', response: 'needsAction' },
      { email: 'ANA@example.com', response: 'accepted' },
    ]);
    expect(participantRecipients(event, 'me@example.com')).toEqual(['lead@example.com', 'ana@example.com']);
  });

  it('self_response_maps_needs_action_to_null', () => {
    const event = make('lead@example.com', [
      { email: 'me@example.com', response: 'needsAction' },
      { email: 'ana@example.com', response: 'declined' },
    ]);
    expect(selfResponse(event, 'ME@example.com')).toBeNull();
    expect(selfResponse(event, 'ana@example.com')).toBe('declined');
    expect(selfResponse(event, 'nobody@example.com')).toBeNull();
    expect(selfResponse(make('', [{ email: 'me@example.com', response: 'tentative' }]), 'me@example.com')).toBe(
      'tentative',
    );
  });

  it('cannot_respond_as_organizer_or_on_read_only', () => {
    const event = make('lead@example.com', [{ email: 'me@example.com', response: 'needsAction' }]);
    expect(canRespond(event, 'me@example.com', false)).toBe(true);
    expect(canRespond(event, 'me@example.com', true)).toBe(false);
    expect(canRespond(make('Me@example.com', []), 'me@example.com', false)).toBe(false);
    expect(
      canRespond(make('me@example.com', [{ email: 'me@example.com', response: 'organizer' }]), 'me@example.com', false),
    ).toBe(false);
  });
});
