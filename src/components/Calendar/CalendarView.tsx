import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { eventsAfterDelete } from '@/lib/calendarEvent';
import { addDays, monthGrid, resolveCalendarAccountId, startOfDay, weekDays } from '@/lib/calendarGrid';
import { errorText, isAuthError } from '@/lib/errors';
import { calendarEnabledAccounts, useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { Account, CalendarEvent } from '@/types';
import { EventDetailDialog } from './EventDetailDialog';
import { MonthGrid } from './MonthGrid';
import { NewEventDialog } from './NewEventDialog';
import { TimeGrid } from './TimeGrid';

type CalendarViewMode = 'month' | 'week' | 'day';

/** Pref key for the calendar's own persisted account selection. */
const ACCOUNT_PREF_KEY = 'calendar_selected_account';

function toSec(d: Date): number {
  return Math.floor(d.getTime() / 1000);
}

interface CalendarViewProps {
  accounts: Account[];
  /** Concrete fallback account (effective account — never the unified sentinel). */
  defaultAccountId: string | null;
}

/**
 * Per-account calendar screen (docs/DECISIONS.md: deliberately no unified
 * "All accounts" option). Month / Week / Day views over `get_calendar_events`,
 * with an on-open / on-switch `sync_calendar_now` refresh.
 */
export function CalendarView({ accounts, defaultAccountId }: CalendarViewProps) {
  const { t, i18n } = useTranslation(['calendar', 'common']);
  const addLog = useLogStore((s) => s.addLog);

  // Only offer accounts whose calendar integration the user enabled in
  // Settings → Calendar (IMAP has no calendar support backend-side).
  const calendarIntegrationIds = useCalendarIntegrationStore((s) => s.enabledIds);
  const calendarAccounts = useMemo(
    () => calendarEnabledAccounts(accounts, calendarIntegrationIds),
    [accounts, calendarIntegrationIds],
  );

  // Persisted account selection: load once, then resolve against the enabled
  // account set (falling back to the effective account when the stored one is
  // gone or disabled).
  const [persistedAccountId, setPersistedAccountId] = useState<string | null>(null);
  const [prefLoaded, setPrefLoaded] = useState(false);
  useEffect(() => {
    let cancelled = false;
    api
      .getPref(ACCOUNT_PREF_KEY)
      .then((raw) => {
        if (cancelled) return;
        setPersistedAccountId(raw);
        setPrefLoaded(true);
      })
      .catch((err) => {
        if (cancelled) return;
        addLog('error', 'system', `Failed to load calendar account pref: ${err}`);
        setPrefLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [addLog]);

  const selectedAccountId = useMemo(
    () => (prefLoaded ? resolveCalendarAccountId(calendarAccounts, persistedAccountId, defaultAccountId) : null),
    [prefLoaded, calendarAccounts, persistedAccountId, defaultAccountId],
  );

  const handleSelectAccount = useCallback(
    (id: string) => {
      setPersistedAccountId(id);
      api.setPref(ACCOUNT_PREF_KEY, id).catch((err) => {
        addLog('error', 'system', `Failed to save calendar account pref: ${err}`);
      });
    },
    [addLog],
  );

  // View state
  const [viewMode, setViewMode] = useState<CalendarViewMode>('week');
  const [anchor, setAnchor] = useState<Date>(() => startOfDay(new Date()));
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [needsReauth, setNeedsReauth] = useState(false);
  const [isReauthing, setIsReauthing] = useState(false);
  const [selectedEvent, setSelectedEvent] = useState<CalendarEvent | null>(null);
  /** Proposed slot for the "New event" dialog (double-clicked grid position). */
  const [createSlot, setCreateSlot] = useState<{ start: number; end: number } | null>(null);

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

  // Load events for the visible range. Guarded against stale responses.
  const fetchIdRef = useRef(0);
  const loadEvents = useCallback(
    async (accountId: string, start: number, end: number) => {
      const fetchId = ++fetchIdRef.current;
      setIsLoading(true);
      try {
        const result = await api.getCalendarEvents(accountId, start, end);
        if (fetchIdRef.current !== fetchId) return;
        setEvents(result);
        setIsLoading(false);
      } catch (e) {
        if (fetchIdRef.current !== fetchId) return;
        setIsLoading(false);
        const msg = errorText(e);
        setError(msg);
        addLog('error', 'sync', `Failed to load calendar events: ${msg}`);
      }
    },
    [addLog],
  );

  useEffect(() => {
    if (!selectedAccountId) {
      setEvents([]);
      return;
    }
    void loadEvents(selectedAccountId, rangeStart, rangeEnd);
  }, [selectedAccountId, rangeStart, rangeEnd, loadEvents]);

  // Keep the latest range in a ref so a finishing sync reloads what's visible now.
  const rangeRef = useRef({ rangeStart, rangeEnd });
  rangeRef.current = { rangeStart, rangeEnd };

  // Sync on view open / account switch, and via the manual Refresh button.
  const runSync = useCallback(
    async (accountId: string) => {
      setIsSyncing(true);
      setError(null);
      setNeedsReauth(false);
      try {
        const stored = await api.syncCalendarNow(accountId);
        addLog('success', 'sync', `Calendar synced (${stored} events)`);
        const { rangeStart: start, rangeEnd: end } = rangeRef.current;
        await loadEvents(accountId, start, end);
      } catch (e) {
        const msg = errorText(e);
        // Auth-class failures get the friendly banner + inline re-auth button;
        // the raw provider message only goes to the output panel.
        if (isAuthError(e, msg)) {
          setNeedsReauth(true);
        } else {
          setError(msg);
        }
        addLog('error', 'sync', `Calendar sync failed: ${msg}`);
      } finally {
        setIsSyncing(false);
      }
    },
    [addLog, loadEvents],
  );

  // Inline re-auth from the banner: run the OAuth flow for the selected
  // account, then sync again so granted access is visible immediately.
  const runReauth = useCallback(async () => {
    if (!selectedAccountId) return;
    setIsReauthing(true);
    try {
      await api.reauthenticateAccount(selectedAccountId);
      addLog('success', 'account', 'Account re-authenticated');
      setNeedsReauth(false);
      await runSync(selectedAccountId);
    } catch (e) {
      const msg = errorText(e);
      setError(msg);
      addLog('error', 'account', `Re-authentication failed: ${msg}`);
    } finally {
      setIsReauthing(false);
    }
  }, [selectedAccountId, addLog, runSync]);

  useEffect(() => {
    if (!selectedAccountId) return;
    void runSync(selectedAccountId);
  }, [selectedAccountId, runSync]);

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

  const openCreateSlot = useCallback((start: number, end: number) => {
    setCreateSlot({ start, end });
  }, []);

  const selectedAccount = useMemo(
    () => calendarAccounts.find((a) => a.id === selectedAccountId) ?? null,
    [calendarAccounts, selectedAccountId],
  );

  if (calendarAccounts.length === 0) {
    return (
      <div className="flex flex-col flex-1 items-center justify-center text-sm text-gray-500 bg-white">
        {t('calendar:noAccount')}
      </div>
    );
  }

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden bg-white">
      {/* Re-auth banner — friendly, actionable, no raw provider payload. */}
      {needsReauth && (
        <div className="flex-shrink-0 border-b border-amber-200 bg-amber-50 px-4 py-2 flex items-center gap-3 text-sm text-amber-900">
          <svg className="w-4 h-4 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M18 8a6 6 0 01-7.743 5.743L10 14l-1 1-1 1H6v2H2v-4l4.257-4.257A6 6 0 1118 8zm-6-4a1 1 0 100 2 2 2 0 012 2 1 1 0 102 0 4 4 0 00-4-4z"
              clipRule="evenodd"
            />
          </svg>
          <span className="min-w-0 flex-1 break-words">{t('calendar:reauthNeeded')}</span>
          <button
            onClick={() => void runReauth()}
            disabled={isReauthing}
            className="flex-shrink-0 px-3 py-1 rounded-md bg-amber-600 text-white text-xs font-medium hover:bg-amber-700 disabled:opacity-60"
          >
            {isReauthing ? t('calendar:reauthInProgress') : t('calendar:reauthButton')}
          </button>
          <button
            onClick={() => setNeedsReauth(false)}
            title={t('common:actions.dismiss')}
            className="flex-shrink-0 p-0.5 text-amber-700 hover:text-amber-900 rounded"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}
      {/* Error banner — pinned at the top of the view, above all scrollable content. */}
      {error && (
        <div className="flex-shrink-0 border-b border-red-200 bg-red-50 px-4 py-2 flex items-start gap-2 text-sm text-red-800">
          <svg className="w-4 h-4 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M10 18a8 8 0 100-16 8 8 0 000 16zM9 9a1 1 0 012 0v4a1 1 0 11-2 0V9zm1-5a1 1 0 100 2 1 1 0 000-2z"
              clipRule="evenodd"
            />
          </svg>
          <div className="min-w-0 flex-1">
            <span className="break-words">{t('calendar:syncError', { message: error })}</span>
          </div>
          <button
            onClick={() => {
              setError(null);
            }}
            title={t('common:actions.dismiss')}
            className="flex-shrink-0 p-0.5 text-red-600 hover:text-red-800 rounded"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}

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
          onClick={() => selectedAccountId && runSync(selectedAccountId)}
          disabled={isSyncing || !selectedAccountId}
          className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors disabled:opacity-50"
        >
          {t('common:actions.refresh')}
        </button>
        {/* Compact per-account selector — the calendar never offers "All accounts". */}
        <select
          value={selectedAccountId ?? ''}
          onChange={(e) => handleSelectAccount(e.target.value)}
          title={t('calendar:selectAccount')}
          className="max-w-[200px] border border-gray-300 rounded-md px-2 py-1.5 text-sm text-gray-700 bg-white focus:outline-none focus:ring-1 focus:ring-primary-500"
        >
          {calendarAccounts.map((a) => (
            <option key={a.id} value={a.id}>
              {a.email}
            </option>
          ))}
        </select>
      </div>

      {/* Grid */}
      {viewMode === 'month' ? (
        <MonthGrid
          anchor={anchor}
          events={events}
          onSelectEvent={setSelectedEvent}
          onOpenDay={openDay}
          onCreateSlot={openCreateSlot}
        />
      ) : (
        <TimeGrid days={days} events={events} onSelectEvent={setSelectedEvent} onCreateSlot={openCreateSlot} />
      )}

      {selectedEvent && (
        <EventDetailDialog
          event={selectedEvent}
          accountId={selectedEvent.accountId}
          provider={selectedAccount?.provider ?? ''}
          onClose={() => setSelectedEvent(null)}
          onDeleted={(eventId, scope, recurringEventId, startTime) => {
            // Provider round-trip already happened — drop the affected
            // occurrence(s) from every view per the chosen scope.
            setEvents((prev) => eventsAfterDelete(prev, { id: eventId, recurringEventId, startTime }, scope));
            setSelectedEvent(null);
          }}
          onAuthError={() => {
            setSelectedEvent(null);
            setNeedsReauth(true);
          }}
        />
      )}

      {createSlot && selectedAccountId && (
        <NewEventDialog
          accountId={selectedAccountId}
          isGmail={selectedAccount?.provider === 'gmail'}
          initialStart={createSlot.start}
          initialEnd={createSlot.end}
          onClose={() => setCreateSlot(null)}
          onCreated={(created, recurrence) => {
            setCreateSlot(null);
            if (recurrence !== 'none') {
              // The provider stored the recurrence *master*; syncing expands it
              // into per-occurrence instances, then reloads the visible range.
              void runSync(selectedAccountId);
            } else {
              // Provider round-trip already happened — show it immediately, no reload.
              setEvents((prev) => [...prev.filter((e) => e.id !== created.id), created]);
            }
          }}
          onAuthError={() => {
            setCreateSlot(null);
            setNeedsReauth(true);
          }}
        />
      )}
    </div>
  );
}
