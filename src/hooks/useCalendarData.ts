import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import * as api from '@/lib/api';
import {
  calendarColorMap,
  calendarKey,
  FALLBACK_CALENDAR_COLORS,
  hiddenCalendarIds,
  visibleEvents,
} from '@/lib/calendarColor';
import { errorText, isAuthError } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Calendar, CalendarEvent } from '@/types';

/** A calendar sync that failed for one account (not an auth failure — those
 *  land in `reauthAccountIds`). */
export interface CalendarSyncError {
  accountId: string;
  message: string;
}

/**
 * Events and calendar registry of the shown accounts over a visible range,
 * with the on-open / on-switch `sync_calendar_now` refresh. Shared by the full
 * calendar view and the day-agenda panel. One account normally; every
 * calendar account in "All accounts". Each account loads and syncs on its
 * own, so one failing account never hides the others' events. Every async
 * path is generation-guarded so a slow answer for a previous selection or
 * range never lands over the current one.
 */
export function useCalendarData(accountIds: readonly string[], rangeStart: number, rangeEnd: number) {
  const addLog = useLogStore((s) => s.addLog);
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [calendars, setCalendars] = useState<Calendar[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [syncErrors, setSyncErrors] = useState<CalendarSyncError[]>([]);
  const [reauthAccountIds, setReauthAccountIds] = useState<string[]>([]);
  const [reauthingAccountId, setReauthingAccountId] = useState<string | null>(null);

  // The selection by value: callers rebuild the id list every render.
  const selectionKey = accountIds.join('\n');
  const ids = useMemo(() => (selectionKey === '' ? [] : selectionKey.split('\n')), [selectionKey]);

  // Load events for the visible range. Guarded against stale responses.
  const fetchIdRef = useRef(0);
  const loadEvents = useCallback(
    async (forIds: readonly string[], start: number, end: number) => {
      const fetchId = ++fetchIdRef.current;
      setIsLoading(true);
      const results = await Promise.allSettled(forIds.map(async (id) => api.getCalendarEvents(id, start, end)));
      if (fetchIdRef.current !== fetchId) return;
      setEvents(results.flatMap((r) => (r.status === 'fulfilled' ? r.value : [])));
      setIsLoading(false);
      const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
      if (failed) {
        const msg = errorText(failed.reason);
        setError(msg);
        addLog('error', 'sync', `Failed to load calendar events: ${msg}`);
      }
    },
    [addLog],
  );

  useEffect(() => {
    if (ids.length === 0) {
      setEvents([]);
      return;
    }
    void loadEvents(ids, rangeStart, rangeEnd);
  }, [ids, rangeStart, rangeEnd, loadEvents]);

  // The accounts' calendar registries: colours and the show/hide toggles.
  // Reloaded after each sync, which is what refreshes colours and picks up
  // newly shared calendars.
  const calendarsIdRef = useRef(0);
  const loadCalendars = useCallback(
    async (forIds: readonly string[]) => {
      const loadId = ++calendarsIdRef.current;
      const results = await Promise.allSettled(forIds.map(async (id) => api.getCalendars(id)));
      if (calendarsIdRef.current !== loadId) return;
      setCalendars(results.flatMap((r) => (r.status === 'fulfilled' ? r.value : [])));
      for (const r of results) {
        // Non-fatal: without the registry every event falls back to a palette
        // colour and nothing is hidden, so the calendar still renders.
        if (r.status === 'rejected') addLog('error', 'sync', `Failed to load calendars: ${errorText(r.reason)}`);
      }
    },
    [addLog],
  );

  useEffect(() => {
    if (ids.length === 0) {
      setCalendars([]);
      return;
    }
    void loadCalendars(ids);
  }, [ids, loadCalendars]);

  const colorMap = useMemo(() => calendarColorMap(calendars), [calendars]);
  const colorFor = useCallback(
    (event: Pick<CalendarEvent, 'accountId' | 'calendarId'>) =>
      colorMap.get(calendarKey(event.accountId, event.calendarId)) ?? FALLBACK_CALENDAR_COLORS[0],
    [colorMap],
  );
  const hidden = useMemo(() => hiddenCalendarIds(calendars), [calendars]);
  const shownEvents = useMemo(() => visibleEvents(events, hidden), [events, hidden]);

  // Optimistic toggle: flip locally first so the grid re-filters instantly,
  // then persist. On failure reload the registry so the UI never keeps a
  // toggle the DB rejected.
  const toggleCalendar = useCallback(
    async (calendar: Calendar) => {
      const next = !calendar.isVisible;
      setCalendars((current) => current.map((c) => (c.id === calendar.id ? { ...c, isVisible: next } : c)));
      try {
        await api.setCalendarVisible(calendar.accountId, calendar.providerCalendarId, next);
      } catch (e) {
        addLog('error', 'sync', `Failed to update calendar visibility: ${errorText(e)}`);
        void loadCalendars(ids);
      }
    },
    [ids, addLog, loadCalendars],
  );

  // Keep the latest range in a ref so a finishing sync reloads what's visible now.
  const rangeRef = useRef({ rangeStart, rangeEnd });
  rangeRef.current = { rangeStart, rangeEnd };

  // Sync on open / selection switch, and via the manual Refresh button.
  // Generation-guarded: a sync that finishes after a newer one started (e.g.
  // the previous account's slow sync landing after an account switch) must not
  // reload its events over the current selection's, nor touch banner state.
  const syncIdRef = useRef(0);
  const syncAccounts = useCallback(
    async (forIds: readonly string[]) => {
      const syncId = ++syncIdRef.current;
      setIsSyncing(true);
      setError(null);
      setSyncErrors([]);
      setReauthAccountIds([]);
      const results = await Promise.allSettled(forIds.map(async (id) => api.syncCalendarNow(id)));
      const reauth: string[] = [];
      const failures: CalendarSyncError[] = [];
      results.forEach((r, i) => {
        if (r.status === 'fulfilled') {
          addLog('success', 'sync', `Calendar synced (${r.value} events)`);
          return;
        }
        const msg = errorText(r.reason);
        addLog('error', 'sync', `Calendar sync failed: ${msg}`);
        // Auth-class failures get the friendly banner + inline re-auth button;
        // the raw provider message only goes to the output panel.
        if (isAuthError(r.reason, msg)) reauth.push(forIds[i]);
        else failures.push({ accountId: forIds[i], message: msg });
      });
      if (syncIdRef.current !== syncId) return;
      setReauthAccountIds(reauth);
      setSyncErrors(failures);
      try {
        const { rangeStart: start, rangeEnd: end } = rangeRef.current;
        await loadEvents(forIds, start, end);
        await loadCalendars(forIds);
      } finally {
        if (syncIdRef.current === syncId) setIsSyncing(false);
      }
    },
    [addLog, loadEvents, loadCalendars],
  );

  const runSync = useCallback(() => syncAccounts(ids), [syncAccounts, ids]);

  // Inline re-auth from the banner: run the OAuth flow for that account, then
  // sync again so granted access is visible immediately.
  const runReauth = useCallback(
    async (accountId: string) => {
      setReauthingAccountId(accountId);
      try {
        await api.reauthenticateAccount(accountId);
        addLog('success', 'account', 'Account re-authenticated');
        await syncAccounts(ids);
      } catch (e) {
        const msg = errorText(e);
        setError(msg);
        addLog('error', 'account', `Re-authentication failed: ${msg}`);
      } finally {
        setReauthingAccountId(null);
      }
    },
    [ids, addLog, syncAccounts],
  );

  /** A dialog hit an auth failure for `accountId`: show its re-auth banner. */
  const markNeedsReauth = useCallback((accountId: string) => {
    setReauthAccountIds((current) => (current.includes(accountId) ? current : [...current, accountId]));
  }, []);
  const dismissReauth = useCallback(() => setReauthAccountIds([]), []);
  const dismissErrors = useCallback(() => {
    setError(null);
    setSyncErrors([]);
  }, []);

  useEffect(() => {
    if (ids.length === 0) return;
    void syncAccounts(ids);
  }, [ids, syncAccounts]);

  return {
    events,
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
    colorFor,
    shownEvents,
    toggleCalendar,
    runSync,
    runReauth,
  };
}
