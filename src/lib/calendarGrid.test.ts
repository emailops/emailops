import { describe, expect, it } from 'vitest';
import type { CalendarEvent } from '@/types';
import {
  addDays,
  EVENT_MIN_BLOCK_PX,
  EVENT_RIGHT_GUTTER_PCT,
  EVENT_TEXT_ONE_LINE_MIN_PX,
  EVENT_TEXT_TWO_LINES_MIN_PX,
  eventColumnGeometry,
  eventsForDay,
  eventTextMode,
  layoutDayEvents,
  monthGrid,
  resolveCalendarAccountId,
  slotFromOffsetY,
  startOfDay,
  startOfWeekMonday,
  startsIn,
  weekDays,
} from './calendarGrid';

/** Local-time epoch seconds — keeps the tests deterministic in any host TZ. */
function sec(y: number, mo: number, d: number, h = 0, mi = 0): number {
  return Math.floor(new Date(y, mo - 1, d, h, mi).getTime() / 1000);
}

function day(y: number, mo: number, d: number): Date {
  return new Date(y, mo - 1, d);
}

function makeEvent(overrides: Partial<CalendarEvent> = {}): CalendarEvent {
  return {
    id: 'ev-1',
    accountId: 'acc-1',
    providerEventId: 'p-1',
    calendarId: 'primary',
    title: 'Standup',
    description: '',
    location: '',
    startTime: sec(2026, 7, 22, 10, 0),
    endTime: sec(2026, 7, 22, 11, 0),
    isAllDay: false,
    timezone: 'Europe/Madrid',
    organizer: 'organizer@example.com',
    attendees: [],
    meetingLink: null,
    meetingPlatform: null,
    status: 'confirmed',
    htmlLink: null,
    notifiedAt: null,
    recurringEventId: null,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

describe('startOfDay / addDays', () => {
  it('startOfDay strips the time-of-day', () => {
    const d = startOfDay(new Date(2026, 6, 22, 15, 42, 30));
    expect([d.getFullYear(), d.getMonth(), d.getDate()]).toEqual([2026, 6, 22]);
    expect([d.getHours(), d.getMinutes(), d.getSeconds()]).toEqual([0, 0, 0]);
  });

  it('addDays does calendar-day arithmetic across month boundaries', () => {
    const d = addDays(day(2026, 7, 31), 1);
    expect([d.getFullYear(), d.getMonth(), d.getDate()]).toEqual([2026, 7, 1]);
    const back = addDays(day(2026, 7, 1), -1);
    expect([back.getMonth(), back.getDate()]).toEqual([5, 30]);
  });
});

describe('startOfWeekMonday', () => {
  const cases: Array<{ name: string; input: Date; expected: [number, number, number] }> = [
    { name: 'Wednesday maps to the preceding Monday', input: day(2026, 7, 22), expected: [2026, 6, 20] },
    { name: 'Sunday belongs to the preceding Monday-start week', input: day(2026, 7, 26), expected: [2026, 6, 20] },
    { name: 'Monday maps to itself', input: day(2026, 7, 20), expected: [2026, 6, 20] },
    { name: 'crosses a month boundary backwards', input: day(2026, 7, 1), expected: [2026, 5, 29] },
  ];
  it.each(cases)('$name', ({ input, expected }) => {
    const monday = startOfWeekMonday(input);
    expect([monday.getFullYear(), monday.getMonth(), monday.getDate()]).toEqual(expected);
    expect(monday.getHours()).toBe(0);
  });
});

describe('weekDays', () => {
  it('returns 7 consecutive local days starting on Monday', () => {
    const days = weekDays(day(2026, 7, 22));
    expect(days).toHaveLength(7);
    expect([days[0].getMonth(), days[0].getDate()]).toEqual([6, 20]);
    expect([days[6].getMonth(), days[6].getDate()]).toEqual([6, 26]);
    for (let i = 1; i < 7; i++) {
      expect(days[i].getTime() - days[i - 1].getTime()).toBeGreaterThan(0);
      expect(days[i].getDate()).toBe(addDays(days[i - 1], 1).getDate());
    }
  });
});

describe('monthGrid', () => {
  it.each([
    { anchor: day(2026, 7, 22), name: 'July 2026' },
    { anchor: day(2026, 6, 1), name: 'June 2026 (starts on Monday)' },
    { anchor: day(2027, 2, 15), name: 'February 2027 (28 days, starts on Monday)' },
    { anchor: day(2024, 2, 10), name: 'February 2024 (leap year)' },
  ])('always returns 42 cells for $name', ({ anchor }) => {
    expect(monthGrid(anchor)).toHaveLength(42);
  });

  it('starts at the Monday on or before the 1st of the month', () => {
    const cells = monthGrid(day(2026, 7, 22));
    // July 1 2026 is a Wednesday → grid starts Monday June 29.
    expect([cells[0].date.getMonth(), cells[0].date.getDate()]).toEqual([5, 29]);
    expect(cells[0].inMonth).toBe(false);
    expect([cells[2].date.getMonth(), cells[2].date.getDate()]).toEqual([6, 1]);
    expect(cells[2].inMonth).toBe(true);
  });

  it('ends 42 consecutive days later, flagging adjacent-month days', () => {
    const cells = monthGrid(day(2026, 7, 22));
    const last = cells[41];
    // June 29 + 41 days = August 9.
    expect([last.date.getMonth(), last.date.getDate()]).toEqual([7, 9]);
    expect(last.inMonth).toBe(false);
  });

  it('marks exactly the days of the anchor month as inMonth', () => {
    const table = [
      { anchor: day(2026, 7, 1), daysInMonth: 31 },
      { anchor: day(2027, 2, 1), daysInMonth: 28 },
      { anchor: day(2024, 2, 1), daysInMonth: 29 },
    ];
    for (const { anchor, daysInMonth } of table) {
      const cells = monthGrid(anchor);
      expect(cells.filter((c) => c.inMonth)).toHaveLength(daysInMonth);
    }
  });

  it('uses Monday as the first column even when the month starts on Monday', () => {
    const cells = monthGrid(day(2026, 6, 15));
    // June 1 2026 is a Monday → no leading adjacent days.
    expect([cells[0].date.getMonth(), cells[0].date.getDate()]).toEqual([5, 1]);
    expect(cells[0].inMonth).toBe(true);
  });
});

describe('eventsForDay', () => {
  const target = day(2026, 7, 22);

  it('includes events inside the local day and excludes other days', () => {
    const inside = makeEvent({ id: 'in' });
    const other = makeEvent({
      id: 'out',
      startTime: sec(2026, 7, 23, 10),
      endTime: sec(2026, 7, 23, 11),
    });
    const result = eventsForDay([other, inside], target);
    expect(result.map((e) => e.id)).toEqual(['in']);
  });

  it('includes multi-day events that span the day', () => {
    const spanning = makeEvent({
      id: 'span',
      startTime: sec(2026, 7, 20, 9),
      endTime: sec(2026, 7, 25, 18),
    });
    expect(eventsForDay([spanning], target).map((e) => e.id)).toEqual(['span']);
  });

  it('treats endTime as exclusive: an event ending exactly at midnight is not part of the next day', () => {
    const endsAtMidnight = makeEvent({
      id: 'prev-day',
      startTime: sec(2026, 7, 21, 22),
      endTime: sec(2026, 7, 22, 0),
    });
    expect(eventsForDay([endsAtMidnight], target)).toHaveLength(0);
  });

  it('includes an event starting exactly at local midnight', () => {
    const atMidnight = makeEvent({
      id: 'midnight',
      startTime: sec(2026, 7, 22, 0),
      endTime: sec(2026, 7, 22, 1),
    });
    expect(eventsForDay([atMidnight], target).map((e) => e.id)).toEqual(['midnight']);
  });

  it('includes zero-duration events falling on the day', () => {
    const zero = makeEvent({
      id: 'zero',
      startTime: sec(2026, 7, 22, 10),
      endTime: sec(2026, 7, 22, 10),
    });
    expect(eventsForDay([zero], target).map((e) => e.id)).toEqual(['zero']);
  });

  it('sorts all-day events first, then timed events by start time', () => {
    const late = makeEvent({ id: 'late', startTime: sec(2026, 7, 22, 15), endTime: sec(2026, 7, 22, 16) });
    const early = makeEvent({ id: 'early', startTime: sec(2026, 7, 22, 8), endTime: sec(2026, 7, 22, 9) });
    const allDay = makeEvent({
      id: 'allday',
      isAllDay: true,
      startTime: sec(2026, 7, 22, 0),
      endTime: sec(2026, 7, 23, 0),
    });
    const result = eventsForDay([late, allDay, early], target);
    expect(result.map((e) => e.id)).toEqual(['allday', 'early', 'late']);
  });
});

describe('layoutDayEvents', () => {
  it('gives non-overlapping events the full width (single column)', () => {
    const a = makeEvent({ id: 'a', startTime: sec(2026, 7, 22, 9), endTime: sec(2026, 7, 22, 10) });
    const b = makeEvent({ id: 'b', startTime: sec(2026, 7, 22, 14), endTime: sec(2026, 7, 22, 15) });
    const layout = layoutDayEvents([b, a]);
    expect(layout.map((p) => [p.event.id, p.column, p.columns])).toEqual([
      ['a', 0, 1],
      ['b', 0, 1],
    ]);
  });

  it('splits two overlapping events into two columns', () => {
    const a = makeEvent({ id: 'a', startTime: sec(2026, 7, 22, 9), endTime: sec(2026, 7, 22, 11) });
    const b = makeEvent({ id: 'b', startTime: sec(2026, 7, 22, 10), endTime: sec(2026, 7, 22, 12) });
    const layout = layoutDayEvents([a, b]);
    expect(layout.map((p) => [p.event.id, p.column, p.columns])).toEqual([
      ['a', 0, 2],
      ['b', 1, 2],
    ]);
  });

  it('reuses a freed column inside an overlap cluster', () => {
    // a: 10-11, b: 10:30-12, c: 11-12:30 — c fits back into a's column, but
    // all three share one cluster of 2 columns.
    const a = makeEvent({ id: 'a', startTime: sec(2026, 7, 22, 10), endTime: sec(2026, 7, 22, 11) });
    const b = makeEvent({ id: 'b', startTime: sec(2026, 7, 22, 10, 30), endTime: sec(2026, 7, 22, 12) });
    const c = makeEvent({ id: 'c', startTime: sec(2026, 7, 22, 11), endTime: sec(2026, 7, 22, 12, 30) });
    const layout = layoutDayEvents([a, b, c]);
    expect(layout.map((p) => [p.event.id, p.column, p.columns])).toEqual([
      ['a', 0, 2],
      ['b', 1, 2],
      ['c', 0, 2],
    ]);
  });

  it('treats touching events (end == next start) as non-overlapping', () => {
    const a = makeEvent({ id: 'a', startTime: sec(2026, 7, 22, 10), endTime: sec(2026, 7, 22, 11) });
    const b = makeEvent({ id: 'b', startTime: sec(2026, 7, 22, 11), endTime: sec(2026, 7, 22, 12) });
    const layout = layoutDayEvents([a, b]);
    expect(layout.map((p) => [p.event.id, p.column, p.columns])).toEqual([
      ['a', 0, 1],
      ['b', 0, 1],
    ]);
  });

  it('stacks three mutually-overlapping events into three columns', () => {
    const a = makeEvent({ id: 'a', startTime: sec(2026, 7, 22, 9), endTime: sec(2026, 7, 22, 12) });
    const b = makeEvent({ id: 'b', startTime: sec(2026, 7, 22, 9, 30), endTime: sec(2026, 7, 22, 11) });
    const c = makeEvent({ id: 'c', startTime: sec(2026, 7, 22, 10), endTime: sec(2026, 7, 22, 10, 30) });
    const layout = layoutDayEvents([a, b, c]);
    expect(layout.map((p) => [p.event.id, p.column, p.columns])).toEqual([
      ['a', 0, 3],
      ['b', 1, 3],
      ['c', 2, 3],
    ]);
  });
});

describe('eventTextMode', () => {
  it('threshold constants are ordered sensibly', () => {
    expect(EVENT_TEXT_TWO_LINES_MIN_PX).toBeGreaterThan(EVENT_TEXT_ONE_LINE_MIN_PX);
    expect(EVENT_TEXT_ONE_LINE_MIN_PX).toBeGreaterThan(0);
    // The clamp keeps every block clickable, even in no-text mode.
    expect(EVENT_MIN_BLOCK_PX).toBeGreaterThan(0);
  });

  const cases: Array<{ name: string; height: number; expected: ReturnType<typeof eventTextMode> }> = [
    { name: 'a one-hour block (48px) shows two lines', height: 48, expected: 'two-lines' },
    {
      name: 'exactly the two-line threshold shows two lines',
      height: EVENT_TEXT_TWO_LINES_MIN_PX,
      expected: 'two-lines',
    },
    {
      name: 'just under the two-line threshold collapses to one line',
      height: EVENT_TEXT_TWO_LINES_MIN_PX - 1,
      expected: 'one-line',
    },
    { name: 'a 30-minute block (24px) shows one line', height: 24, expected: 'one-line' },
    { name: 'exactly the one-line threshold shows one line', height: EVENT_TEXT_ONE_LINE_MIN_PX, expected: 'one-line' },
    {
      name: 'just under the one-line threshold shows no text',
      height: EVENT_TEXT_ONE_LINE_MIN_PX - 1,
      expected: 'no-text',
    },
    { name: 'a tiny 5-minute block (4px) shows no text', height: 4, expected: 'no-text' },
    { name: 'zero height shows no text', height: 0, expected: 'no-text' },
  ];
  it.each(cases)('$name', ({ height, expected }) => {
    expect(eventTextMode(height)).toBe(expected);
  });
});

describe('eventColumnGeometry', () => {
  it('a single column takes the full width minus the right gutter', () => {
    expect(eventColumnGeometry(0, 1)).toEqual({ leftPct: 0, widthPct: 100 - EVENT_RIGHT_GUTTER_PCT });
  });

  it('two concurrent events split the non-gutter width equally, side by side', () => {
    const first = eventColumnGeometry(0, 2);
    const second = eventColumnGeometry(1, 2);
    expect(first.widthPct).toBe(second.widthPct);
    expect(first.leftPct).toBe(0);
    // Second starts exactly where the first ends — no gap, no overlap.
    expect(second.leftPct).toBe(first.widthPct);
    // Together they span exactly the non-gutter width.
    expect(second.leftPct + second.widthPct).toBeCloseTo(100 - EVENT_RIGHT_GUTTER_PCT);
  });

  it('three columns split into equal thirds of the non-gutter width', () => {
    const geo = [0, 1, 2].map((c) => eventColumnGeometry(c, 3));
    const expectedWidth = (100 - EVENT_RIGHT_GUTTER_PCT) / 3;
    for (const g of geo) expect(g.widthPct).toBeCloseTo(expectedWidth);
    expect(geo.map((g) => g.leftPct)).toEqual([0, expectedWidth, expectedWidth * 2]);
  });

  it('two events starting at the same time lay out as equal-width side-by-side columns', () => {
    const a = makeEvent({ id: 'a', startTime: sec(2026, 7, 22, 9), endTime: sec(2026, 7, 22, 10) });
    const b = makeEvent({ id: 'b', startTime: sec(2026, 7, 22, 9), endTime: sec(2026, 7, 22, 10) });
    const layout = layoutDayEvents([a, b]);
    const geo = layout.map((p) => eventColumnGeometry(p.column, p.columns));
    expect(geo[0].widthPct).toBe(geo[1].widthPct);
    const lefts = geo.map((g) => g.leftPct).sort((x, y) => x - y);
    expect(lefts[1]).toBe(lefts[0] + geo[0].widthPct);
  });
});

describe('slotFromOffsetY', () => {
  const dayStart = day(2026, 7, 22);
  const PX_PER_HOUR = 48;

  const cases: Array<{ name: string; offsetY: number; start: number; end: number }> = [
    { name: 'top of the grid is 00:00–01:00', offsetY: 0, start: sec(2026, 7, 22, 0), end: sec(2026, 7, 22, 1) },
    {
      name: 'inside the first half-hour snaps down to 00:00',
      offsetY: 23,
      start: sec(2026, 7, 22, 0),
      end: sec(2026, 7, 22, 1),
    },
    {
      name: 'inside the second half-hour snaps down to 00:30',
      offsetY: 47,
      start: sec(2026, 7, 22, 0, 30),
      end: sec(2026, 7, 22, 1, 30),
    },
    { name: '7:00 exactly', offsetY: 7 * 48, start: sec(2026, 7, 22, 7), end: sec(2026, 7, 22, 8) },
    {
      name: '7:20-ish snaps down to 7:00',
      offsetY: 7 * 48 + 20,
      start: sec(2026, 7, 22, 7),
      end: sec(2026, 7, 22, 8),
    },
    { name: '7:30 exactly', offsetY: 7.5 * 48, start: sec(2026, 7, 22, 7, 30), end: sec(2026, 7, 22, 8, 30) },
    { name: 'negative offsets clamp to 00:00', offsetY: -10, start: sec(2026, 7, 22, 0), end: sec(2026, 7, 22, 1) },
    {
      name: 'bottom of the grid clamps to the last 23:30 slot (end crosses midnight)',
      offsetY: 24 * 48,
      start: sec(2026, 7, 22, 23, 30),
      end: sec(2026, 7, 23, 0, 30),
    },
    {
      name: 'just above the bottom snaps to 23:30',
      offsetY: 24 * 48 - 1,
      start: sec(2026, 7, 22, 23, 30),
      end: sec(2026, 7, 23, 0, 30),
    },
  ];
  it.each(cases)('$name', ({ offsetY, start, end }) => {
    expect(slotFromOffsetY(offsetY, dayStart, PX_PER_HOUR)).toEqual({ start, end });
  });

  it('respects a different pxPerHour scale', () => {
    expect(slotFromOffsetY(90, dayStart, 60)).toEqual({
      start: sec(2026, 7, 22, 1, 30),
      end: sec(2026, 7, 22, 2, 30),
    });
  });
});

describe('startsIn', () => {
  const now = sec(2026, 7, 22, 10, 0);
  const cases: Array<{ name: string; start: number; expected: ReturnType<typeof startsIn> }> = [
    { name: '10 minutes ahead', start: now + 600, expected: { kind: 'minutes', minutes: 10 } },
    { name: 'rounds to the nearest minute', start: now + 601, expected: { kind: 'minutes', minutes: 10 } },
    { name: '90 seconds rounds to 2 minutes', start: now + 90, expected: { kind: 'minutes', minutes: 2 } },
    { name: 'one hour ahead', start: now + 3600, expected: { kind: 'minutes', minutes: 60 } },
    { name: 'under a minute is "now"', start: now + 59, expected: { kind: 'now' } },
    { name: 'exactly now has started', start: now, expected: { kind: 'started' } },
    { name: 'in the past has started', start: now - 30, expected: { kind: 'started' } },
  ];
  it.each(cases)('$name', ({ start, expected }) => {
    expect(startsIn(start, now)).toEqual(expected);
  });
});

describe('resolveCalendarAccountId', () => {
  const accounts = [
    { id: 'a', enabled: true },
    { id: 'b', enabled: false },
    { id: 'c', enabled: true },
  ];

  const cases: Array<{
    name: string;
    persisted: string | null;
    preferred: string | null;
    expected: string | null;
  }> = [
    { name: 'valid persisted account wins', persisted: 'c', preferred: 'a', expected: 'c' },
    { name: 'unknown persisted falls back to preferred', persisted: 'ghost', preferred: 'a', expected: 'a' },
    { name: 'disabled persisted falls back to preferred', persisted: 'b', preferred: 'c', expected: 'c' },
    { name: 'no persisted uses preferred', persisted: null, preferred: 'a', expected: 'a' },
    { name: 'disabled preferred falls back to first enabled', persisted: null, preferred: 'b', expected: 'a' },
    { name: 'nothing valid picks the first enabled', persisted: null, preferred: null, expected: 'a' },
  ];
  it.each(cases)('$name', ({ persisted, preferred, expected }) => {
    expect(resolveCalendarAccountId(accounts, persisted, preferred)).toBe(expected);
  });

  it('returns null when no account is enabled', () => {
    expect(resolveCalendarAccountId([{ id: 'x', enabled: false }], 'x', 'x')).toBeNull();
  });
});
