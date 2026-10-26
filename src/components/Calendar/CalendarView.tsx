import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { useCalendarAccount } from '@/hooks/useCalendarAccount';
import { useCalendarData } from '@/hooks/useCalendarData';
import {
  type CreateAccountChoice,
  calendarAccountOptions,
  calendarLegendGroups,
  calendarSelectionIds,
  createAccountChoices,
  eventColorFor,
} from '@/lib/calendarAccounts';
import { eventsAfterDelete } from '@/lib/calendarEvent';
import { isReadOnlyCalendar } from '@/lib/calendarFeeds';
import { addDays, monthGrid, startOfDay, weekDays } from '@/lib/calendarGrid';
import { accountColorHex } from '@/lib/colors';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Account, Calendar, CalendarEvent } from '@/types';
import { EventDetailDialog } from './EventDetailDialog';
import { MonthGrid } from './MonthGrid';
import { NewEventDialog } from './NewEventDialog';
import { TimeGrid } from './TimeGrid';

type CalendarViewMode = 'month' | 'week' | 'day';

function toSec(d: Date): number {
  return Math.floor(d.getTime() / 1000);
}

interface CalendarViewProps {
  accounts: Account[];
  /** Concrete fallback account (effective account — never the unified sentinel). */
  defaultAccountId: string | null;
}

/**
 * Calendar screen: one account, or "All accounts" — every calendar account's
 * events on one grid, each in its account's colour (docs/DECISIONS.md).
 * Month / Week / Day views over `get_calendar_events`, with an on-open /
 * on-switch `sync_calendar_now` refresh.
 */
