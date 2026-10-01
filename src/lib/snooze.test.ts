import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { parseCustomSnooze, snoozePresets, toDatetimeLocalValue } from './snooze';

// Wall-clock rules ("tomorrow at 8:00") depend on the zone: pin one with a
// daylight-saving change so the DST cases are real.
beforeAll(() => {
  vi.stubEnv('TZ', 'Europe/Madrid');
});
afterAll(() => {
  vi.unstubAllEnvs();
});

/** A local wall-clock time in the pinned zone. */
const local = (y: number, mo: number, d: number, h = 0, mi = 0) => new Date(y, mo - 1, d, h, mi);
const presetsAt = (now: Date) => Object.fromEntries(snoozePresets(now).map((p) => [p.id, p.at]));

describe('snoozePresets', () => {
  it('offers later today (+3h, up to the next full hour), tomorrow, this weekend and next week on a weekday', () => {
    // Wednesday 2026-10-07 10:20
    const p = presetsAt(local(2026, 10, 7, 10, 20));
    expect(p.laterToday).toEqual(local(2026, 10, 7, 14, 0));
    expect(p.tomorrow).toEqual(local(2026, 10, 8, 8, 0));
    expect(p.thisWeekend).toEqual(local(2026, 10, 10, 8, 0));
    expect(p.nextWeek).toEqual(local(2026, 10, 12, 8, 0));
  });

  it('keeps an exact hour as it is', () => {
    expect(presetsAt(local(2026, 10, 7, 9, 0)).laterToday).toEqual(local(2026, 10, 7, 12, 0));
  });

  it('drops later today from 18:00 on', () => {
    expect(presetsAt(local(2026, 10, 7, 17, 59)).laterToday).toEqual(local(2026, 10, 7, 21, 0));
    expect(presetsAt(local(2026, 10, 7, 18, 0)).laterToday).toBeUndefined();
    expect(presetsAt(local(2026, 10, 7, 23, 30)).laterToday).toBeUndefined();
  });

  it('lists presets in time order without duplicates', () => {
    // Friday: this weekend would be tomorrow 8:00 — listed once, as tomorrow.
    const friday = snoozePresets(local(2026, 10, 9, 9, 0));
    expect(friday.map((p) => p.id)).toEqual(['laterToday', 'tomorrow', 'nextWeek']);
    // Sunday: next week would be tomorrow 8:00.
    const sunday = snoozePresets(local(2026, 10, 11, 20, 0));
    expect(sunday.map((p) => p.id)).toEqual(['tomorrow']);
    // Saturday: no "this weekend" (it is the weekend); next week is Monday.
    const saturday = presetsAt(local(2026, 10, 10, 9, 0));
    expect(saturday.thisWeekend).toBeUndefined();
    expect(saturday.nextWeek).toEqual(local(2026, 10, 12, 8, 0));
    // Monday: next week is the Monday after.
    expect(presetsAt(local(2026, 10, 12, 9, 0)).nextWeek).toEqual(local(2026, 10, 19, 8, 0));
  });

  it('keeps 8:00 wall-clock time across a daylight-saving change', () => {
    // Saturday 2026-03-28 20:00 CET; clocks jump to CEST at 02:00 on Sunday.
    const now = local(2026, 3, 28, 20, 0);
    const tomorrow = presetsAt(now).tomorrow;
    expect(tomorrow.getHours()).toBe(8);
    expect(tomorrow.getDate()).toBe(29);
    // 12 h of wall clock, but only 11 h of real time.
    expect(tomorrow.getTime() - now.getTime()).toBe(11 * 3600 * 1000);
    // Autumn: Saturday 2026-10-24, clocks go back an hour on Sunday.
    const autumn = local(2026, 10, 24, 20, 0);
    const back = presetsAt(autumn).tomorrow;
    expect(back.getHours()).toBe(8);
    expect(back.getTime() - autumn.getTime()).toBe(13 * 3600 * 1000);
  });

  it('later today counts real hours across the spring change', () => {
    // 00:30 CET + 3 h of real time = 04:30 CEST (the 02:00 hour does not exist).
    expect(presetsAt(local(2026, 3, 29, 0, 30)).laterToday).toEqual(local(2026, 3, 29, 5, 0));
  });
});

describe('parseCustomSnooze', () => {
  const now = local(2026, 10, 7, 10, 0);

  it('reads a datetime-local value in local time when it is in the future', () => {
    expect(parseCustomSnooze('2026-10-07T10:30', now)).toEqual(local(2026, 10, 7, 10, 30));
  });

  it('rejects the past, now, and malformed input', () => {
    expect(parseCustomSnooze('2026-10-07T09:59', now)).toBeNull();
    expect(parseCustomSnooze('2026-10-07T10:00', now)).toBeNull();
    expect(parseCustomSnooze('', now)).toBeNull();
    expect(parseCustomSnooze('tomorrow', now)).toBeNull();
  });

  it('round-trips with toDatetimeLocalValue', () => {
    const at = local(2026, 1, 5, 7, 5);
    expect(toDatetimeLocalValue(at)).toBe('2026-01-05T07:05');
    expect(parseCustomSnooze(toDatetimeLocalValue(at), now)).toBeNull();
    expect(parseCustomSnooze(toDatetimeLocalValue(local(2027, 1, 5, 7, 5)), now)).toEqual(local(2027, 1, 5, 7, 5));
  });
});
