// React binding for the locale-aware formatters in `src/lib/intl.ts`.
//
// Reads the active i18next language, pairs it with the OS region read at boot
// (`formattingLocale`), and returns formatters pre-bound to that locale.
// Because it subscribes via `useTranslation`, every consumer re-renders (and
// re-formats) when the user switches language — no manual locale threading.
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import { osRegion } from '@/i18n';
import {
  formatBytes,
  formatDate,
  formatDateTime,
  formatNumber,
  formatReceptionTime,
  formatRelativeTime,
  formatTime,
  formattingLocale,
} from '@/lib/intl';

export interface Formatters {
  relativeTime: (unixSeconds: number | null | undefined) => string;
  date: (unixSeconds: number | null | undefined, options?: Intl.DateTimeFormatOptions) => string;
  time: (unixSeconds: number | null | undefined, options?: Intl.DateTimeFormatOptions) => string;
  dateTime: (unixSeconds: number | null | undefined, options?: Intl.DateTimeFormatOptions) => string;
  /** The email list's date column (`formatReceptionTime`). */
  receivedAt: (unixSeconds: number) => string;
  number: (n: number, options?: Intl.NumberFormatOptions) => string;
  bytes: (n: number) => string;
}

export function useFormatters(): Formatters {
  const { i18n } = useTranslation();
  const locale = formattingLocale(i18n.language || 'en', osRegion());

  return useMemo<Formatters>(
    () => ({
      relativeTime: (ts) => formatRelativeTime(ts, locale),
      date: (ts, options) => formatDate(ts, locale, options),
      time: (ts, options) => formatTime(ts, locale, options),
      dateTime: (ts, options) => formatDateTime(ts, locale, options),
      receivedAt: (ts) => formatReceptionTime(ts, locale, Date.now()),
      number: (n, options) => formatNumber(n, locale, options),
      bytes: (n) => formatBytes(n, locale),
    }),
    [locale],
  );
}
