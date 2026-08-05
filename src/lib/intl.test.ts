import { describe, expect, it } from 'vitest';

import { formatBytes, formatDate, formatInboxTimestamp, formatNumber, formatRelativeTime, relativeParts } from './intl';

describe('relativeParts (unit selection)', () => {
  it('selects seconds under a minute', () => {
    expect(relativeParts(5)).toEqual({ value: -5, unit: 'second' });
  });
  it('selects minutes under an hour', () => {
    expect(relativeParts(180)).toEqual({ value: -3, unit: 'minute' });
  });
  it('selects hours under a day', () => {
    expect(relativeParts(7200)).toEqual({ value: -2, unit: 'hour' });
  });
  it('selects days under a month', () => {
    expect(relativeParts(3 * 86_400)).toEqual({ value: -3, unit: 'day' });
  });
  it('selects months under a year', () => {
    expect(relativeParts(2 * 2_592_000)).toEqual({ value: -2, unit: 'month' });
  });
  it('selects years past a year', () => {
    expect(relativeParts(3 * 31_536_000)).toEqual({ value: -3, unit: 'year' });
  });
  it('uses a positive value for future timestamps', () => {
    expect(relativeParts(-7200)).toEqual({ value: 2, unit: 'hour' });
  });
});

describe('formatRelativeTime', () => {
  const nowMs = 1_700_000_000_000;
  const nowSec = Math.floor(nowMs / 1000);

  it('returns the em dash placeholder for null/undefined/0', () => {
    expect(formatRelativeTime(null, 'en', nowMs)).toBe('—');
    expect(formatRelativeTime(undefined, 'en', nowMs)).toBe('—');
    expect(formatRelativeTime(0, 'en', nowMs)).toBe('—');
  });

  it('localizes a 2-hour-ago timestamp differently per language', () => {
    const ts = nowSec - 7200;
    const en = formatRelativeTime(ts, 'en', nowMs);
    const es = formatRelativeTime(ts, 'es', nowMs);
    expect(en).toMatch(/2/);
    expect(es).toMatch(/2/);
    // English and Spanish phrasings differ.
    expect(en).not.toBe(es);
  });
});

describe('formatNumber', () => {
  it('groups thousands per locale', () => {
    expect(formatNumber(1234567, 'en')).toBe('1,234,567');
    expect(formatNumber(1234567, 'de')).toBe('1.234.567');
  });
});

describe('formatBytes', () => {
  it('returns 0 B for zero', () => {
    expect(formatBytes(0, 'en')).toBe('0 B');
  });
  it('scales to KB/MB/GB', () => {
    expect(formatBytes(1024, 'en')).toBe('1 KB');
    expect(formatBytes(1024 * 1024, 'en')).toBe('1 MB');
    expect(formatBytes(1536, 'en')).toBe('1.5 KB');
  });
});

describe('formatDate', () => {
  it('formats a unix-seconds timestamp for the given locale', () => {
    // 2023-11-14 in UTC; assert the year appears and locales differ in order.
    const ts = 1_700_000_000;
    const en = formatDate(ts, 'en', { year: 'numeric', month: 'short', day: 'numeric' });
    expect(en).toMatch(/2023/);
  });
});

describe('formatInboxTimestamp', () => {
  // 2026-08-05 15:04 local time — the reference "now" for every case below.
  const now = new Date(2026, 7, 5, 15, 4).getTime();
  const at = (y: number, m: number, d: number, h = 9, min = 7) =>
    Math.floor(new Date(y, m, d, h, min).getTime() / 1000);

  it('shows the time of day for today, so the newest mail reads at a glance', () => {
    expect(formatInboxTimestamp(at(2026, 7, 5, 10, 4), 'es', now)).toBe('10:04');
    expect(formatInboxTimestamp(at(2026, 7, 5, 0, 0), 'es', now)).toBe('00:00');
    // Later today (clock skew, a message from a device running fast).
    expect(formatInboxTimestamp(at(2026, 7, 5, 23, 59), 'es', now)).toBe('23:59');
  });

  it('shows day + abbreviated month within the current year', () => {
    // "03/08/2026" carries no information the list header does not; the month
    // name is what the eye actually scans for.
    expect(formatInboxTimestamp(at(2026, 7, 9), 'es')).toMatch(/^9\s/);
    expect(formatInboxTimestamp(at(2026, 7, 9), 'es', now)).toContain('ago');
    expect(formatInboxTimestamp(at(2026, 0, 3), 'es', now)).toContain('ene');
  });

  it('localizes the month name', () => {
    expect(formatInboxTimestamp(at(2026, 7, 9), 'en', now)).toContain('Aug');
    expect(formatInboxTimestamp(at(2026, 7, 9), 'de', now)).toContain('Aug');
    expect(formatInboxTimestamp(at(2026, 7, 9), 'fr', now)).toContain('août');
  });

  it('falls back to a numeric date once the year differs', () => {
    // The year is the one thing an abbreviated "3 ene" cannot express.
    expect(formatInboxTimestamp(at(2025, 0, 3), 'es', now)).toBe('03/01/2025');
    expect(formatInboxTimestamp(at(2019, 11, 31), 'es', now)).toBe('31/12/2019');
    // A future year is equally ambiguous without it.
    expect(formatInboxTimestamp(at(2027, 0, 1), 'es', now)).toBe('01/01/2027');
  });

  it('treats yesterday as a different day even minutes apart', () => {
    // 23:59 the previous day is ~65 minutes before `now` — a duration-based
    // check would call that "today" and print a bare time for it.
    const almostNow = new Date(2026, 7, 5, 0, 30).getTime();
    expect(formatInboxTimestamp(at(2026, 7, 4, 23, 59), 'es', almostNow)).toContain('ago');
  });

  it('renders the em dash for a missing timestamp', () => {
    expect(formatInboxTimestamp(null, 'es', now)).toBe('—');
    expect(formatInboxTimestamp(undefined, 'es', now)).toBe('—');
  });
});
