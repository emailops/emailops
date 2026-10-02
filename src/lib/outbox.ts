import type { OutgoingMessage } from '@/lib/api';
import { plainTextToHtml } from '@/lib/composeHtml';

/**
 * Undo send and scheduled send — the pure rules. Both go through the local
 * outbox (`queue_outgoing_email`): undo send is a send a few seconds in the
 * future, scheduled send one at a chosen time.
 */

/** Preference key (SQLite user prefs) holding the undo window in seconds. */
export const UNDO_SEND_DELAY_PREF = 'compose.undo_send_delay_secs';

/** The windows Settings offers; 0 = off (Send sends at once). */
export const UNDO_SEND_DELAY_OPTIONS = [0, 5, 10, 20, 30] as const;

export const DEFAULT_UNDO_SEND_DELAY = 10;

/** The stored preference as a window in seconds; unset or unknown values are
 *  the default. */
export function parseUndoSendDelay(raw: string | null | undefined): number {
  if (raw === null || raw === undefined || raw.trim() === '') return DEFAULT_UNDO_SEND_DELAY;
  const secs = Number(raw);
  return (UNDO_SEND_DELAY_OPTIONS as readonly number[]).includes(secs) ? secs : DEFAULT_UNDO_SEND_DELAY;
}

export type ScheduleSendPresetId = 'tomorrowMorning' | 'tomorrowAfternoon' | 'mondayMorning';

export interface ScheduleSendPreset {
  id: ScheduleSendPresetId;
  at: Date;
}

const MORNING_HOUR = 8;
const AFTERNOON_HOUR = 13;

/** `daysAhead` days after `now`'s date at `hour`:00 local time. The Date
 *  constructor resolves the wall-clock time, so month ends and DST changes
 *  keep the hour. */
function dayAt(now: Date, daysAhead: number, hour: number): Date {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + daysAhead, hour, 0, 0, 0);
}

/**
 * The "Schedule send" presets for `now`, in local time:
 * - **Tomorrow morning** 08:00, **tomorrow afternoon** 13:00;
 * - **Monday morning** 08:00 — next Monday (a week ahead on a Monday), left
 *   out on a Sunday where it is the same instant as tomorrow morning.
 */
export function scheduleSendPresets(now: Date): ScheduleSendPreset[] {
  const presets: ScheduleSendPreset[] = [
    { id: 'tomorrowMorning', at: dayAt(now, 1, MORNING_HOUR) },
    { id: 'tomorrowAfternoon', at: dayAt(now, 1, AFTERNOON_HOUR) },
  ];
  const weekday = now.getDay(); // 0 = Sunday … 6 = Saturday
  const daysToMonday = (8 - weekday) % 7 || 7;
  if (daysToMonday !== 1) presets.push({ id: 'mondayMorning', at: dayAt(now, daysToMonday, MORNING_HOUR) });
  return presets;
}

/**
 * The editor HTML for a message taken back from the outbox (undo, edit): the
 * inline images the send turned into `cid:` references go back to `data:`
 * URIs, so the composer shows them and sends them again; a plain-text message
 * becomes paragraphs.
 */
export function restoredBodyHtml(message: Pick<OutgoingMessage, 'body' | 'bodyHtml' | 'inlineImages'>): string {
  if (!message.bodyHtml) return plainTextToHtml(message.body);
  let html = message.bodyHtml;
  for (const img of message.inlineImages) {
    if (!img.contentId) continue;
    html = html.split(`cid:${img.contentId}`).join(`data:${img.mimeType};base64,${img.data}`);
  }
  return html;
}
