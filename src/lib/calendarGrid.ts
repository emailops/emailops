// Pure date/grid helpers for the Calendar view.
//
// Everything here operates on **local time** via `Date` (the calendar renders
// the user's local day/week/month), and stays deterministic by taking the
// anchor date / "now" as explicit parameters — no hidden clock reads.
// Timestamps follow the app's DB convention: unix **seconds**.

import type { CalendarEvent } from '@/types';

/** One cell of the 6×7 month grid. */
export interface MonthCell {
  /** Local midnight of the cell's day. */
  date: Date;
  /** False for the dimmed adjacent-month filler days. */
  inMonth: boolean;
}

/** An event with its column slot inside a week/day time column. */
export interface PositionedEvent {
  event: CalendarEvent;
  /** 0-based column index inside the overlap cluster. */
  column: number;
  /** Total columns in the event's overlap cluster (≥ 1). */
  columns: number;
}

/** How far away an event start is, for the reminder banner. */
export type StartsIn = { kind: 'started' } | { kind: 'now' } | { kind: 'minutes'; minutes: number };

/** Local midnight of `d`'s day. */
export function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

/** Calendar-day arithmetic (DST-safe — never adds raw milliseconds). */
export function addDays(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
}

/** Local midnight of the Monday on or before `d`. */
export function startOfWeekMonday(d: Date): Date {
  const mondayOffset = (d.getDay() + 6) % 7; // Mon=0 … Sun=6
  return addDays(startOfDay(d), -mondayOffset);
}

/** The 7 local days (Mon–Sun) of the week containing `anchor`. */
export function weekDays(anchor: Date): Date[] {
  const monday = startOfWeekMonday(anchor);
  return Array.from({ length: 7 }, (_, i) => addDays(monday, i));
}

/**
 * Classic 6×7 month grid for `anchor`'s month: 42 cells starting at the
 * Monday on or before the 1st, with adjacent-month days flagged `inMonth: false`.
 */
export function monthGrid(anchor: Date): MonthCell[] {
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const gridStart = startOfWeekMonday(first);
  return Array.from({ length: 42 }, (_, i) => {
    const date = addDays(gridStart, i);
    return {
      date,
      inMonth: date.getMonth() === anchor.getMonth() && date.getFullYear() === anchor.getFullYear(),
    };
  });
}

/** Effective exclusive end, guarding against zero/negative durations. */
function effectiveEnd(e: CalendarEvent): number {
  return Math.max(e.endTime, e.startTime + 1);
}

/**
 * Events overlapping the **local** day of `day`, sorted for display:
 * all-day first, then by start time, longer events before shorter ones on ties.
 * `endTime` is exclusive — an event ending exactly at midnight belongs to the
 * previous day only.
 */
export function eventsForDay(events: CalendarEvent[], day: Date): CalendarEvent[] {
  const dayStart = Math.floor(startOfDay(day).getTime() / 1000);
  const dayEnd = Math.floor(addDays(day, 1).getTime() / 1000);
  return events
    .filter((e) => e.startTime < dayEnd && effectiveEnd(e) > dayStart)
    .sort((a, b) => {
      if (a.isAllDay !== b.isAllDay) return a.isAllDay ? -1 : 1;
      if (a.startTime !== b.startTime) return a.startTime - b.startTime;
      if (a.endTime !== b.endTime) return b.endTime - a.endTime;
      return a.title.localeCompare(b.title);
    });
}

/**
 * Assign column slots to (timed) events sharing one day column, Google
 * Calendar-style: transitively-overlapping events form a cluster; within a
 * cluster each event takes the lowest free column, and every member reports
 * the cluster's total column count so the renderer can split the width evenly.
 * Touching events (`end === next start`) do not overlap (exclusive end).
 */
