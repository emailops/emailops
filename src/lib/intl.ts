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
