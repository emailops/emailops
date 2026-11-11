import type { Calendar } from '@/types';

/** Host of Proton Calendar's web app and of its "share via link" feeds. */
export const PROTON_CALENDAR_HOST = 'calendar.proton.me';

/** Calendars the user cannot write to (subscribed feeds, free/busy shares). */
export function isReadOnlyCalendar(calendar: Pick<Calendar, 'accessRole'>): boolean {
  return calendar.accessRole === 'reader' || calendar.accessRole === 'freeBusyReader';
}

/** A read-only calendar fed by a Proton Calendar share link. */
export function isProtonFeed(calendar: Pick<Calendar, 'feedHost'>): boolean {
  return calendar.feedHost?.toLowerCase() === PROTON_CALENDAR_HOST;
}

/**
 * Proton Calendar's create-event deep link for a local day. Its web client
 * reads the day from the path and `action=create` opens the new-event form
 * (`title`, `location`, `description` are the only prefillable fields; the
 * time defaults to now and the calendar to the user's default one).
 */
export function protonCreateEventUrl(day: Date): string {
  const params = new URLSearchParams({ action: 'create', title: '' });
  return `https://${PROTON_CALENDAR_HOST}/week/${day.getFullYear()}/${day.getMonth() + 1}/${day.getDate()}?${params}`;
}

export type CreateSlotPlan = { kind: 'dialog' } | { kind: 'proton'; url: string } | { kind: 'none' };

/**
 * What a double-click on an empty slot does for an account: the in-app
 * dialog when it has any writable calendar (events are created on the
 * primary one), a hand-over to Proton Calendar when its visible calendars
 * are Proton feeds, nothing for other read-only feeds.
 */
export function planCreateSlot(calendars: readonly Calendar[], slotStartSec: number): CreateSlotPlan {
  if (calendars.some((c) => !isReadOnlyCalendar(c))) return { kind: 'dialog' };
  if (calendars.some((c) => c.isVisible && isProtonFeed(c))) {
    return { kind: 'proton', url: protonCreateEventUrl(new Date(slotStartSec * 1000)) };
  }
  return { kind: 'none' };
}