export function layoutDayEvents(events: CalendarEvent[]): PositionedEvent[] {
  const sorted = [...events].sort((a, b) => a.startTime - b.startTime || b.endTime - a.endTime);

  const result: PositionedEvent[] = [];
  let clusterItems: PositionedEvent[] = [];
  let columnEnds: number[] = [];
  let clusterMaxEnd = Number.NEGATIVE_INFINITY;

  const flushCluster = () => {
    const columns = Math.max(columnEnds.length, 1);
    for (const item of clusterItems) item.columns = columns;
    result.push(...clusterItems);
    clusterItems = [];
    columnEnds = [];
    clusterMaxEnd = Number.NEGATIVE_INFINITY;
  };

  for (const event of sorted) {
    if (clusterItems.length > 0 && event.startTime >= clusterMaxEnd) flushCluster();
    const end = effectiveEnd(event);
    let column = columnEnds.findIndex((colEnd) => colEnd <= event.startTime);
    if (column === -1) {
      column = columnEnds.length;
      columnEnds.push(end);
    } else {
      columnEnds[column] = end;
    }
    clusterItems.push({ event, column, columns: 1 });
    clusterMaxEnd = Math.max(clusterMaxEnd, end);
  }
  flushCluster();
  return result;
}

// ── Multi-day events ─────────────────────────────────────────────────────────

/** A day in seconds. An event must run *longer* than this to be multi-day. */
const ONE_DAY_SECS = 86_400;

/**
 * Whether an event spans several days and so belongs in the spanning band
 * above the grid rather than as a block inside the day columns.
 *
 * The test is duration, not "does it touch two dates": a 23:00 → 00:30 meeting
 * touches two dates but is 90 minutes long and reads correctly as two blocks in
 * the time grid. A one-day all-day event is exactly 24h, so the comparison is
 * deliberately strict — only something longer than a full day spans.
 */
export function isMultiDayEvent(e: CalendarEvent): boolean {
  return effectiveEnd(e) - e.startTime > ONE_DAY_SECS;
}

/** A multi-day event placed across the visible day columns. */
export interface EventSpan {
  event: CalendarEvent;
  /** First covered column, clamped to the visible range. */
  startIndex: number;
  /** Last covered column (inclusive), clamped to the visible range. */
  endIndex: number;
  /** The event already started before the first visible day. */
  continuesBefore: boolean;
  /** The event runs past the last visible day. */
  continuesAfter: boolean;
  /** Stacking row, so overlapping spans never draw on top of each other. */
  lane: number;
}

/**
 * Place every multi-day event in `events` across the `days` columns as a single
 * continuous bar, stacked into lanes so overlapping spans stay readable.
 *
 * Single-day events are skipped (they belong in the day columns), as are events
 * that do not reach the visible range at all. Spans are clamped to the visible
 * days, with `continuesBefore` / `continuesAfter` recording that the real event
 * extends further.
 */
export function layoutEventSpans(events: CalendarEvent[], days: Date[]): EventSpan[] {
  if (days.length === 0) return [];
  const dayStarts = days.map((d) => Math.floor(startOfDay(d).getTime() / 1000));
  const dayEnds = days.map((d) => Math.floor(addDays(d, 1).getTime() / 1000));
  const rangeStart = dayStarts[0];
  const rangeEnd = dayEnds[dayEnds.length - 1];

  const candidates = events
    .filter(isMultiDayEvent)
    .filter((e) => e.startTime < rangeEnd && effectiveEnd(e) > rangeStart)
    .map((event) => {
      const end = effectiveEnd(event);
      const startIndex = dayEnds.findIndex((dayEnd) => dayEnd > event.startTime);
      // Last column the event still covers: exclusive end, so a span ending
      // exactly at midnight stops on the previous day.
      let endIndex = startIndex;
      for (let i = days.length - 1; i >= 0; i -= 1) {
        if (dayStarts[i] < end) {
          endIndex = i;
          break;
        }
      }
      return {
        event,
        startIndex: Math.max(startIndex, 0),
        endIndex,
        continuesBefore: event.startTime < rangeStart,
        continuesAfter: end > rangeEnd,
      };
    })
    // Longest bars first within a start column, so the eye follows stable rows.
    .sort(
      (a, b) =>
        a.startIndex - b.startIndex ||
        b.endIndex - b.startIndex - (a.endIndex - a.startIndex) ||
        a.event.title.localeCompare(b.event.title),
    );

  /** Last occupied column per lane; a lane frees up the column after. */
  const laneEnds: number[] = [];
  return candidates.map((candidate) => {
    let lane = laneEnds.findIndex((occupiedThrough) => occupiedThrough < candidate.startIndex);
    if (lane === -1) {
      lane = laneEnds.length;
    }
    laneEnds[lane] = candidate.endIndex;
    return { ...candidate, lane };
  });
}

/** How many lanes the spanning band needs to draw `spans`. */
export function spanLaneCount(spans: readonly EventSpan[]): number {
  return spans.reduce((max, s) => Math.max(max, s.lane + 1), 0);
}

