import { describe, expect, it } from 'vitest';
import {
  attendeeStatusMeta,
  cancellationMessage,
  describeRecurrence,
  eventsAfterDelete,
  isValidInviteeEmail,
  linkifySegments,
  recurrenceOptions,
  visibleAttendees,
} from './calendarEvent';

describe('visibleAttendees', () => {
  const list = [
    { email: 'Organizer@Example.com', response: 'organizer' },
    { email: 'ana@example.com', response: 'accepted' },
    { email: 'bo@example.com', response: 'needsAction' },
  ];

  it('drops the organizer from the list when the event has an organizer row', () => {
    const visible = visibleAttendees(list, 'organizer@example.com');
    expect(visible.map((a) => a.email)).toEqual(['ana@example.com', 'bo@example.com']);
  });

  it('matches the organizer email case-insensitively', () => {
    const visible = visibleAttendees(list, 'ORGANIZER@EXAMPLE.COM');
    expect(visible).toHaveLength(2);
  });

  it('keeps everyone when the event has no organizer field', () => {
    expect(visibleAttendees(list, '')).toHaveLength(3);
  });

  it('keeps an organizer-status attendee whose email differs from the organizer row', () => {
    // Shared/delegated calendars: the RSVP list may flag a different address
    // as organizer — that one is NOT a duplicate of the Organizer row.
    const delegated = [{ email: 'assistant@example.com', response: 'organizer' }];
    expect(visibleAttendees(delegated, 'organizer@example.com')).toHaveLength(1);
  });
});

describe('recurrenceOptions', () => {
  // 2026-07-22 is a Wednesday.
  const wednesday = new Date(2026, 6, 22);

  it('returns the six options in order', () => {
    const values = recurrenceOptions(wednesday, 'en').map((o) => o.value);
    expect(values).toEqual(['none', 'daily', 'weekly', 'weekdays', 'monthly', 'yearly']);
  });

  it('anchors the weekly label to the chosen date weekday (localized)', () => {
    const weeklyEn = recurrenceOptions(wednesday, 'en').find((o) => o.value === 'weekly');
    expect(weeklyEn?.params).toEqual({ weekday: 'Wednesday' });
    const weeklyEs = recurrenceOptions(wednesday, 'es').find((o) => o.value === 'weekly');
    expect(weeklyEs?.params).toEqual({ weekday: 'miércoles' });
  });

  it('only the weekly option carries interpolation params', () => {
    for (const o of recurrenceOptions(wednesday, 'en')) {
      if (o.value === 'weekly') expect(o.params).toBeDefined();
      else expect(o.params).toBeUndefined();
    }
  });

  it('uses each option value as its label key', () => {
    for (const o of recurrenceOptions(wednesday, 'en')) {
      expect(o.labelKey).toBe(o.value);
    }
  });
});

describe('attendeeStatusMeta', () => {
  const cases: Array<{ response: string; expected: string }> = [
    { response: 'accepted', expected: 'accepted' },
    { response: 'declined', expected: 'declined' },
    { response: 'tentative', expected: 'tentative' },
    { response: 'needsAction', expected: 'needsAction' },
    { response: 'organizer', expected: 'organizer' },
    { response: 'somethingNew', expected: 'needsAction' },
    { response: '', expected: 'needsAction' },
  ];
  it.each(cases)('$response → $expected', ({ response, expected }) => {
    expect(attendeeStatusMeta(response)).toBe(expected);
  });
});

describe('isValidInviteeEmail', () => {
  const cases: Array<{ input: string; valid: boolean }> = [
    { input: 'ana@example.com', valid: true },
    { input: '  ana@example.com  ', valid: true },
    { input: 'Ana Pérez <ana@example.com>', valid: true },
    { input: 'ANA@EXAMPLE.CO.UK', valid: true },
    { input: 'plainstring', valid: false },
    { input: 'missing-domain@', valid: false },
    { input: '@missing-local.com', valid: false },
    { input: 'no-tld@example', valid: false },
    { input: 'dot-first@.com', valid: false },
    { input: 'spa ce@example.com', valid: false },
    { input: '', valid: false },
  ];
  it.each(cases)('"$input" → $valid', ({ input, valid }) => {
    expect(isValidInviteeEmail(input)).toBe(valid);
  });
});