export function CalendarView({ accounts, defaultAccountId }: CalendarViewProps) {
  const { t, i18n } = useTranslation(['calendar', 'common']);
  const addLog = useLogStore((s) => s.addLog);

  const {
    calendarAccounts,
    disabledCurrentAccount,
    selectedAccountId,
    isAllAccounts,
    handleSelectAccount,
    enableCurrentCalendar,
    isEnabling,
  } = useCalendarAccount(accounts, defaultAccountId);
  const shownAccountIds = calendarSelectionIds(selectedAccountId, calendarAccounts);
  const accountById = useMemo(() => new Map(accounts.map((a) => [a.id, a])), [accounts]);

  // View state
  const [viewMode, setViewMode] = useState<CalendarViewMode>('week');
  const [anchor, setAnchor] = useState<Date>(() => startOfDay(new Date()));
  const [selectedEvent, setSelectedEvent] = useState<CalendarEvent | null>(null);
  /** Proposed slot for the "New event" dialog (double-clicked grid position)
   *  and the accounts it can go to. */
  const [createSlot, setCreateSlot] = useState<{ start: number; end: number; choices: CreateAccountChoice[] } | null>(
    null,
  );

  // Visible range for the current view/anchor, in unix seconds.
  const { rangeStart, rangeEnd } = useMemo(() => {
    if (viewMode === 'month') {
      const cells = monthGrid(anchor);
      return { rangeStart: toSec(cells[0].date), rangeEnd: toSec(addDays(cells[41].date, 1)) };
    }
    if (viewMode === 'week') {
      const days = weekDays(anchor);
      return { rangeStart: toSec(days[0]), rangeEnd: toSec(addDays(days[6], 1)) };
    }
    const dayStart = startOfDay(anchor);
    return { rangeStart: toSec(dayStart), rangeEnd: toSec(addDays(dayStart, 1)) };
  }, [viewMode, anchor]);

  const {
    setEvents,
    calendars,
    isLoading,
    isSyncing,
    error,
    setError,
    syncErrors,
    dismissErrors,
    reauthAccountIds,
    markNeedsReauth,
    dismissReauth,
    reauthingAccountId,
    colorFor: calendarColorFor,
    shownEvents,
    toggleCalendar,
    runSync,
    runReauth,
  } = useCalendarData(shownAccountIds, rangeStart, rangeEnd);

  const colorFor = useMemo(
    () => eventColorFor(isAllAccounts, calendarAccounts, calendarColorFor),
    [isAllAccounts, calendarAccounts, calendarColorFor],
  );
  // A legend swatch is painted like the calendar's events.
  const legendColor = (calendar: Calendar) =>
    colorFor({ accountId: calendar.accountId, calendarId: calendar.providerCalendarId } as CalendarEvent);
  const shownAccounts = useMemo(
    () => calendarAccounts.filter((a) => shownAccountIds.includes(a.id)),
    [calendarAccounts, shownAccountIds],
  );

  const enableCalendar = useCallback(() => {
    setError(null);
    enableCurrentCalendar().catch((e) => setError(errorText(e)));
  }, [enableCurrentCalendar, setError]);

  // Navigation
  const goToday = useCallback(() => setAnchor(startOfDay(new Date())), []);
  const navigate = useCallback(
    (direction: 1 | -1) => {
      setAnchor((prev) => {
        if (viewMode === 'month') return new Date(prev.getFullYear(), prev.getMonth() + direction, 1);
        return addDays(prev, direction * (viewMode === 'week' ? 7 : 1));
      });
    },
    [viewMode],
  );

  const openDay = useCallback((day: Date) => {
    setAnchor(startOfDay(day));
    setViewMode('day');
  }, []);

  // Current-range label
  const rangeLabel = useMemo(() => {
    const locale = i18n.language || 'en';
    if (viewMode === 'month') {
      return new Intl.DateTimeFormat(locale, { month: 'long', year: 'numeric' }).format(anchor);
    }
    if (viewMode === 'week') {
      const days = weekDays(anchor);
      const fmt = new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' });
      const year = new Intl.DateTimeFormat(locale, { year: 'numeric' }).format(days[6]);
      return `${fmt.format(days[0])} – ${fmt.format(days[6])}, ${year}`;
    }
    return new Intl.DateTimeFormat(locale, { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' }).format(
      anchor,
    );
  }, [viewMode, anchor, i18n.language]);

  const days = useMemo(() => (viewMode === 'week' ? weekDays(anchor) : [startOfDay(anchor)]), [viewMode, anchor]);

  // A double-click on an empty slot: the in-app dialog for the accounts that
  // can take the event (asking which in "All accounts"). A lone account whose
  // calendars are Proton feeds (read only — Proton has no write API) hands the
  // day straight over to Proton Calendar's own create form in the browser;
  // nothing for other read-only feeds.
  const openCreateSlot = useCallback(
    (start: number, end: number) => {
      const choices = createAccountChoices(shownAccounts, calendars, start);
      const only = choices.length === 1 ? choices[0] : null;
      if (only?.plan.kind === 'proton') {
        addLog('info', 'system', t('calendar:createInProton'));
        openExternal(only.plan.url).catch((err) => {
          addLog('error', 'system', `Failed to open Proton Calendar: ${err}`);
        });
      } else if (choices.length > 0) {
        setCreateSlot({ start, end, choices });
      }
    },
    [shownAccounts, calendars, addLog, t],
  );

  // Frosted full-view overlay for the account the user is "in" when its
  // calendar integration is off: the calendar behind renders but is blurred
  // and non-interactive ("frozen"), with the enable card centered both ways
  // (rendered in the empty state too, where it is the only way back without a
  // trip to Settings).
  const enableCalendarOverlay = disabledCurrentAccount && (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-white/60 backdrop-blur-sm">
      <div className="flex flex-col items-center gap-3 max-w-sm rounded-lg border border-gray-200 bg-white px-6 py-5 shadow-lg text-center">
        <p className="text-sm text-gray-800 break-words">
          {t('calendar:integrationDisabled', { email: disabledCurrentAccount.email })}
        </p>
        <button
          onClick={enableCalendar}
          disabled={isEnabling}
          className="px-4 py-1.5 rounded-md bg-primary-600 text-white text-sm font-medium hover:bg-primary-700 disabled:opacity-60"
        >
          {t('calendar:enableCalendarButton')}
        </button>
      </div>
    </div>
  );

  // Error banner — pinned at the top of the view, above all scrollable content
  // (z-30 keeps it readable above the frosted overlay, e.g. when enabling fails).
  // A sync failure names its account in "All accounts".
  const errorMessages = [
    ...(error ? [error] : []),
    ...syncErrors.map((e) =>
      isAllAccounts ? `${accountById.get(e.accountId)?.email ?? e.accountId}: ${e.message}` : e.message,
    ),
  ];
  const errorBanner = errorMessages.length > 0 && (
    <div className="relative z-30 flex-shrink-0 border-b border-red-200 bg-red-50 px-4 py-2 flex items-start gap-2 text-sm text-red-800">
      <svg className="w-4 h-4 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
        <path
          fillRule="evenodd"
          d="M10 18a8 8 0 100-16 8 8 0 000 16zM9 9a1 1 0 012 0v4a1 1 0 11-2 0V9zm1-5a1 1 0 100 2 1 1 0 000-2z"
          clipRule="evenodd"
        />
      </svg>
      <div className="min-w-0 flex-1">
        {errorMessages.map((message) => (
          <p key={message} className="break-words">
            {t('calendar:syncError', { message })}
          </p>
        ))}
      </div>
      <button
        onClick={dismissErrors}
        title={t('common:actions.dismiss')}
        className="flex-shrink-0 p-0.5 text-red-600 hover:text-red-800 rounded"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );

  if (calendarAccounts.length === 0) {
    return (
      <div className="relative flex flex-col flex-1 min-h-0 overflow-hidden bg-white">
        {errorBanner}
        <div className="flex flex-col flex-1 items-center justify-center text-sm text-gray-500">
          {t('calendar:noAccount')}
        </div>
        {enableCalendarOverlay}
      </div>
    );
  }

  return (
    <div className="relative flex flex-col flex-1 min-h-0 overflow-hidden bg-white">
      {/* Re-auth banner — friendly, actionable, no raw provider payload. One
          row per account that needs it ("All accounts" names the account). */}
      {reauthAccountIds.map((reauthId) => (
        <div
          key={reauthId}
          className="flex-shrink-0 border-b border-amber-200 bg-amber-50 px-4 py-2 flex items-center gap-3 text-sm text-amber-900"
        >
          <svg className="w-4 h-4 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M18 8a6 6 0 01-7.743 5.743L10 14l-1 1-1 1H6v2H2v-4l4.257-4.257A6 6 0 1118 8zm-6-4a1 1 0 100 2 2 2 0 012 2 1 1 0 102 0 4 4 0 00-4-4z"
              clipRule="evenodd"
            />
          </svg>
          <span className="min-w-0 flex-1 break-words">
            {isAllAccounts
              ? t('calendar:reauthNeededAccount', { email: accountById.get(reauthId)?.email ?? reauthId })
              : t('calendar:reauthNeeded')}
          </span>
          <button
            onClick={() => void runReauth(reauthId)}
            disabled={reauthingAccountId !== null}
            className="flex-shrink-0 px-3 py-1 rounded-md bg-amber-600 text-white text-xs font-medium hover:bg-amber-700 disabled:opacity-60"
          >
            {reauthingAccountId === reauthId ? t('calendar:reauthInProgress') : t('calendar:reauthButton')}
          </button>
          <button
            onClick={dismissReauth}
            title={t('common:actions.dismiss')}
            className="flex-shrink-0 p-0.5 text-amber-700 hover:text-amber-900 rounded"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      ))}
      {errorBanner}

      {/* Header: navigation + range label + view toggle + account selector */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-200 flex-shrink-0 flex-wrap">
        <button
          onClick={goToday}
          className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors"
        >
          {t('calendar:today')}
        </button>
        <div className="flex items-center">
          <button
            onClick={() => navigate(-1)}
            title={t('calendar:previous')}
            className="p-1.5 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded transition-colors"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
            </svg>
          </button>
          <button
            onClick={() => navigate(1)}
            title={t('calendar:next')}
            className="p-1.5 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded transition-colors"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          </button>
        </div>
        <h2 className="text-base font-semibold text-gray-900 min-w-0 truncate">{rangeLabel}</h2>
        {(isSyncing || isLoading) && (
          <span className="flex items-center gap-1.5 text-xs text-gray-400">
            <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
            </svg>
            {t('calendar:refreshing')}
          </span>
        )}
        <span className="flex-1" />
        {/* Month / Week / Day toggle */}
        <div className="flex rounded-md border border-gray-300 overflow-hidden">
          {(['month', 'week', 'day'] as const).map((mode) => (
            <button
              key={mode}
              onClick={() => setViewMode(mode)}
              className={`px-3 py-1.5 text-sm transition-colors ${
                viewMode === mode ? 'bg-primary-600 text-white' : 'text-gray-700 hover:bg-gray-50'
              }`}
            >
              {t(`calendar:viewModes.${mode}` as const)}
            </button>
          ))}
        </div>
        <button
          onClick={() => void runSync()}
          disabled={isSyncing || shownAccountIds.length === 0}
          className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors disabled:opacity-50"
        >
          {t('common:actions.refresh')}
        </button>
        {/* Compact account selector: "All accounts" first once two have a calendar. */}
        <div className="max-w-[200px]">
          <Select
            value={selectedAccountId ?? ''}
            onChange={handleSelectAccount}
            options={calendarAccountOptions(calendarAccounts, t('calendar:allAccounts'))}
            ariaLabel={t('calendar:selectAccount')}
            placeholder={t('calendar:selectAccount')}
            align="right"
          />
        </div>
      </div>

      {/* Calendar legend: which calendar each colour means, and a click to
          show/hide it. Only shown with more than one calendar — a
          single-calendar account gains nothing from a one-item legend. In
          "All accounts" the calendars are grouped under their account. */}
      {calendars.length > 1 && (
        <div className="flex items-center gap-1.5 px-4 py-1.5 border-b border-gray-200 flex-shrink-0 flex-wrap">
          {isAllAccounts
            ? calendarLegendGroups(calendars, calendarAccounts).map((group) => (
                <div key={group.account.id} className="flex items-center gap-1.5 flex-wrap mr-2">
                  <span className="flex items-center gap-1 text-xs font-medium text-gray-500">
                    <span
                      className="w-2 h-2 rounded-full flex-shrink-0"
                      style={{ backgroundColor: accountColorHex(group.account.color) }}
                    />
                    <span className="max-w-[180px] truncate">{group.account.email}</span>
                  </span>
                  {group.calendars.map((calendar) => (
                    <LegendChip
                      key={calendar.id}
                      calendar={calendar}
                      color={legendColor(calendar)}
                      onToggle={toggleCalendar}
                    />
                  ))}
                </div>
              ))
            : calendars.map((calendar) => (
                <LegendChip
                  key={calendar.id}
                  calendar={calendar}
                  color={legendColor(calendar)}
                  onToggle={toggleCalendar}
                />
              ))}
        </div>
      )}

      {/* Grid */}
      {viewMode === 'month' ? (
        <MonthGrid
          anchor={anchor}
          events={shownEvents}
          colorFor={colorFor}
          onSelectEvent={setSelectedEvent}
          onOpenDay={openDay}
          onCreateSlot={openCreateSlot}
        />
      ) : (
        <TimeGrid
          days={days}
          events={shownEvents}
          colorFor={colorFor}
          onSelectEvent={setSelectedEvent}
          onCreateSlot={openCreateSlot}
        />
      )}

      {enableCalendarOverlay}

      {selectedEvent && (
        <EventDetailDialog
          event={selectedEvent}
          accountId={selectedEvent.accountId}
          accountEmail={isAllAccounts ? accountById.get(selectedEvent.accountId)?.email : undefined}
          provider={accountById.get(selectedEvent.accountId)?.provider ?? ''}
          readOnly={calendars.some(
            (c) =>
              c.accountId === selectedEvent.accountId &&
              c.providerCalendarId === selectedEvent.calendarId &&
              isReadOnlyCalendar(c),
          )}
          onClose={() => setSelectedEvent(null)}
          onDeleted={(eventId, scope, recurringEventId, startTime) => {
            // Provider round-trip already happened — drop the affected
            // occurrence(s) from every view per the chosen scope.
            setEvents((prev) => eventsAfterDelete(prev, { id: eventId, recurringEventId, startTime }, scope));
            setSelectedEvent(null);
          }}
          onAuthError={() => {
            setSelectedEvent(null);
            markNeedsReauth(selectedEvent.accountId);
          }}
        />
      )}

      {createSlot && (
        <NewEventDialog
          choices={createSlot.choices}
          initialStart={createSlot.start}
          initialEnd={createSlot.end}
          onClose={() => setCreateSlot(null)}
          onCreated={(created, recurrence) => {
            setCreateSlot(null);
            if (recurrence !== 'none') {
              // The provider stored the recurrence *master*; syncing expands it
              // into per-occurrence instances, then reloads the visible range.
              void runSync();
            } else {
              // Provider round-trip already happened — show it immediately, no reload.
              setEvents((prev) => [...prev.filter((e) => e.id !== created.id), created]);
            }
          }}
          onAuthError={(accountId) => {
            setCreateSlot(null);
            markNeedsReauth(accountId);
          }}
        />
      )}
    </div>
  );
}

/** One calendar in the legend: its colour swatch and name; a click shows or
 *  hides its events. */
function LegendChip({
  calendar,
  color,
  onToggle,
}: {
  calendar: Calendar;
  color: string;
  onToggle: (calendar: Calendar) => Promise<void>;
}) {
  const { t } = useTranslation(['calendar']);
  return (
    <button
      onClick={() => void onToggle(calendar)}
      title={
        calendar.isVisible
          ? t('calendar:calendars.hideOne', { name: calendar.name })
          : t('calendar:calendars.showOne', { name: calendar.name })
      }
      aria-pressed={calendar.isVisible}
      className={`flex items-center gap-1.5 px-2 py-0.5 rounded-full border text-xs transition-colors ${
        calendar.isVisible
          ? 'border-gray-300 text-gray-700 hover:bg-gray-50'
          : 'border-gray-200 text-gray-400 hover:bg-gray-50'
      }`}
    >
      <span
        className="w-2.5 h-2.5 rounded-full flex-shrink-0 border border-black/10"
        style={{
          backgroundColor: calendar.isVisible ? color : 'transparent',
          borderColor: color,
        }}
      />
      <span className="max-w-[160px] truncate">{calendar.name || calendar.providerCalendarId}</span>
    </button>
  );
}