// ── Event block presentation (week/day time grid) ────────────────────────────

/** What an event block has room to render, given its pixel height. */
export type EventTextMode = 'two-lines' | 'one-line' | 'no-text';

/** Minimum rendered block height (px) — fits one 11px/leading-tight text line
 *  plus the block's vertical padding, and keeps tiny events clickable. */
export const EVENT_MIN_BLOCK_PX = 18;
/** Below this height text would touch the block edges — render none at all. */
export const EVENT_TEXT_ONE_LINE_MIN_PX = 20;
/** Two 14px lines + 4px vertical padding need at least this much room. */
export const EVENT_TEXT_TWO_LINES_MIN_PX = 32;
/** Right-hand strip of the day column (in %) left free of event blocks so the
 *  underlying slot stays visible and double-clickable (Google Calendar-style). */
export const EVENT_RIGHT_GUTTER_PCT = 10;

/**
 * Decide how much text a week/day event block can show from its rendered
 * pixel height: title + time on two lines, "title · time" inline on one
 * line, or nothing at all (tooltip only) for tiny blocks.
 */
export function eventTextMode(blockHeightPx: number): EventTextMode {
  if (blockHeightPx >= EVENT_TEXT_TWO_LINES_MIN_PX) return 'two-lines';
  if (blockHeightPx >= EVENT_TEXT_ONE_LINE_MIN_PX) return 'one-line';
  return 'no-text';
}

/**
 * Horizontal placement (in % of the day column) for an event in its overlap
 * cluster: concurrent events split the non-gutter width equally, side by
 * side, always leaving `EVENT_RIGHT_GUTTER_PCT` free on the right.
 */
export function eventColumnGeometry(column: number, columns: number): { leftPct: number; widthPct: number } {
  const available = 100 - EVENT_RIGHT_GUTTER_PCT;
  const widthPct = available / Math.max(columns, 1);
  return { leftPct: column * widthPct, widthPct };
}

/** Minutes in the last snappable slot of a day (23:30). */
const LAST_SLOT_MIN = 23 * 60 + 30;
const SLOT_STEP_MIN = 30;
const DEFAULT_EVENT_DURATION_MIN = 60;

/**
 * Convert a double-click's Y offset inside a day column into a proposed event
 * slot: snapped **down** to the nearest 30 minutes, one hour long, clamped to
 * `[00:00, 23:30]` starts. Returns unix seconds (the end may cross midnight
 * into the next day for the last slots).
 */
export function slotFromOffsetY(offsetY: number, dayStart: Date, pxPerHour: number): { start: number; end: number } {
  const rawMinutes = (offsetY / pxPerHour) * 60;
  const snapped = Math.min(Math.max(Math.floor(rawMinutes / SLOT_STEP_MIN) * SLOT_STEP_MIN, 0), LAST_SLOT_MIN);
  const at = (minutes: number) =>
    Math.floor(new Date(dayStart.getFullYear(), dayStart.getMonth(), dayStart.getDate(), 0, minutes).getTime() / 1000);
  return { start: at(snapped), end: at(snapped + DEFAULT_EVENT_DURATION_MIN) };
}

/**
 * Distance to an event start for the reminder banner. `< 60 s` away reads as
 * "starting now"; otherwise minutes rounded to the nearest whole minute.
 */
export function startsIn(startTimeSec: number, nowSec: number): StartsIn {
  const diff = startTimeSec - nowSec;
  if (diff <= 0) return { kind: 'started' };
  if (diff < 60) return { kind: 'now' };
  return { kind: 'minutes', minutes: Math.max(1, Math.round(diff / 60)) };
}

/**
 * Pick the calendar's account: the persisted selection if it still names an
 * enabled account, else the caller's preferred (effective) account, else the
 * first enabled account, else null.
 */
export function resolveCalendarAccountId(
  accounts: ReadonlyArray<{ id: string; enabled: boolean }>,
  persistedId: string | null,
  preferredId: string | null,
): string | null {
  const isCandidate = (id: string | null): id is string =>
    id !== null && accounts.some((a) => a.id === id && a.enabled);
  if (isCandidate(persistedId)) return persistedId;
  if (isCandidate(preferredId)) return preferredId;
  return accounts.find((a) => a.enabled)?.id ?? null;
}