describe('cancellationMessage', () => {
  // Regression: the delete command rejects `null` ("invalid type: null,
  // expected a string") — this helper must ALWAYS return a string.
  const cases = [
    { notify: true, provider: 'outlook', raw: '  Moving to next week  ', expected: 'Moving to next week' },
    { notify: true, provider: 'outlook', raw: '', expected: '' },
    { notify: true, provider: 'gmail', raw: 'ignored — Google has no custom message', expected: '' },
    { notify: false, provider: 'outlook', raw: 'ignored — nobody is notified', expected: '' },
    { notify: false, provider: 'gmail', raw: '', expected: '' },
  ] as const;
  it.each(cases)('notify=$notify provider=$provider → "$expected"', ({ notify, provider, raw, expected }) => {
    const result = cancellationMessage(notify, provider, raw);
    expect(result).toBe(expected);
    expect(typeof result).toBe('string');
  });
});

describe('eventsAfterDelete', () => {
  interface Ev {
    id: string;
    recurringEventId: string | null;
    startTime: number;
  }
  const ev = (id: string, recurringEventId: string | null, startTime: number): Ev => ({
    id,
    recurringEventId,
    startTime,
  });

  // A weekly series `series-1` with three instances, an unrelated series
  // instance, and a standalone event.
  const events: Ev[] = [
    ev('a1', 'series-1', 100),
    ev('a2', 'series-1', 200),
    ev('a3', 'series-1', 300),
    ev('b1', 'series-2', 150),
    ev('solo', null, 250),
  ];

  const cases: Array<{
    name: string;
    deleted: Ev;
    scope: 'instance' | 'following' | 'all';
    remaining: string[];
  }> = [
    {
      name: 'instance scope removes only the deleted occurrence',
      deleted: ev('a2', 'series-1', 200),
      scope: 'instance',
      remaining: ['a1', 'a3', 'b1', 'solo'],
    },
    {
      name: 'all scope removes every occurrence of the series',
      deleted: ev('a2', 'series-1', 200),
      scope: 'all',
      remaining: ['b1', 'solo'],
    },
    {
      name: 'following scope removes the deleted occurrence and later ones in the same series',
      deleted: ev('a2', 'series-1', 200),
      scope: 'following',
      remaining: ['a1', 'b1', 'solo'],
    },
    {
      name: 'following scope keeps same-series occurrences that start earlier',
      deleted: ev('a3', 'series-1', 300),
      scope: 'following',
      remaining: ['a1', 'a2', 'b1', 'solo'],
    },
    {
      name: 'non-recurring event with all scope removes only that event',
      deleted: ev('solo', null, 250),
      scope: 'all',
      remaining: ['a1', 'a2', 'a3', 'b1'],
    },
    {
      name: 'non-recurring event with following scope removes only that event',
      deleted: ev('solo', null, 250),
      scope: 'following',
      remaining: ['a1', 'a2', 'a3', 'b1'],
    },
  ];

  it.each(cases)('$name', ({ deleted, scope, remaining }) => {
    expect(eventsAfterDelete(events, deleted, scope).map((e) => e.id)).toEqual(remaining);
  });

  it('all scope also removes the series master when it appears by id', () => {
    const withMaster = [ev('series-1', null, 50), ...events];
    const result = eventsAfterDelete(withMaster, ev('a2', 'series-1', 200), 'all');
    expect(result.map((e) => e.id)).toEqual(['b1', 'solo']);
  });

  it('does not mutate the input array', () => {
    const input = [...events];
    eventsAfterDelete(input, ev('a2', 'series-1', 200), 'all');
    expect(input).toEqual(events);
  });
});

