// Providers give an all-day event dates, not instants, and they are stored as
// UTC midnights. The grid works in local days, so these tests run in zones on
// both sides of UTC: a suite running in UTC would never see the bug.

import { afterEach, describe, expect, it } from 'vitest';
import type { CalendarEvent } from '@/types';
import { eventBounds, eventsForDay, layoutEventSpans } from './calendarGrid';

const originalTz = process.env.TZ;

function restoreZone() {
  if (originalTz === undefined) Reflect.deleteProperty(process.env, 'TZ');
  else process.env.TZ = originalTz;
}

function allDay(fromDay: number, toDayExclusive: number): CalendarEvent {
  return {
    id: 'ev-1',
    title: 'Holiday',
    isAllDay: true,
    startTime: Date.UTC(2026, 6, fromDay) / 1000,
    endTime: Date.UTC(2026, 6, toDayExclusive) / 1000,
  } as CalendarEvent;
}

describe.each(['Europe/Madrid', 'America/New_York', 'Asia/Tokyo'])('all-day events in %s', (zone) => {
  afterEach(restoreZone);

  it('shows a one-day event on its own date only', () => {
    process.env.TZ = zone;
    const shownOn = [27, 28, 29].filter((day) => eventsForDay([allDay(28, 29)], new Date(2026, 6, day)).length > 0);

    expect(shownOn).toEqual([28]);
  });

  it('draws a three-day event across exactly its three dates', () => {
    process.env.TZ = zone;
    const days = [26, 27, 28, 29, 30, 31].map((day) => new Date(2026, 6, day));

    const [span] = layoutEventSpans([allDay(28, 31)], days);

    expect([span.startIndex, span.endIndex]).toEqual([2, 4]);
  });

  it('reports the bounds as the local midnights of its dates', () => {
    process.env.TZ = zone;

    expect(eventBounds(allDay(28, 29))).toEqual({
      start: new Date(2026, 6, 28).getTime() / 1000,
      end: new Date(2026, 6, 29).getTime() / 1000,
    });
  });
});

describe('eventBounds', () => {
  afterEach(restoreZone);

  it('keeps the date of a provider that hands the midnight of the zone', () => {
    process.env.TZ = 'Europe/Madrid';
    const event = {
      ...allDay(28, 29),
      startTime: Date.UTC(2026, 6, 27, 22) / 1000,
      endTime: Date.UTC(2026, 6, 28, 22) / 1000,
    };

    expect(eventBounds(event).start).toBe(new Date(2026, 6, 28).getTime() / 1000);
  });

  it('leaves a timed event at its instants', () => {
    const event = { startTime: 1000, endTime: 2000, isAllDay: false } as CalendarEvent;

    expect(eventBounds(event)).toEqual({ start: 1000, end: 2000 });
  });
});
