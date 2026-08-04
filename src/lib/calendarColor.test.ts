import { describe, expect, it } from 'vitest';
import {
  type CalendarColorSource,
  calendarColor,
  calendarColorMap,
  eventBlockStyle,
  eventChipStyle,
  FALLBACK_CALENDAR_COLORS,
  hiddenCalendarIds,
  isHexColor,
  readableTextColor,
  visibleEvents,
  withAlpha,
} from './calendarColor';

function cal(providerCalendarId: string, color: string, isVisible = true): CalendarColorSource {
  return { providerCalendarId, color, isVisible };
}

describe('isHexColor', () => {
  it('accepts 6-digit and 3-digit hex', () => {
    expect(isHexColor('#33b679')).toBe(true);
    expect(isHexColor('#abc')).toBe(true);
  });

  it('rejects the empty colour providers send when they have none', () => {
    expect(isHexColor('')).toBe(false);
  });

  it('rejects named colours and malformed values', () => {
    expect(isHexColor('lightGreen')).toBe(false);
    expect(isHexColor('#12345')).toBe(false);
    expect(isHexColor('33b679')).toBe(false);
    expect(isHexColor(null)).toBe(false);
    expect(isHexColor(undefined)).toBe(false);
  });
});

describe('calendarColor', () => {
  it("uses the provider's own colour when there is one", () => {
    expect(calendarColor('#33b679', 'team@group.calendar.google.com')).toBe('#33b679');
  });

  it('expands shorthand hex so downstream maths has one shape to handle', () => {
    expect(calendarColor('#abc', 'cal')).toBe('#aabbcc');
  });

  it('falls back to a palette slot when the provider gave no colour', () => {
    const color = calendarColor('', 'team@group.calendar.google.com');
    expect(FALLBACK_CALENDAR_COLORS).toContain(color);
  });

  it('gives the same calendar the same fallback colour every time', () => {
    // Colours must not shuffle between launches — the user learns them.
    const first = calendarColor('', 'holidays@group.v.calendar.google.com');
    const second = calendarColor(null, 'holidays@group.v.calendar.google.com');
    expect(second).toBe(first);
  });

  it('gives different calendars different fallback colours', () => {
    const ids = ['a@group.calendar.google.com', 'b@group.calendar.google.com', 'c@group.calendar.google.com'];
    const colors = new Set(ids.map((id) => calendarColor('', id)));
    expect(colors.size).toBe(ids.length);
  });
});

describe('readableTextColor', () => {
  it('puts dark text on light fills', () => {
    expect(readableTextColor('#f6bf26')).toBe('#111827');
  });

  it('puts white text on dark fills', () => {
    expect(readableTextColor('#0b8043')).toBe('#ffffff');
  });

  it('keeps every fallback palette colour legible', () => {
    for (const color of FALLBACK_CALENDAR_COLORS) {
      expect(['#111827', '#ffffff']).toContain(readableTextColor(color));
    }
  });
});

describe('withAlpha', () => {
  it('appends the alpha byte', () => {
    expect(withAlpha('#039be5', 1)).toBe('#039be5ff');
    expect(withAlpha('#039be5', 0)).toBe('#039be500');
  });

  it('clamps out-of-range alpha instead of emitting invalid hex', () => {
    expect(withAlpha('#039be5', 5)).toBe('#039be5ff');
    expect(withAlpha('#039be5', -1)).toBe('#039be500');
  });

  it('always produces a two-digit alpha byte', () => {
    expect(withAlpha('#039be5', 0.02)).toMatch(/^#[0-9a-f]{8}$/);
  });
});

describe('eventBlockStyle', () => {
  it('tints the block in the calendar colour with a solid left rule', () => {
    const style = eventBlockStyle('#33b679', false);
    expect(style.borderColor).toBe('#33b679');
    expect(style.backgroundColor.startsWith('#33b679')).toBe(true);
    expect(style.borderStyle).toBe('solid');
  });

  it('marks tentative events with a dashed border and a fainter tint', () => {
    const tentative = eventBlockStyle('#33b679', true);
    const confirmed = eventBlockStyle('#33b679', false);
    expect(tentative.borderStyle).toBe('dashed');
    expect(tentative.backgroundColor).not.toBe(confirmed.backgroundColor);
  });
});

describe('eventChipStyle', () => {
  it('fills with the calendar colour and picks contrasting text', () => {
    expect(eventChipStyle('#0b8043')).toEqual({ backgroundColor: '#0b8043', color: '#ffffff' });
    expect(eventChipStyle('#f6bf26')).toEqual({ backgroundColor: '#f6bf26', color: '#111827' });
  });
});

describe('calendarColorMap', () => {
  it('maps each calendar id to its resolved colour', () => {
    const map = calendarColorMap([cal('primary', '#039be5'), cal('team@g.com', '#33b679')]);
    expect(map.get('primary')).toBe('#039be5');
    expect(map.get('team@g.com')).toBe('#33b679');
  });

  it('resolves a fallback for calendars the provider gave no colour', () => {
    const map = calendarColorMap([cal('team@g.com', '')]);
    expect(FALLBACK_CALENDAR_COLORS).toContain(map.get('team@g.com'));
  });

  it('includes hidden calendars, so re-showing one needs no refetch', () => {
    const map = calendarColorMap([cal('team@g.com', '#33b679', false)]);
    expect(map.get('team@g.com')).toBe('#33b679');
  });
});

describe('hiddenCalendarIds', () => {
  it('collects only the calendars switched off', () => {
    const hidden = hiddenCalendarIds([
      cal('primary', '#039be5'),
      cal('holidays@g.com', '#0b8043', false),
      cal('team@g.com', '#33b679', false),
    ]);
    expect(hidden).toEqual(new Set(['holidays@g.com', 'team@g.com']));
  });

  it('is empty when everything is visible', () => {
    expect(hiddenCalendarIds([cal('primary', '#039be5')]).size).toBe(0);
  });
});

describe('visibleEvents', () => {
  const events = [
    { id: 'a', calendarId: 'primary' },
    { id: 'b', calendarId: 'team@g.com' },
    { id: 'c', calendarId: 'primary' },
  ];

  it('drops events belonging to hidden calendars', () => {
    const shown = visibleEvents(events, new Set(['team@g.com']));
    expect(shown.map((e) => e.id)).toEqual(['a', 'c']);
  });

  it('keeps everything when nothing is hidden', () => {
    expect(visibleEvents(events, new Set())).toHaveLength(3);
  });

  it('can hide every event', () => {
    expect(visibleEvents(events, new Set(['primary', 'team@g.com']))).toEqual([]);
  });

  it('keeps the same meeting shown via a visible calendar when its copy in a hidden one is dropped', () => {
    // Providers repeat one event id across every calendar it appears in, so
    // hiding one calendar must not hide the copy in another.
    const duplicated = [
      { id: 'acc:primary:ev', calendarId: 'primary' },
      { id: 'acc:team@g.com:ev', calendarId: 'team@g.com' },
    ];
    const shown = visibleEvents(duplicated, new Set(['team@g.com']));
    expect(shown).toHaveLength(1);
    expect(shown[0].calendarId).toBe('primary');
  });
});
