// Pure helpers for the calendar event dialogs (create / detail).
//
// Kept free of React and Tauri so they are unit-testable: the components
// translate the returned keys via `calendar:*` and render the icons.

import { extractEmail } from './composeRecipients';

/** Recurrence values accepted by the `create_calendar_event` command. */
export type CalendarRecurrence = 'none' | 'daily' | 'weekly' | 'weekdays' | 'monthly' | 'yearly';

export interface RecurrenceOption {
  value: CalendarRecurrence;
  /** Leaf key under `calendar:create.recurrence.*`. */
  labelKey: CalendarRecurrence;
  /** Interpolation params — only the weekly option carries `{{weekday}}`. */
  params?: { weekday: string };
}

/**
 * The recurrence select options for an event on `date`: the weekly option is
 * anchored to `date`'s weekday ("Weekly on Wednesday"), localized via `locale`.
 */
export function recurrenceOptions(date: Date, locale: string): RecurrenceOption[] {
  const weekday = new Intl.DateTimeFormat(locale, { weekday: 'long' }).format(date);
  return [
    { value: 'none', labelKey: 'none' },
    { value: 'daily', labelKey: 'daily' },
    { value: 'weekly', labelKey: 'weekly', params: { weekday } },
    { value: 'weekdays', labelKey: 'weekdays' },
    { value: 'monthly', labelKey: 'monthly' },
    { value: 'yearly', labelKey: 'yearly' },
  ];
}

/** Normalized RSVP kind — doubles as the `calendar:detail.attendeeStatus.*` leaf key. */
export type AttendeeStatusKind = 'accepted' | 'declined' | 'tentative' | 'needsAction' | 'organizer';

/**
 * Map a provider attendee `response` (open string set) onto the UI's status
 * kind. Unknown/missing values read as "no response yet" (`needsAction`).
 */
export function attendeeStatusMeta(response: string): AttendeeStatusKind {
  switch (response) {
    case 'accepted':
    case 'declined':
    case 'tentative':
    case 'organizer':
      return response;
    default:
      return 'needsAction';
  }
}

/**
 * Basic email-shape check for the invitee chip input: local@domain.tld with
 * no whitespace. Accepts `Name <addr>` wrappers (normalized via `extractEmail`).
 * Deliberately loose — the backend does the authoritative validation.
 */
export function isValidInviteeEmail(raw: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(extractEmail(raw));
}

/**
 * The cancellation note to send with a delete. ALWAYS a string — the Tauri
 * command rejects `null` args. Non-empty only when attendees are notified and
 * the provider supports a custom message (Outlook; Google's API only sends
 * its standard cancellation email).
 */
export function cancellationMessage(notifyAttendees: boolean, provider: string, raw: string): string {
  if (!notifyAttendees || provider !== 'outlook') return '';
  return raw.trim();
}

/**
 * Attendees to render in the event-detail list. The event's organizer already
 * has a dedicated "Organizer" row, so their attendee entry (matched by email,
 * case-insensitively) is dropped to avoid showing them twice. An
 * organizer-status attendee with a *different* address (delegated calendars)
 * is kept — it isn't a duplicate of the Organizer row.
 */
export function visibleAttendees<T extends { email: string }>(attendees: readonly T[], organizerEmail: string): T[] {
  if (!organizerEmail) return [...attendees];
  const organizer = organizerEmail.toLowerCase();
  return attendees.filter((a) => a.email.toLowerCase() !== organizer);
}

/** Scope options accepted by the `delete_calendar_event` command. */
export type CalendarDeleteScope = 'instance' | 'following' | 'all';

/** The bits of the deleted event the scope filter needs. */
export interface DeletedEventRef {
  id: string;
  /** Non-null when the event is an instance of a recurring series. */
  recurringEventId: string | null;
  /** UTC epoch seconds of the deleted occurrence. */
  startTime: number;
}

/**
 * Events remaining in local state after a provider-side delete with `scope`:
 * - `instance`: only the deleted occurrence goes.
 * - `all`: every occurrence of the series goes (including the series master
 *   when it appears under the series id itself).
 * - `following`: same-series occurrences starting at or after the deleted
 *   one go.
 * Non-recurring events (`recurringEventId === null`) always behave like
 * `instance`. Pure — never mutates `events`.
 */
