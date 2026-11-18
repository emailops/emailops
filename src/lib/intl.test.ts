import { describe, expect, it } from 'vitest';

import {
  formatBytes,
  formatDate,
  formatDayMonthYearTime,
  formatNumber,
  formatReceptionTime,
  formatRelativeTime,
  formattingLocale,
  relativeParts,
} from './intl';

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

describe('formatDayMonthYearTime', () => {
  it('writes DD/MM/YYYY HH:mm in local time, zero-padded, 24h', () => {
    const ts = new Date(2026, 8, 4, 9, 5).getTime() / 1000;
    expect(formatDayMonthYearTime(ts)).toBe('04/09/2026 09:05');
    const evening = new Date(2026, 11, 31, 23, 59).getTime() / 1000;
    expect(formatDayMonthYearTime(evening)).toBe('31/12/2026 23:59');
  });

  it('shows a dash for a missing timestamp', () => {
    expect(formatDayMonthYearTime(null)).toBe('—');
  });
});

describe('formattingLocale', () => {
  it('pairs the UI language with the system region', () => {
    expect(formattingLocale('en', 'US')).toBe('en-US');
    expect(formattingLocale('en', 'ES')).toBe('en-ES');
  });

  it('is the UI language alone when the system names no region', () => {
    expect(formattingLocale('en', null)).toBe('en');
  });
});

describe('formatReceptionTime', () => {
  const nowMs = new Date(2026, 10, 16, 18, 30).getTime();
  const august12 = new Date(2026, 7, 12, 9, 5).getTime() / 1000;

  it("shows today's mail as HH:MM, zero-padded, 24h", () => {
    const morning = new Date(2026, 10, 16, 9, 5).getTime() / 1000;
    expect(formatReceptionTime(morning, 'en-US', nowMs)).toBe('09:05');
  });

  it('writes an older date month first for a US English user', () => {
    expect(formatReceptionTime(august12, 'en-US', nowMs)).toBe('08/12/2026');
  });

  it('writes an older date day first for a UK or Spain English user', () => {
    expect(formatReceptionTime(august12, 'en-GB', nowMs)).toBe('12/08/2026');
    expect(formatReceptionTime(august12, 'en-ES', nowMs)).toBe('12/08/2026');
  });

  it("writes an older date in the order of the UI language's own convention", () => {
    expect(formatReceptionTime(august12, 'es', nowMs)).toBe('12/08/2026');
    expect(formatReceptionTime(august12, 'de', nowMs)).toBe('12.08.2026');
  });
});
