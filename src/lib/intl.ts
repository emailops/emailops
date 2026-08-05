// Locale-aware date/number/relative-time formatting.
//
// All `format*` helpers take an explicit BCP-47 `locale` so they stay pure and
// unit-testable. React components should consume them through the
// `useFormatters()` hook (src/hooks/useFormatters.ts), which binds the active
// i18next language and re-renders on language change. Timestamps follow the
// app's DB convention: **unix seconds** (not milliseconds).

const EM_DASH = '—';

const SECOND = 1;
const MINUTE = 60;
const HOUR = 3_600;
const DAY = 86_400;
const MONTH = 2_592_000; // 30 days
const YEAR = 31_536_000; // 365 days

/**
 * Map a signed second delta to an `Intl.RelativeTimeFormat` (value, unit) pair.
 * `diffSeconds = now - ts`, so a positive delta is in the past and yields a
 * negative value ("2h ago" = `format(-2, 'hour')`). Pure — no clock, no Intl.
 */
export function relativeParts(diffSeconds: number): {
  value: number;
  unit: Intl.RelativeTimeFormatUnit;
} {
  const past = diffSeconds >= 0;
  const abs = Math.abs(diffSeconds);

  let magnitude: number;
  let unit: Intl.RelativeTimeFormatUnit;
  if (abs < MINUTE) {
    magnitude = Math.floor(abs / SECOND);
    unit = 'second';
  } else if (abs < HOUR) {
    magnitude = Math.floor(abs / MINUTE);
    unit = 'minute';
  } else if (abs < DAY) {
    magnitude = Math.floor(abs / HOUR);
    unit = 'hour';
  } else if (abs < MONTH) {
    magnitude = Math.floor(abs / DAY);
    unit = 'day';
  } else if (abs < YEAR) {
    magnitude = Math.floor(abs / MONTH);
    unit = 'month';
  } else {
    magnitude = Math.floor(abs / YEAR);
    unit = 'year';
  }

  return { value: past ? -magnitude : magnitude, unit };
}

/**
 * Format a unix-seconds timestamp as a localized relative phrase ("2h ago").
 * Returns the em-dash placeholder for null/undefined/0 (the app's "no value"
 * sentinel). `nowMs` is injectable for deterministic tests.
 */
export function formatRelativeTime(
  unixSeconds: number | null | undefined,
  locale: string,
  nowMs: number = Date.now(),
): string {
  if (!unixSeconds) return EM_DASH;
  const diffSeconds = Math.floor(nowMs / 1000) - unixSeconds;
  const { value, unit } = relativeParts(diffSeconds);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto', style: 'narrow' });
  return rtf.format(value, unit);
}

/** Format a unix-seconds timestamp as a localized absolute date. */
export function formatDate(
  unixSeconds: number | null | undefined,
  locale: string,
  options: Intl.DateTimeFormatOptions = { year: 'numeric', month: 'short', day: 'numeric' },
): string {
  if (!unixSeconds) return EM_DASH;
  return new Intl.DateTimeFormat(locale, options).format(new Date(unixSeconds * 1000));
}

/** Format a unix-seconds timestamp as a localized time-of-day. */
export function formatTime(
  unixSeconds: number | null | undefined,
  locale: string,
  options: Intl.DateTimeFormatOptions = { hour: '2-digit', minute: '2-digit' },
): string {
  if (!unixSeconds) return EM_DASH;
  return new Intl.DateTimeFormat(locale, options).format(new Date(unixSeconds * 1000));
}

/** Format a unix-seconds timestamp as a localized date + time. */
export function formatDateTime(
  unixSeconds: number | null | undefined,
  locale: string,
  options: Intl.DateTimeFormatOptions = {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  },
): string {
  if (!unixSeconds) return EM_DASH;
  return new Intl.DateTimeFormat(locale, options).format(new Date(unixSeconds * 1000));
}

/**
 * Format a unix-seconds timestamp the way a mail list column should read.
 *
 * Three tiers, each dropping the part the reader can already infer:
 *
 *  - **Today** → `10:04`. The date is the list's own context.
 *  - **Earlier this year** → `9 ago` / `Aug 9`. A month name is what the eye
 *    scans for; `03/08/2026` makes it parse three numbers to learn one thing,
 *    and the year is redundant.
 *  - **Another year** → `03/01/2025`. The year is the one thing an
 *    abbreviated date cannot express, so the numeric form earns its width.
 *
 * "Today" is a calendar-day comparison in local time, not an elapsed duration:
 * 23:59 yesterday is an hour old at 00:30 but is emphatically not today.
 *
 * `nowMs` is injectable so the tiering is testable without freezing the clock.
 */
export function formatInboxTimestamp(
  unixSeconds: number | null | undefined,
  locale: string,
  nowMs: number = Date.now(),
): string {
  if (!unixSeconds) return EM_DASH;
  const d = new Date(unixSeconds * 1000);
  const now = new Date(nowMs);

  if (d.getFullYear() !== now.getFullYear()) {
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()}`;
  }

  if (d.getMonth() === now.getMonth() && d.getDate() === now.getDate()) {
    return new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit', hour12: false }).format(d);
  }

  return new Intl.DateTimeFormat(locale, { day: 'numeric', month: 'short' }).format(d);
}

/** Format a number with locale-aware grouping/decimal separators. */
export function formatNumber(n: number, locale: string, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(locale, options).format(n);
}

/**
 * Format a byte count as a localized, human-readable size (B/KB/MB/GB/TB) using
 * binary (1024) steps. Up to one fractional digit, locale-aware decimal mark.
 */
export function formatBytes(bytes: number, locale: string): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exp;
  const formatted = formatNumber(value, locale, { maximumFractionDigits: 1 });
  return `${formatted} ${units[exp]}`;
}
