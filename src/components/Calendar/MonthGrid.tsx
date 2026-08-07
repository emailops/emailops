import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import { eventBlockStyle, eventChipStyle } from '@/lib/calendarColor';
import {
  eventsForDay,
  isMultiDayEvent,
  layoutEventSpans,
  monthGrid,
  spanLaneCount,
  startOfDay,
} from '@/lib/calendarGrid';
import type { CalendarEvent } from '@/types';

/** Max event chips per day cell before collapsing into "+N more". */
const MAX_CHIPS = 3;

/** Default slot proposed when double-clicking a month cell: 09:00–10:00. */
const DEFAULT_CREATE_HOUR = 9;

/** Height of one multi-day lane (bar + gap), matching the week view's band. */
const SPAN_LANE_PX = 18;

/** Vertical room the day-number row takes, so span bars start below it. */
const DAY_NUMBER_ROW_PX = 28;

const DAYS_PER_WEEK = 7;

interface MonthGridProps {
  anchor: Date;
  events: CalendarEvent[];
  /** Resolved colour per `calendarId` — see `calendarColor`. */
  colorFor: (calendarId: string) => string;
  onSelectEvent: (event: CalendarEvent) => void;
  /** "+N more" clicked → open that day in Day view. */
  onOpenDay: (day: Date) => void;
  /** Double-click on a day cell → propose a new event `[start, end)` (unix seconds). */
  onCreateSlot: (start: number, end: number) => void;
}

