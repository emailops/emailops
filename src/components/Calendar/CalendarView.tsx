import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { useResponsiveLayout } from '@/hooks/useResponsiveLayout';
import * as api from '@/lib/api';
import { calendarColorMap, FALLBACK_CALENDAR_COLORS, hiddenCalendarIds, visibleEvents } from '@/lib/calendarColor';
import { eventsAfterDelete } from '@/lib/calendarEvent';
import { addDays, monthGrid, resolveCalendarAccountId, startOfDay, weekDays } from '@/lib/calendarGrid';
import { errorText, isAuthError } from '@/lib/errors';
import {
  calendarDisabledCurrentAccount,
  calendarEnabledAccounts,
  useCalendarIntegrationStore,
} from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { Account, Calendar, CalendarEvent } from '@/types';
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
  const integrationLoaded = useCalendarIntegrationStore((s) => s.isLoaded);
  const setIntegrationEnabled = useCalendarIntegrationStore((s) => s.setEnabled);
  const calendarAccounts = useMemo(
    () => calendarEnabledAccounts(accounts, calendarIntegrationIds),
    [accounts, calendarIntegrationIds],
  );

  // The account the user is "in" (sidebar selection) when its calendar
  // integration is switched off — instead of silently falling back to another
  // account, offer to enable it right here. Gated on the pref load so the
  // banner never flashes before the enabled set is known.
  const disabledCurrentAccount = useMemo(
    () =>
      integrationLoaded ? calendarDisabledCurrentAccount(accounts, calendarIntegrationIds, defaultAccountId) : null,
    [integrationLoaded, accounts, calendarIntegrationIds, defaultAccountId],
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

  // The selector follows the account the user is "in": on open (and whenever
  // the sidebar account changes to one with an enabled calendar) it overrides
  // the persisted selection — a manual pick in the selector wins afterwards,
  // until the sidebar account changes again. Gated on both loads so the
  // override is never undone by the async pref read landing late.
  const appliedDefaultRef = useRef<string | null>(null);
  useEffect(() => {
    if (!prefLoaded || !integrationLoaded || !defaultAccountId) return;
    if (appliedDefaultRef.current === defaultAccountId) return;
    if (!calendarAccounts.some((a) => a.id === defaultAccountId)) return;
    appliedDefaultRef.current = defaultAccountId;
    setPersistedAccountId(defaultAccountId);
  }, [prefLoaded, integrationLoaded, defaultAccountId, calendarAccounts]);

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
  const { isStacked } = useResponsiveLayout();
  // Overflow menu holding the controls that do not fit a phone header row.
  const [isMenuOpen, setIsMenuOpen] = useState(false);
  const [viewMode, setViewMode] = useState<CalendarViewMode>('week');
  const [anchor, setAnchor] = useState<Date>(() => startOfDay(new Date()));
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [calendars, setCalendars] = useState<Calendar[]>([]);
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

  // The account's calendar registry: colours for the grids and the legend's
  // show/hide toggles. Reloaded after each sync, which is what refreshes
  // colours and picks up newly shared calendars.
  const loadCalendars = useCallback(
    async (accountId: string) => {
      try {
        setCalendars(await api.getCalendars(accountId));
      } catch (e) {
        // Non-fatal: without the registry every event falls back to a palette
        // colour and nothing is hidden, so the calendar still renders.
        addLog('error', 'sync', `Failed to load calendars: ${errorText(e)}`);
      }
    },
    [addLog],
  );

  useEffect(() => {
    if (!selectedAccountId) {
      setCalendars([]);
      return;
    }
    void loadCalendars(selectedAccountId);
  }, [selectedAccountId, loadCalendars]);

  const colorMap = useMemo(() => calendarColorMap(calendars), [calendars]);
  const colorFor = useCallback(
    (calendarId: string) => colorMap.get(calendarId) ?? FALLBACK_CALENDAR_COLORS[0],
    [colorMap],
  );
  const hidden = useMemo(() => hiddenCalendarIds(calendars), [calendars]);
  const shownEvents = useMemo(() => visibleEvents(events, hidden), [events, hidden]);

  // Optimistic toggle: flip locally first so the grid re-filters instantly,
  // then persist. On failure we reload the registry so the UI never keeps a
  // toggle the DB rejected.
  const toggleCalendar = useCallback(
    async (calendar: Calendar) => {
      if (!selectedAccountId) return;
      const next = !calendar.isVisible;
      setCalendars((current) =>
        current.map((c) => (c.providerCalendarId === calendar.providerCalendarId ? { ...c, isVisible: next } : c)),
      );
      try {
        await api.setCalendarVisible(selectedAccountId, calendar.providerCalendarId, next);
      } catch (e) {
        addLog('error', 'sync', `Failed to update calendar visibility: ${errorText(e)}`);
        void loadCalendars(selectedAccountId);
      }
    },
    [selectedAccountId, addLog, loadCalendars],
  );

  // Keep the latest range in a ref so a finishing sync reloads what's visible now.
  const rangeRef = useRef({ rangeStart, rangeEnd });
  rangeRef.current = { rangeStart, rangeEnd };

  // Sync on view open / account switch, and via the manual Refresh button.
  // Generation-guarded: a sync that finishes after a newer one started (e.g.
  // the previous account's slow sync landing after an account switch) must not
  // reload its events over the current account's, nor touch banner state.
  const syncIdRef = useRef(0);
  const runSync = useCallback(
    async (accountId: string) => {
      const syncId = ++syncIdRef.current;
      setIsSyncing(true);
      setError(null);
      setNeedsReauth(false);
      try {
        const stored = await api.syncCalendarNow(accountId);
        addLog('success', 'sync', `Calendar synced (${stored} events)`);
        if (syncIdRef.current !== syncId) return;
        const { rangeStart: start, rangeEnd: end } = rangeRef.current;
        await loadEvents(accountId, start, end);
        await loadCalendars(accountId);
      } catch (e) {
        const msg = errorText(e);
        addLog('error', 'sync', `Calendar sync failed: ${msg}`);
        if (syncIdRef.current !== syncId) return;
        // Auth-class failures get the friendly banner + inline re-auth button;
        // the raw provider message only goes to the output panel.
        if (isAuthError(e, msg)) {
          setNeedsReauth(true);
        } else {
          setError(msg);
        }
      } finally {
        if (syncIdRef.current === syncId) setIsSyncing(false);
      }
    },
    [addLog, loadEvents, loadCalendars],
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

  // Inline enable from the banner: flip the integration pref back on, then
  // select the account — the selection effect above triggers the first sync
  // (auth failures land in the existing re-auth banner).
  const [isEnabling, setIsEnabling] = useState(false);
  const enableCurrentCalendar = useCallback(async () => {
    if (!disabledCurrentAccount) return;
    setIsEnabling(true);
    setError(null);
    try {
      await setIntegrationEnabled(disabledCurrentAccount.id, true);
      addLog('info', 'sync', `Calendar integration enabled for ${disabledCurrentAccount.email} — syncing…`);
      handleSelectAccount(disabledCurrentAccount.id);
    } catch (e) {
      const msg = errorText(e);
      setError(msg);
      addLog('error', 'system', `Failed to enable calendar integration: ${msg}`);
    } finally {
      setIsEnabling(false);
    }
  }, [disabledCurrentAccount, setIntegrationEnabled, handleSelectAccount, addLog]);

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

  // Frosted full-view overlay for the account the user is "in" when its
  // calendar integration is off: the calendar behind renders but is blurred
  // and non-interactive ("frozen"), with the enable card centered both ways
  // (rendered in the empty state too, where it is the only way back without a
  // trip to Settings).
  const enableCalendarOverlay = disabledCurrentAccount && (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-white/60 backdrop-blur-sm dark:bg-surface/60">
      <div className="flex flex-col items-center gap-3 max-w-sm rounded-lg border border-gray-200 bg-white px-6 py-5 shadow-lg text-center dark:border-gray-700 dark:bg-surface">
        <p className="text-sm text-gray-800 break-words dark:text-gray-200">
          {t('calendar:integrationDisabled', { email: disabledCurrentAccount.email })}
        </p>
        <button
          onClick={() => void enableCurrentCalendar()}
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
  const errorBanner = error && (
    <div className="relative z-30 flex-shrink-0 border-b border-red-200 bg-red-50 px-4 py-2 flex items-start gap-2 text-sm text-red-800 dark:border-red-800 dark:bg-red-900/20 dark:text-red-300">
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
        className="flex-shrink-0 p-0.5 text-red-600 hover:text-red-800 rounded dark:text-red-400 dark:hover:text-red-300"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );

  // The three control groups that do not survive a phone-width header row.
  // Declared once and placed either inline (desktop) or inside the overflow
  // menu (stacked), so the two layouts can never drift apart in behaviour.
  const viewModeToggle = (
    <div className="flex rounded-md border border-gray-300 overflow-hidden dark:border-gray-600">
      {(['month', 'week', 'day'] as const).map((mode) => (
        <button
          key={mode}
          onClick={() => {
            setViewMode(mode);
            setIsMenuOpen(false);
          }}
          className={`flex-1 px-3 py-1.5 text-sm transition-colors ${
            viewMode === mode
              ? 'bg-primary-600 text-white'
              : 'text-gray-700 hover:bg-gray-50 dark:text-gray-300 dark:hover:bg-surface-raised'
          }`}
        >
          {t(`calendar:viewModes.${mode}` as const)}
        </button>
      ))}
    </div>
  );

  const refreshButton = (
    <button
      onClick={() => {
        setIsMenuOpen(false);
        if (selectedAccountId) void runSync(selectedAccountId);
      }}
      disabled={isSyncing || !selectedAccountId}
      className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors disabled:opacity-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-surface-raised"
    >
      {t('common:actions.refresh')}
    </button>
  );

  // Compact per-account selector — the calendar never offers "All accounts".
  const accountSelect = (
    <Select
      value={selectedAccountId ?? ''}
      onChange={(id) => {
        setIsMenuOpen(false);
        handleSelectAccount(id);
      }}
      options={calendarAccounts.map((a) => ({ value: a.id, label: a.email }))}
      ariaLabel={t('calendar:selectAccount')}
      placeholder={t('calendar:selectAccount')}
      align="right"
      variant="light"
    />
  );

  // Which calendar each colour means, and a click to show/hide it. Only worth
  // rendering when the account has more than one calendar — a single-calendar
  // account gains nothing from a one-item legend.
  const calendarToggles = calendars.length > 1 && (
    <div className="flex items-center gap-1.5 flex-wrap">
      {calendars.map((calendar) => (
        <button
          key={calendar.providerCalendarId}
          onClick={() => void toggleCalendar(calendar)}
          title={
            calendar.isVisible
              ? t('calendar:calendars.hideOne', { name: calendar.name })
              : t('calendar:calendars.showOne', { name: calendar.name })
          }
          aria-pressed={calendar.isVisible}
          className={`flex items-center gap-1.5 px-2 py-0.5 rounded-full border text-xs transition-colors ${
            calendar.isVisible
              ? 'border-gray-300 text-gray-700 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-surface-raised'
              : 'border-gray-200 text-gray-400 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-500 dark:hover:bg-surface-raised'
          }`}
        >
          <span
            className="w-2.5 h-2.5 rounded-full flex-shrink-0 border border-black/10"
            style={{
              backgroundColor: calendar.isVisible ? colorFor(calendar.providerCalendarId) : 'transparent',
              borderColor: colorFor(calendar.providerCalendarId),
            }}
          />
          <span className="max-w-[160px] truncate">{calendar.name || calendar.providerCalendarId}</span>
        </button>
      ))}
    </div>
  );

  if (calendarAccounts.length === 0) {
    return (
      <div className="relative flex flex-col flex-1 min-h-0 overflow-hidden bg-white dark:bg-surface">
        {errorBanner}
        <div className="flex flex-col flex-1 items-center justify-center text-sm text-gray-500 dark:text-gray-400">
          {t('calendar:noAccount')}
        </div>
        {enableCalendarOverlay}
      </div>
    );
  }

  return (
    <div className="relative flex flex-col flex-1 min-h-0 overflow-hidden bg-white dark:bg-surface">
      {/* Re-auth banner — friendly, actionable, no raw provider payload. */}
      {needsReauth && (
        <div className="flex-shrink-0 border-b border-amber-200 bg-amber-50 px-4 py-2 flex items-center gap-3 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-900/20 dark:text-amber-200">
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
            className="flex-shrink-0 p-0.5 text-amber-700 hover:text-amber-900 rounded dark:text-amber-300 dark:hover:text-amber-200"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}
      {errorBanner}

      {/* Header: navigation + range label + view toggle + account selector.
          When stacked, everything but date navigation moves behind the
          overflow menu — five inline controls wrapped onto three rows at phone
          width and pushed the grid itself off the first screen. */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-200 flex-shrink-0 flex-wrap dark:border-gray-700">
        {isStacked && (
          <div className="relative flex-shrink-0">
            <button
              type="button"
              onClick={() => setIsMenuOpen((open) => !open)}
              aria-label={t('calendar:moreOptions')}
              aria-expanded={isMenuOpen}
              className="flex h-9 w-9 items-center justify-center rounded-md border border-gray-300 text-gray-600 active:bg-gray-100 dark:border-gray-600 dark:text-gray-400 dark:active:bg-surface-hover"
            >
              <svg className="w-4 h-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={2}>
                <path d="M3 6h14M3 10h14M3 14h14" strokeLinecap="round" />
              </svg>
            </button>
            {isMenuOpen && (
              <>
                {/* Tap-anywhere-else to dismiss. Below the panel, above the grid. */}
                <button
                  type="button"
                  aria-label={t('common:actions.close')}
                  onClick={() => setIsMenuOpen(false)}
                  className="fixed inset-0 z-40 cursor-default"
                />
                <div className="absolute left-0 top-full z-50 mt-1 w-64 space-y-3 rounded-lg border border-gray-200 bg-white p-3 shadow-lg dark:border-gray-700 dark:bg-surface">
                  {viewModeToggle}
                  <div className="flex items-center gap-2">{refreshButton}</div>
                  {accountSelect}
                  {calendarToggles}
                </div>
              </>
            )}
          </div>
        )}
        <button
          onClick={goToday}
          className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors dark:border-gray-600 dark:text-gray-300 dark:hover:bg-surface-raised"
        >
          {t('calendar:today')}
        </button>
        <div className="flex items-center">
          <button
            onClick={() => navigate(-1)}
            title={t('calendar:previous')}
            className="p-1.5 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded transition-colors dark:text-gray-400 dark:hover:text-gray-300 dark:hover:bg-surface-hover"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
            </svg>
          </button>
          <button
            onClick={() => navigate(1)}
            title={t('calendar:next')}
            className="p-1.5 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded transition-colors dark:text-gray-400 dark:hover:text-gray-300 dark:hover:bg-surface-hover"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          </button>
        </div>
        <h2 className="text-base font-semibold text-gray-900 min-w-0 truncate dark:text-gray-100">{rangeLabel}</h2>
        {(isSyncing || isLoading) && (
          <span className="flex items-center gap-1.5 text-xs text-gray-400 dark:text-gray-500">
            <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
            </svg>
            {t('calendar:refreshing')}
          </span>
        )}
        <span className="flex-1" />
        {!isStacked && (
          <>
            {viewModeToggle}
            {refreshButton}
            <div className="max-w-[200px]">{accountSelect}</div>
          </>
        )}
      </div>

      {/* Calendar legend. Hidden when stacked: it is a whole row of chips above
          the grid, and the same toggles live in the overflow menu. */}
      {!isStacked && calendarToggles && (
        <div className="px-4 py-1.5 border-b border-gray-200 flex-shrink-0 dark:border-gray-700">{calendarToggles}</div>
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
