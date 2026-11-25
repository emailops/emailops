import type { CalendarEvent, RsvpChoice } from '@/types';

const same = (a: string, b: string): boolean => a.trim().toLowerCase() === b.trim().toLowerCase();

/** My meeting: I am the organizer, or it has none. Only then can I cancel it for everyone. */
export function isMyMeeting(event: CalendarEvent, selfEmail: string): boolean {
  return !event.organizer.trim() || same(event.organizer, selfEmail);
}

/** The organizer's address, or null when there is none or it is me. */
export function organizerRecipient(event: CalendarEvent, selfEmail: string): string | null {
  return isMyMeeting(event, selfEmail) ? null : event.organizer;
}

/** Organizer first, then attendees: minus me and empty addresses,
 *  de-duplicated case-insensitively, original order. */
export function participantRecipients(event: CalendarEvent, selfEmail: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const address of [event.organizer, ...event.attendees.map((a) => a.email)]) {
    const key = address.trim().toLowerCase();
    if (!key || same(address, selfEmail) || seen.has(key)) continue;
    seen.add(key);
    out.push(address);
  }
  return out;
}

const CHOICES: readonly string[] = ['accepted', 'declined', 'tentative'];

/** My current answer; `needsAction` (or no entry) is "not answered" → null. */
export function selfResponse(event: CalendarEvent, selfEmail: string): RsvpChoice | null {
  const mine = event.attendees.find((a) => same(a.email, selfEmail));
  return mine && CHOICES.includes(mine.response) ? (mine.response as RsvpChoice) : null;
}

/** I can answer only when the calendar is writable, I am an invited attendee and not the organizer. */
export function canRespond(event: CalendarEvent, selfEmail: string, readOnly: boolean): boolean {
  if (readOnly || !selfEmail) return false;
  if (same(event.organizer, selfEmail)) return false;
  const mine = event.attendees.find((a) => same(a.email, selfEmail));
  return mine !== undefined && mine.response !== 'organizer';
}