describe('describeRecurrence', () => {
  // 2026-07-28 is a Tuesday.
  const tuesday = new Date(2026, 6, 28);

  it('FREQ=DAILY → daily', () => {
    expect(describeRecurrence('FREQ=DAILY', tuesday, 'en')).toEqual({ labelKey: 'daily', raw: 'FREQ=DAILY' });
  });

  it('FREQ=WEEKLY → weekly anchored to the start weekday (localized)', () => {
    expect(describeRecurrence('FREQ=WEEKLY', tuesday, 'en')).toEqual({
      labelKey: 'weekly',
      params: { weekday: 'Tuesday' },
      raw: 'FREQ=WEEKLY',
    });
    expect(describeRecurrence('FREQ=WEEKLY', tuesday, 'es')).toEqual({
      labelKey: 'weekly',
      params: { weekday: 'martes' },
      raw: 'FREQ=WEEKLY',
    });
  });

  it('FREQ=WEEKLY;BYDAY=<single day> still reads as weekly on the start weekday', () => {
    expect(describeRecurrence('FREQ=WEEKLY;BYDAY=TU', tuesday, 'en')).toEqual({
      labelKey: 'weekly',
      params: { weekday: 'Tuesday' },
      raw: 'FREQ=WEEKLY;BYDAY=TU',
    });
  });

  it('BYDAY=MO,TU,WE,TH,FR → every weekday, regardless of day order', () => {
    expect(describeRecurrence('FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR', tuesday, 'en').labelKey).toBe('weekdays');
    expect(describeRecurrence('FREQ=WEEKLY;BYDAY=FR,TH,WE,TU,MO', tuesday, 'en').labelKey).toBe('weekdays');
  });

  it('FREQ=MONTHLY → monthly (Google-style BYDAY qualifiers tolerated)', () => {
    expect(describeRecurrence('FREQ=MONTHLY', tuesday, 'en').labelKey).toBe('monthly');
    expect(describeRecurrence('FREQ=MONTHLY;BYDAY=3TU', tuesday, 'en').labelKey).toBe('monthly');
  });

  it('FREQ=YEARLY → yearly', () => {
    expect(describeRecurrence('FREQ=YEARLY', tuesday, 'en').labelKey).toBe('yearly');
  });

  it('ignores COUNT/UNTIL qualifiers when the base pattern is recognized', () => {
    expect(describeRecurrence('FREQ=DAILY;COUNT=10', tuesday, 'en').labelKey).toBe('daily');
    expect(describeRecurrence('FREQ=WEEKLY;UNTIL=20261231T000000Z', tuesday, 'en').labelKey).toBe('weekly');
  });

  it('is case-insensitive and tolerates a leading RRULE: prefix', () => {
    expect(describeRecurrence('freq=weekly', tuesday, 'en').labelKey).toBe('weekly');
    expect(describeRecurrence('RRULE:FREQ=DAILY', tuesday, 'en').labelKey).toBe('daily');
  });

  const rawCases = [
    { rrule: 'FREQ=WEEKLY;INTERVAL=2', why: 'interval > 1' },
    { rrule: 'FREQ=WEEKLY;BYDAY=MO,WE', why: 'multiple non-weekday BYDAY' },
    { rrule: 'FREQ=HOURLY', why: 'unsupported frequency' },
    { rrule: 'every other tuesday', why: 'not an RRULE at all' },
    { rrule: '', why: 'empty' },
  ];
  it.each(rawCases)('falls back to the raw value for $why', ({ rrule }) => {
    expect(describeRecurrence(rrule, tuesday, 'en')).toEqual({ labelKey: null, raw: rrule });
  });
});

describe('linkifySegments', () => {
  it('returns a single text segment when there are no URLs', () => {
    expect(linkifySegments('Bring the Q3 numbers')).toEqual([{ kind: 'text', value: 'Bring the Q3 numbers' }]);
  });

  it('splits a URL in the middle into text/link/text', () => {
    expect(linkifySegments('Doc: https://example.com/agenda before the call')).toEqual([
      { kind: 'text', value: 'Doc: ' },
      { kind: 'link', value: 'https://example.com/agenda' },
      { kind: 'text', value: ' before the call' },
    ]);
  });

  it('trims trailing prose punctuation off the link (kept as text)', () => {
    expect(linkifySegments('Join https://meet.jit.si/Weekly42.')).toEqual([
      { kind: 'text', value: 'Join ' },
      { kind: 'link', value: 'https://meet.jit.si/Weekly42' },
      { kind: 'text', value: '.' },
    ]);
  });

  it('handles multiple URLs and keeps query strings', () => {
    const segments = linkifySegments('A https://a.example.com/x?y=1 and http://b.example.org');
    expect(segments.filter((s) => s.kind === 'link').map((s) => s.value)).toEqual([
      'https://a.example.com/x?y=1',
      'http://b.example.org',
    ]);
  });

  it('handles a URL spanning the whole string', () => {
    expect(linkifySegments('https://example.com/only')).toEqual([{ kind: 'link', value: 'https://example.com/only' }]);
  });

  it('does not linkify non-http schemes', () => {
    expect(linkifySegments('mail me at mailto:a@example.com or ftp://x.example')).toEqual([
      { kind: 'text', value: 'mail me at mailto:a@example.com or ftp://x.example' },
    ]);
  });

  it('stops a URL at quotes and angle brackets', () => {
    expect(linkifySegments('see <https://example.com/path> now')).toEqual([
      { kind: 'text', value: 'see <' },
      { kind: 'link', value: 'https://example.com/path' },
      { kind: 'text', value: '> now' },
    ]);
  });
});
