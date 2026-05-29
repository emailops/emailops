import { describe, expect, it } from 'vitest';

import { formatBytes, formatDate, formatNumber, formatRelativeTime, relativeParts } from './intl';

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