/** Classic 6×7 Google Calendar-style month grid (weeks start Monday). */
export function MonthGrid({ anchor, events, colorFor, onSelectEvent, onOpenDay, onCreateSlot }: MonthGridProps) {
  const { t, i18n } = useTranslation(['calendar']);
  const { time } = useFormatters();
  const cells = useMemo(() => monthGrid(anchor), [anchor]);
  const todayMs = startOfDay(new Date()).getTime();

  const weekdayLabels = useMemo(() => {
    const fmt = new Intl.DateTimeFormat(i18n.language || 'en', { weekday: 'short' });
    return cells.slice(0, DAYS_PER_WEEK).map((c) => fmt.format(c.date));
  }, [cells, i18n.language]);

  // The grid is built a week at a time: a multi-day event is drawn once as a
  // bar across that week's days instead of repeating its chip (and its start
  // time) in every cell it touches. An event crossing a week boundary gets one
  // clamped bar per row, with a chevron marking the continuation.
  const weeks = useMemo(() => {
    const rows = [];
    for (let i = 0; i < cells.length; i += DAYS_PER_WEEK) {
      const weekCells = cells.slice(i, i + DAYS_PER_WEEK);
      const spans = layoutEventSpans(
        events,
        weekCells.map((c) => c.date),
      );
      rows.push({ weekCells, spans, laneCount: spanLaneCount(spans) });
    }
    return rows;
  }, [cells, events]);

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="grid grid-cols-7 border-b border-gray-200 flex-shrink-0 dark:border-gray-700">
        {weekdayLabels.map((label) => (
          <div
            key={label}
            className="px-2 py-1 text-xs font-semibold text-gray-500 uppercase tracking-wider text-center dark:text-gray-400"
          >
            {label}
          </div>
        ))}
      </div>
      <div className="flex flex-col flex-1 min-h-0">
        {weeks.map(({ weekCells, spans, laneCount }) => (
          <div key={weekCells[0].date.getTime()} className="flex-1 min-h-0 relative flex">
            {weekCells.map((cell) => {
              const dayEvents = eventsForDay(events, cell.date).filter((e) => !isMultiDayEvent(e));
              // Span bars eat into the cell's vertical room, so fewer chips fit.
              const maxChips = Math.max(1, MAX_CHIPS - laneCount);
              const visible = dayEvents.slice(0, maxChips);
              const overflow = dayEvents.length - visible.length;
              const isToday = cell.date.getTime() === todayMs;
              return (
                <div
                  key={cell.date.getTime()}
                  className={`flex-1 min-w-0 border-b border-r border-gray-100 p-1 min-h-0 overflow-hidden flex flex-col dark:border-gray-800 ${
                    cell.inMonth ? 'bg-white dark:bg-surface' : 'bg-gray-50 dark:bg-surface-raised'
                  }`}
                  onDoubleClick={(e) => {
                    // Chips and "+N more" own their clicks — only empty cell space creates.
                    if ((e.target as HTMLElement).closest('button')) return;
                    const d = cell.date;
                    const start = new Date(d.getFullYear(), d.getMonth(), d.getDate(), DEFAULT_CREATE_HOUR);
                    const end = new Date(d.getFullYear(), d.getMonth(), d.getDate(), DEFAULT_CREATE_HOUR + 1);
                    onCreateSlot(Math.floor(start.getTime() / 1000), Math.floor(end.getTime() / 1000));
                  }}
                >
                  <div className="flex justify-end flex-shrink-0">
                    <span
                      className={`text-xs w-6 h-6 flex items-center justify-center rounded-full ${
                        isToday
                          ? 'bg-primary-600 text-white font-semibold'
                          : cell.inMonth
                            ? 'text-gray-700 dark:text-gray-300'
                            : 'text-gray-400 dark:text-gray-500'
                      }`}
                    >
                      {cell.date.getDate()}
                    </span>
                  </div>
                  <div className="space-y-0.5 min-h-0 overflow-hidden" style={{ marginTop: laneCount * SPAN_LANE_PX }}>
                    {visible.map((event) => (
                      <button
                        key={event.id}
                        onClick={() => onSelectEvent(event)}
                        title={event.title}
                        className={`w-full text-left px-1.5 py-0.5 rounded text-[11px] leading-tight truncate transition-opacity hover:opacity-80 ${
                          event.isAllDay ? '' : 'border text-gray-900 dark:text-gray-100'
                        }`}
                        style={
                          event.isAllDay
                            ? eventChipStyle(colorFor(event.calendarId))
                            : eventBlockStyle(colorFor(event.calendarId), event.status === 'tentative')
                        }
                      >
                        {!event.isAllDay && <span className="font-medium mr-1">{time(event.startTime)}</span>}
                        {event.title || '—'}
                      </button>
                    ))}
                    {overflow > 0 && (
                      <button
                        onClick={() => onOpenDay(cell.date)}
                        className="w-full text-left px-1.5 py-0.5 text-[11px] text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded dark:text-gray-400 dark:hover:text-gray-300 dark:hover:bg-surface-hover"
                      >
                        {t('calendar:moreEvents', { n: overflow })}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}

            {/* Multi-day bars, drawn over the week's cells. The layer ignores
                pointer events so the cells underneath stay double-clickable;
                each bar re-enables them for itself. */}
            {spans.length > 0 && (
              <div
                className="absolute inset-x-0 pointer-events-none"
                style={{ top: DAY_NUMBER_ROW_PX, height: laneCount * SPAN_LANE_PX }}
              >
                {spans.map(({ event, startIndex, endIndex, continuesBefore, continuesAfter, lane }) => (
                  <button
                    key={event.id}
                    onClick={() => onSelectEvent(event)}
                    title={event.title}
                    className={`absolute pointer-events-auto text-left px-1.5 py-0.5 text-[11px] leading-tight truncate transition-opacity hover:opacity-80 flex items-center gap-1 ${
                      continuesBefore ? '' : 'rounded-l'
                    } ${continuesAfter ? '' : 'rounded-r'}`}
                    style={{
                      top: lane * SPAN_LANE_PX,
                      left: `calc(${(startIndex / DAYS_PER_WEEK) * 100}% + 3px)`,
                      width: `calc(${((endIndex - startIndex + 1) / DAYS_PER_WEEK) * 100}% - 6px)`,
                      height: SPAN_LANE_PX - 2,
                      ...eventChipStyle(colorFor(event.calendarId)),
                    }}
                  >
                    {continuesBefore && <span aria-hidden="true">‹</span>}
                    <span className="truncate flex-1">{event.title || '—'}</span>
                    {continuesAfter && <span aria-hidden="true">›</span>}
                  </button>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
