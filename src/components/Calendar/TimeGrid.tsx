import { useLayoutEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import { eventBlockStyle, eventChipStyle } from '@/lib/calendarColor';
import {
  addDays,
  EVENT_MIN_BLOCK_PX,
  eventColumnGeometry,
  eventsForDay,
  eventTextMode,
  isMultiDayEvent,
  layoutDayEvents,
  layoutEventSpans,
  slotFromOffsetY,
  spanLaneCount,
  startOfDay,
} from '@/lib/calendarGrid';
import type { CalendarEvent } from '@/types';

/** Pixel height of one hour row. */
const HOUR_PX = 48;
const MINUTES_PER_DAY = 24 * 60;
/** On mount / view switch the grid scrolls so this hour sits at the top. */
const INITIAL_SCROLL_HOUR = 7;
/** Height of one lane in the multi-day spanning band (bar + gap). */
const SPAN_LANE_PX = 20;

interface TimeGridProps {
  /** The local days to render as columns (7 for week view, 1 for day view). */
  days: Date[];
  events: CalendarEvent[];
  /** Resolved colour per `calendarId`, so each calendar's events are tinted
   *  with the colour the provider shows them in. */
  colorFor: (calendarId: string) => string;
  onSelectEvent: (event: CalendarEvent) => void;
  /** Double-click on an empty slot → propose a new event `[start, end)` (unix seconds). */
  onCreateSlot: (start: number, end: number) => void;
}

/**
 * Week/Day time grid: an all-day row on top, then a time gutter with
 * absolutely-positioned event blocks. Overlapping events share the column
 * width via `layoutDayEvents`.
 */
export function TimeGrid({ days, events, colorFor, onSelectEvent, onCreateSlot }: TimeGridProps) {
  const { t, i18n } = useTranslation(['calendar']);
  const { time } = useFormatters();
  const todayMs = startOfDay(new Date()).getTime();

  // Start the scroll at INITIAL_SCROLL_HOUR on mount and when the view shape
  // changes (week ↔ day). Never re-applied afterwards — the user's manual
  // scroll position survives event reloads and prev/next navigation.
  //
  // Deferred two animation frames past the commit on purpose: setting
  // scrollTop synchronously while WKWebView is still compositing the freshly
  // mounted scroll container can leave hit-testing misaligned with the
  // painted content — events render at 07:00+ but clicks resolve against the
  // pre-scroll layout, so "clicking does nothing" until a remount repaints.
  const scrollRef = useRef<HTMLDivElement>(null);
  const dayCount = days.length;
  useLayoutEffect(() => {
    if (dayCount === 0) return;
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        if (scrollRef.current) scrollRef.current.scrollTop = INITIAL_SCROLL_HOUR * HOUR_PX;
      });
    });
    return () => {
      cancelAnimationFrame(firstFrame);
      cancelAnimationFrame(secondFrame);
    };
  }, [dayCount]);

  const hourLabels = useMemo(() => {
    const fmt = new Intl.DateTimeFormat(i18n.language || 'en', { hour: 'numeric' });
    const base = new Date(2026, 0, 1);
    return Array.from({ length: 24 }, (_, h) => fmt.format(new Date(base.getFullYear(), 0, 1, h)));
  }, [i18n.language]);

  const dayLabelFmt = useMemo(
    () => new Intl.DateTimeFormat(i18n.language || 'en', { weekday: 'short', day: 'numeric' }),
    [i18n.language],
  );

  // Multi-day events are drawn once as a bar across the days they cover, not
  // repeated as a full-height block in every column — so they are pulled out of
  // the per-day lists entirely.
  const spans = useMemo(() => layoutEventSpans(events, days), [events, days]);
  const laneCount = spanLaneCount(spans);

  const perDay = useMemo(
    () =>
      days.map((day) => {
        const dayEvents = eventsForDay(events, day).filter((e) => !isMultiDayEvent(e));
        return {
          day,
          allDay: dayEvents.filter((e) => e.isAllDay),
          timed: layoutDayEvents(dayEvents.filter((e) => !e.isAllDay)),
        };
      }),
    [days, events],
  );

  // "Aug 5, 9:00 AM – Aug 7, 6:00 PM" for the bar's tooltip. All-day events
  // carry an exclusive midnight end, so step back a second to name the last
  // day the user actually sees the event on.
  const spanRangeFmt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language || 'en', {
        month: 'short',
        day: 'numeric',
      }),
    [i18n.language],
  );
  const spanTooltip = (event: (typeof events)[number]) => {
    const lastMoment = event.isAllDay ? event.endTime - 1 : event.endTime;
    const range = `${spanRangeFmt.format(event.startTime * 1000)} – ${spanRangeFmt.format(lastMoment * 1000)}`;
    return event.isAllDay
      ? `${event.title} · ${range}`
      : `${event.title} · ${range} · ${time(event.startTime)} – ${time(event.endTime)}`;
  };

  return (
    <div className="flex flex-col flex-1 min-h-0">
      {/* Day headers, multi-day band, all-day row */}
      <div className="border-b border-gray-200 flex-shrink-0 dark:border-gray-700">
        {/* Day labels */}
        <div className="flex">
          <div className="w-14 flex-shrink-0 border-r border-gray-100 dark:border-gray-800" />
          {perDay.map(({ day }) => (
            <div key={day.getTime()} className="flex-1 min-w-0 border-r border-gray-100 px-1 py-1 dark:border-gray-800">
              <div
                className={`text-xs text-center font-semibold ${
                  day.getTime() === todayMs
                    ? 'text-primary-600 dark:text-primary-400'
                    : 'text-gray-600 dark:text-gray-400'
                }`}
              >
                {dayLabelFmt.format(day)}
              </div>
            </div>
          ))}
        </div>

        {/* Multi-day events: one continuous bar per event, spanning the days it
            covers, with a chevron where it runs past the visible range. */}
        {spans.length > 0 && (
          <div className="flex">
            <div className="w-14 flex-shrink-0 border-r border-gray-100 dark:border-gray-800" />
            <div className="flex-1 min-w-0 relative" style={{ height: laneCount * SPAN_LANE_PX }}>
              {spans.map(({ event, startIndex, endIndex, continuesBefore, continuesAfter, lane }) => (
                <button
                  key={event.id}
                  onClick={() => onSelectEvent(event)}
                  title={spanTooltip(event)}
                  className={`absolute text-left px-1.5 py-0.5 text-[11px] leading-tight truncate transition-opacity hover:opacity-80 flex items-center gap-1 ${
                    continuesBefore ? '' : 'rounded-l'
                  } ${continuesAfter ? '' : 'rounded-r'}`}
                  style={{
                    top: lane * SPAN_LANE_PX,
                    left: `calc(${(startIndex / days.length) * 100}% + 2px)`,
                    width: `calc(${((endIndex - startIndex + 1) / days.length) * 100}% - 4px)`,
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
          </div>
        )}

        {/* Single-day all-day events, per column */}
        <div className="flex">
          <div className="w-14 flex-shrink-0 border-r border-gray-100 dark:border-gray-800" />
          {perDay.map(({ day, allDay }) => (
            <div key={day.getTime()} className="flex-1 min-w-0 border-r border-gray-100 px-1 pb-1 dark:border-gray-800">
              <div className="space-y-0.5">
                {allDay.map((event) => (
                  <button
                    key={event.id}
                    onClick={() => onSelectEvent(event)}
                    title={`${event.title} · ${t('calendar:allDay')}`}
                    className="w-full text-left px-1.5 py-0.5 rounded text-[11px] leading-tight truncate transition-opacity hover:opacity-80"
                    style={eventChipStyle(colorFor(event.calendarId))}
                  >
                    {event.title || '—'}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Scrollable time grid */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        <div className="flex" style={{ height: 24 * HOUR_PX }}>
          {/* Hour gutter */}
          <div className="w-14 flex-shrink-0 border-r border-gray-100 relative dark:border-gray-800">
            {hourLabels.map((label, h) => (
              <div
                key={label}
                className="absolute right-1 text-[10px] text-gray-400 -translate-y-1/2 dark:text-gray-500"
                style={{ top: h * HOUR_PX }}
              >
                {h > 0 && label}
              </div>
            ))}
          </div>
          {perDay.map(({ day, timed }) => {
            const dayStartSec = Math.floor(day.getTime() / 1000);
            const dayEndSec = Math.floor(addDays(day, 1).getTime() / 1000);
            const minutesInDay = (dayEndSec - dayStartSec) / 60;
            return (
              <div
                key={day.getTime()}
                className="flex-1 min-w-0 border-r border-gray-100 relative dark:border-gray-800"
                onDoubleClick={(e) => {
                  // Event blocks own their double-clicks; empty column space
                  // (including the right gutter next to blocks) creates.
                  if ((e.target as HTMLElement).closest('button')) return;
                  const rect = e.currentTarget.getBoundingClientRect();
                  const slot = slotFromOffsetY(e.clientY - rect.top, day, HOUR_PX);
                  onCreateSlot(slot.start, slot.end);
                }}
              >
                {/* Hour lines */}
                {hourLabels.map((label, h) => (
                  <div
                    key={label}
                    className="absolute inset-x-0 border-t border-gray-100 dark:border-gray-800"
                    style={{ top: h * HOUR_PX }}
                  />
                ))}
                {timed.map(({ event, column, columns }) => {
                  // Clamp to the day column (multi-day / cross-midnight events).
                  const startMin = Math.max(0, (event.startTime - dayStartSec) / 60);
                  const endMin = Math.min(
                    minutesInDay,
                    (Math.max(event.endTime, event.startTime + 60) - dayStartSec) / 60,
                  );
                  const top = (startMin / MINUTES_PER_DAY) * 24 * HOUR_PX;
                  // Min-height keeps even the shortest event one text line tall
                  // and clickable; the text mode decides what fits inside.
                  const height = Math.max(((endMin - startMin) / MINUTES_PER_DAY) * 24 * HOUR_PX, EVENT_MIN_BLOCK_PX);
                  const textMode = eventTextMode(height);
                  const { leftPct, widthPct } = eventColumnGeometry(column, columns);
                  return (
                    <button
                      key={event.id}
                      onClick={() => onSelectEvent(event)}
                      onDoubleClick={(e) => e.stopPropagation()}
                      title={`${event.title} · ${time(event.startTime)} – ${time(event.endTime)}`}
                      className="absolute text-left rounded px-1.5 py-0.5 text-[11px] leading-tight overflow-hidden border text-gray-900 transition-opacity hover:opacity-80 flex flex-col items-stretch justify-start dark:text-gray-100"
                      style={{
                        top,
                        height,
                        left: `${leftPct}%`,
                        width: `calc(${widthPct}% - 2px)`,
                        ...eventBlockStyle(colorFor(event.calendarId), event.status === 'tentative'),
                      }}
                    >
                      {textMode === 'two-lines' && (
                        <>
                          <span className="font-semibold block truncate">{event.title || '—'}</span>
                          <span className="block truncate opacity-80">
                            {time(event.startTime)} – {time(event.endTime)}
                          </span>
                        </>
                      )}
                      {textMode === 'one-line' && (
                        <span className="block truncate">
                          <span className="font-semibold">{event.title || '—'}</span>
                          <span className="opacity-80"> · {time(event.startTime)}</span>
                        </span>
                      )}
                      {/* 'no-text': tooltip only — the block is too short for readable text. */}
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