export function eventsAfterDelete<T extends DeletedEventRef>(
  events: readonly T[],
  deleted: DeletedEventRef,
  scope: CalendarDeleteScope,
): T[] {
  const seriesId = deleted.recurringEventId;
  const inSeries = (e: DeletedEventRef): boolean =>
    seriesId !== null && (e.recurringEventId === seriesId || e.id === seriesId);
  return events.filter((e) => {
    if (e.id === deleted.id) return false;
    if (scope === 'all') return !inSeries(e);
    if (scope === 'following') return !(inSeries(e) && e.startTime >= deleted.startTime);
    return true;
  });
}

/** Weekday codes an RRULE uses for "every weekday". */
const WEEKDAY_BYDAY = ['MO', 'TU', 'WE', 'TH', 'FR'];

/** Humanized recurrence for the invite card. `labelKey` is a leaf under
 *  `calendar:create.recurrence.*`; `null` means "show `raw` verbatim". */
export interface RecurrenceDescription {
  labelKey: 'daily' | 'weekly' | 'weekdays' | 'monthly' | 'yearly' | null;
  /** Only the weekly description carries `{{weekday}}`. */
  params?: { weekday: string };
  /** The original RRULE value, for the unrecognized-pattern fallback. */
  raw: string;
}

/**
 * Minimally humanize a raw RRULE value ("FREQ=WEEKLY;BYDAY=TU") for the
 * invite card. Recognizes the simple daily/weekly/weekday/monthly/yearly
 * shapes (ignoring COUNT/UNTIL bounds); anything more exotic — INTERVAL > 1,
 * multi-day BYDAY sets, unsupported frequencies, non-RRULE strings — falls
 * back to the raw value. The weekly label is anchored to `startDate`'s
 * weekday, localized via `locale` (same convention as `recurrenceOptions`).
 */
export function describeRecurrence(rrule: string, startDate: Date, locale: string): RecurrenceDescription {
  const raw: RecurrenceDescription = { labelKey: null, raw: rrule };

  const parts = new Map<string, string>();
  for (const piece of rrule.replace(/^RRULE:/i, '').split(';')) {
    const eq = piece.indexOf('=');
    if (eq <= 0) continue;
    parts.set(
      piece.slice(0, eq).trim().toUpperCase(),
      piece
        .slice(eq + 1)
        .trim()
        .toUpperCase(),
    );
  }

  const freq = parts.get('FREQ');
  if (!freq) return raw;

  const interval = parts.get('INTERVAL');
  if (interval !== undefined && Number(interval) !== 1) return raw;

  const byday = (parts.get('BYDAY') ?? '').split(',').filter((d) => d.length > 0);
  const isWeekdaySet = byday.length === WEEKDAY_BYDAY.length && WEEKDAY_BYDAY.every((code) => byday.includes(code));

  if ((freq === 'WEEKLY' || freq === 'DAILY') && isWeekdaySet) return { labelKey: 'weekdays', raw: rrule };

  switch (freq) {
    case 'DAILY':
      return byday.length === 0 ? { labelKey: 'daily', raw: rrule } : raw;
    case 'WEEKLY': {
      if (byday.length > 1) return raw;
      const weekday = new Intl.DateTimeFormat(locale, { weekday: 'long' }).format(startDate);
      return { labelKey: 'weekly', params: { weekday }, raw: rrule };
    }
    case 'MONTHLY':
      return { labelKey: 'monthly', raw: rrule };
    case 'YEARLY':
      return { labelKey: 'yearly', raw: rrule };
    default:
      return raw;
  }
}

/** One piece of a linkified plain-text run. */
export interface TextSegment {
  kind: 'text' | 'link';
  value: string;
}

/**
 * Split plain text into text/link segments so http(s) URLs can render as
 * clickable elements. Only detects http/https (the renderer still routes
 * through `getSafeExternalUrl` before opening). Trailing prose punctuation
 * stays with the surrounding text, mirroring the backend meeting-link
 * extractor's boundary rules.
 */
export function linkifySegments(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  const urlPattern = /https?:\/\/[^\s<>"']+/g;
  let cursor = 0;
  for (const match of text.matchAll(urlPattern)) {
    const start = match.index ?? 0;
    const url = match[0].replace(/[.,;:!?)\]}'"]+$/, '');
    if (url.length === 0) continue;
    if (start > cursor) segments.push({ kind: 'text', value: text.slice(cursor, start) });
    segments.push({ kind: 'link', value: url });
    cursor = start + url.length;
  }
  if (cursor < text.length) segments.push({ kind: 'text', value: text.slice(cursor) });
  return segments;
}
