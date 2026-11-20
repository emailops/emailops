import type { CalendarEvent, RsvpChoice } from '@/types';

const same = (a: string, b: string): boolean => a.trim().toLowerCase() === b.trim().toLowerCase();

/** The organizer's address, or null when there is none or it is me. */
export function organizerRecipient(event: CalendarEvent, selfEmail: string): string | null {
  if (!event.organizer.trim() || same(event.organizer, selfEmail)) return null;
  return event.organizer;
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

/** I can answer an invitation only when the calendar is writable and I am not its organizer. */
export function canRespond(event: CalendarEvent, selfEmail: string, readOnly: boolean): boolean {
  if (readOnly || !selfEmail) return false;
  if (same(event.organizer, selfEmail)) return false;
  return !event.attendees.some((a) => same(a.email, selfEmail) && a.response === 'organizer');
}
