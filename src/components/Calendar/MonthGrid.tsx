import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import { eventsForDay, monthGrid, startOfDay } from '@/lib/calendarGrid';
import type { CalendarEvent } from '@/types';

/** Max event chips per day cell before collapsing into "+N more". */
const MAX_CHIPS = 3;

/** Default slot proposed when double-clicking a month cell: 09:00–10:00. */
const DEFAULT_CREATE_HOUR = 9;

interface MonthGridProps {
  anchor: Date;
  events: CalendarEvent[];
  onSelectEvent: (event: CalendarEvent) => void;
  /** "+N more" clicked → open that day in Day view. */
  onOpenDay: (day: Date) => void;
  /** Double-click on a day cell → propose a new event `[start, end)` (unix seconds). */
  onCreateSlot: (start: number, end: number) => void;
}

/** Classic 6×7 Google Calendar-style month grid (weeks start Monday). */
export function MonthGrid({ anchor, events, onSelectEvent, onOpenDay, onCreateSlot }: MonthGridProps) {
  const { t, i18n } = useTranslation(['calendar']);
  const { time } = useFormatters();
  const cells = useMemo(() => monthGrid(anchor), [anchor]);
  const todayMs = startOfDay(new Date()).getTime();

  const weekdayLabels = useMemo(() => {
    const fmt = new Intl.DateTimeFormat(i18n.language || 'en', { weekday: 'short' });
    return cells.slice(0, 7).map((c) => fmt.format(c.date));
  }, [cells, i18n.language]);

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="grid grid-cols-7 border-b border-gray-200 flex-shrink-0">
        {weekdayLabels.map((label) => (
          <div
            key={label}
            className="px-2 py-1 text-xs font-semibold text-gray-500 uppercase tracking-wider text-center"
          >
            {label}
          </div>
        ))}
      </div>
      <div className="grid grid-cols-7 grid-rows-6 flex-1 min-h-0">
        {cells.map((cell) => {
          const dayEvents = eventsForDay(events, cell.date);
          const visible = dayEvents.slice(0, MAX_CHIPS);
          const overflow = dayEvents.length - visible.length;
          const isToday = cell.date.getTime() === todayMs;
          return (
            <div
              key={cell.date.getTime()}
              className={`border-b border-r border-gray-100 p-1 min-h-0 overflow-hidden flex flex-col ${
                cell.inMonth ? 'bg-white' : 'bg-gray-50'
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
                        ? 'text-gray-700'
                        : 'text-gray-400'
                  }`}
                >
                  {cell.date.getDate()}
                </span>
              </div>
              <div className="space-y-0.5 min-h-0 overflow-hidden">
                {visible.map((event) => (
                  <button
                    key={event.id}
                    onClick={() => onSelectEvent(event)}
                    title={event.title}
                    className={`w-full text-left px-1.5 py-0.5 rounded text-[11px] leading-tight truncate transition-colors ${
                      event.isAllDay
                        ? 'bg-primary-600 text-white hover:bg-primary-700'
                        : event.status === 'tentative'
                          ? 'bg-primary-50 text-primary-700 border border-dashed border-primary-300 hover:bg-primary-100'
                          : 'bg-primary-100 text-primary-800 hover:bg-primary-200'
                    }`}
                  >
                    {!event.isAllDay && <span className="font-medium mr-1">{time(event.startTime)}</span>}
                    {event.title || '—'}
                  </button>
                ))}
                {overflow > 0 && (
                  <button
                    onClick={() => onOpenDay(cell.date)}
                    className="w-full text-left px-1.5 py-0.5 text-[11px] text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded"
                  >
                    {t('calendar:moreEvents', { n: overflow })}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
