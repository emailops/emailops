/**
 * Snooze times offered by the picker, computed in the user's local time zone.
 * Pure: `now` is passed in, so the rules are testable across days of the week
 * and daylight-saving changes.
 */

export type SnoozePresetId = 'laterToday' | 'tomorrow' | 'thisWeekend' | 'nextWeek';

export interface SnoozePreset {
  id: SnoozePresetId;
  at: Date;
}

/** Wall-clock hour the day-based presets wake at. */
const MORNING_HOUR = 8;
/** "Later today" is this many real hours ahead, up to the next full hour. */
const LATER_TODAY_HOURS = 3;
/** From this hour on, "later today" would be tonight: tomorrow covers it. */
const LATER_TODAY_UNTIL_HOUR = 18;

/** `daysAhead` days from `now`'s date at 8:00 local time. The Date
 *  constructor resolves the wall-clock time, so a DST change in between keeps
 *  the 8:00. */
function morning(now: Date, daysAhead: number): Date {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + daysAhead, MORNING_HOUR, 0, 0, 0);
}

/**
 * The presets for `now`, soonest first:
 * - **Later today**: three hours from now, rounded up to the full hour; not
 *   offered from 18:00 on.
 * - **Tomorrow**: tomorrow at 8:00.
 * - **This weekend**: Saturday at 8:00, offered Monday to Friday.
 * - **Next week**: next Monday at 8:00.
 *
 * A preset landing on the same instant as an earlier one (this weekend on a
 * Friday, next week on a Sunday) is left out.
 */
export function snoozePresets(now: Date): SnoozePreset[] {
  const candidates: SnoozePreset[] = [];
  if (now.getHours() < LATER_TODAY_UNTIL_HOUR) {
    const at = new Date(now.getTime() + LATER_TODAY_HOURS * 3600 * 1000);
    if (at.getMinutes() !== 0 || at.getSeconds() !== 0 || at.getMilliseconds() !== 0) {
      at.setMinutes(60, 0, 0);
    }
    candidates.push({ id: 'laterToday', at });
  }
  candidates.push({ id: 'tomorrow', at: morning(now, 1) });
  const weekday = now.getDay(); // 0 = Sunday … 6 = Saturday
  if (weekday >= 1 && weekday <= 5) candidates.push({ id: 'thisWeekend', at: morning(now, 6 - weekday) });
  candidates.push({ id: 'nextWeek', at: morning(now, (8 - weekday) % 7 || 7) });

  const seen = new Set<number>();
  return candidates.filter((p) => {
    if (seen.has(p.at.getTime())) return false;
    seen.add(p.at.getTime());
    return true;
  });
}

const DATETIME_LOCAL = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/;

/** A `<input type="datetime-local">` value as a local Date, or null when it is
 *  malformed or not after `now` (a snooze must end in the future). */
export function parseCustomSnooze(value: string, now: Date): Date | null {
  const m = DATETIME_LOCAL.exec(value);
  if (!m) return null;
  const [, y, mo, d, h, mi] = m.map(Number);
  const at = new Date(y, mo - 1, d, h, mi);
  if (Number.isNaN(at.getTime()) || at.getTime() <= now.getTime()) return null;
  return at;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** `date` as a `datetime-local` input value (local time, minutes). */
export function toDatetimeLocalValue(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(
    date.getMinutes(),
  )}`;
}

/** Unix seconds, the unit the backend stores. */
export function toUnixSeconds(date: Date): number {
  return Math.floor(date.getTime() / 1000);
}
