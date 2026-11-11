import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { useCalendarAccount } from '@/hooks/useCalendarAccount';
import { useCalendarData } from '@/hooks/useCalendarData';
import {
  type CreateAccountChoice,
  calendarAccountOptions,
  calendarSelectionIds,
  createAccountChoices,
  eventColorFor,
} from '@/lib/calendarAccounts';
import { eventsAfterDelete } from '@/lib/calendarEvent';
import { isReadOnlyCalendar } from '@/lib/calendarFeeds';
import { addDays, startOfDay } from '@/lib/calendarGrid';
import { errorText } from '@/lib/errors';
import { calendarPanelState, useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { Account, CalendarEvent } from '@/types';
import { EventDetailDialog } from './EventDetailDialog';
import { NewEventDialog } from './NewEventDialog';
import { TimeGrid } from './TimeGrid';

interface CalendarPanelProps {
  accounts: Account[];
  /** The account the user is "in" (effective account — never the unified sentinel). */
  defaultAccountId: string | null;
  onClose: () => void;
  /** Switch to the full calendar view (month / week / day grids). */
  onOpenFullCalendar: () => void;
  /** Open Settings → Calendar (to subscribe an IMAP account to a feed). */
  onOpenSettings: () => void;
}

function toSec(d: Date): number {
  return Math.floor(d.getTime() / 1000);
}

/**
 * Right-docked day agenda — the calendar's default surface, like the side
 * panel in Gmail: one day of the account the user is in on an hour-by-hour
 * grid (the same `TimeGrid` as the day view, so clicks and double-clicks
 * behave the same), with the account picker when several have a calendar;
 * for an IMAP account without a feed it explains how to get one, and for an
 * account whose integration is switched off it offers to enable it.
 */
export function CalendarPanel({
  accounts,
  defaultAccountId,
  onClose,
  onOpenFullCalendar,
  onOpenSettings,
}: CalendarPanelProps) {
  const { t, i18n } = useTranslation(['calendar', 'common']);
  const addLog = useLogStore((s) => s.addLog);
  const enabledIds = useCalendarIntegrationStore((s) => s.enabledIds);
  const feedsByAccount = useCalendarIntegrationStore((s) => s.feedsByAccount);

  const currentAccount = useMemo(
    () => accounts.find((a) => a.id === defaultAccountId) ?? null,
    [accounts, defaultAccountId],
  );
  const panelState = calendarPanelState(currentAccount, enabledIds, feedsByAccount);

  const {
    calendarAccounts,
    disabledCurrentAccount,
    selectedAccountId,
    handOffSelection,
    isAllAccounts,
    followsAllAccounts,
    handleSelectAccount,
    enableCurrentCalendar,
    isEnabling,
  } = useCalendarAccount(accounts, defaultAccountId);

  const [anchor, setAnchor] = useState<Date>(() => startOfDay(new Date()));
  const rangeStart = toSec(anchor);
  const rangeEnd = toSec(addDays(anchor, 1));
  // The account the user is in needs a feed first — unless the sidebar is on
  // "All accounts", where the user is in no single account.
  const needsSetup = panelState === 'setup' && !followsAllAccounts;
  // No data traffic while the current account still has nothing to show.
  const shownAccountIds = needsSetup ? [] : calendarSelectionIds(selectedAccountId, calendarAccounts);
  const data = useCalendarData(shownAccountIds, rangeStart, rangeEnd);
  const accountById = useMemo(() => new Map(accounts.map((a) => [a.id, a])), [accounts]);
  const colorFor = useMemo(
    () => eventColorFor(isAllAccounts, calendarAccounts, data.colorFor),
    [isAllAccounts, calendarAccounts, data.colorFor],
  );
  const [selectedEvent, setSelectedEvent] = useState<CalendarEvent | null>(null);
  /** Proposed slot for the "New event" dialog (double-clicked grid position)
   *  and the accounts it can go to. */
  const [createSlot, setCreateSlot] = useState<{ start: number; end: number; choices: CreateAccountChoice[] } | null>(
    null,
  );

  // Same rule as the full view: the in-app dialog for the accounts that can
  // take the event, a lone Proton account straight to Proton Calendar.
  const openCreateSlot = useCallback(
    (start: number, end: number) => {
      const shown = calendarAccounts.filter((a) => shownAccountIds.includes(a.id));
      const choices = createAccountChoices(shown, data.calendars, start);
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
    [calendarAccounts, shownAccountIds, data.calendars, addLog, t],
  );

  const dayLabel = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language || 'en', { weekday: 'long', day: 'numeric', month: 'long' }).format(anchor),
    [anchor, i18n.language],
  );
  const days = useMemo(() => [anchor], [anchor]);

  const enable = useCallback(() => {
    enableCurrentCalendar().catch((e) => data.setError(errorText(e)));
  }, [enableCurrentCalendar, data.setError]);

  const iconButton = 'rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600';

  return (
    <div data-testid="calendar-panel" className="flex h-full flex-col bg-white">
      <div className="flex items-center gap-1 border-b border-gray-200 bg-white px-2 py-1.5">
        <span className="min-w-0 flex-1 truncate px-1 text-sm font-medium text-gray-900">
          {t('calendar:panel.title')}
        </span>
        <button
          type="button"
          onClick={() => {
            // The full view opens on what this agenda's selector shows.
            handOffSelection();
            onOpenFullCalendar();
          }}
          title={t('calendar:panel.openFull')}
          aria-label={t('calendar:panel.openFull')}
          className={iconButton}
        >
          <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5}>
            <path d="M6 2H2v4M10 14h4v-4" strokeLinecap="round" strokeLinejoin="round" />
            <path d="M2 2l5 5M14 14l-5-5" strokeLinecap="round" />
          </svg>
        </button>
        <button
          type="button"
          onClick={onClose}
          title={t('calendar:panel.close')}
          aria-label={t('calendar:panel.close')}
          className={iconButton}
        >
          <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
            <path d="M4 4l8 8M12 4l-8 8" strokeLinecap="round" />
          </svg>
        </button>
      </div>

      {needsSetup && currentAccount ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-5 text-center">
          <p className="text-sm font-medium text-gray-900">{t('calendar:panel.setupTitle')}</p>
          <p className="text-xs text-gray-500 break-words">{currentAccount.email}</p>
          <p className="text-xs text-gray-600">{t('calendar:panel.setupBody')}</p>
          <button
            type="button"
            onClick={onOpenSettings}
            className="px-3 py-1.5 rounded-md bg-primary-600 text-white text-xs font-medium hover:bg-primary-700"
          >
            {t('calendar:panel.setupButton')}
          </button>
        </div>
      ) : disabledCurrentAccount ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-5 text-center">
          <p className="text-sm text-gray-800 break-words">
            {t('calendar:integrationDisabled', { email: disabledCurrentAccount.email })}
          </p>
          <button
            type="button"
            onClick={enable}
            disabled={isEnabling}
            className="px-3 py-1.5 rounded-md bg-primary-600 text-white text-xs font-medium hover:bg-primary-700 disabled:opacity-60"
          >
            {t('calendar:enableCalendarButton')}
          </button>
          {data.error && <p className="text-xs text-red-700 break-words">{data.error}</p>}
        </div>
      ) : calendarAccounts.length === 0 ? (
        <div className="flex flex-1 items-center justify-center px-4 text-center text-xs text-gray-500">
          {t('calendar:noAccount')}
        </div>
      ) : (
        <>
          {calendarAccounts.length > 1 && selectedAccountId && (
            <div className="border-b border-gray-100 px-2 py-1.5">
              <Select
                value={selectedAccountId}
                options={calendarAccountOptions(calendarAccounts, t('calendar:allAccounts'))}
                onChange={handleSelectAccount}
                ariaLabel={t('calendar:selectAccount')}
                size="xs"
                fullWidth
              />
            </div>
          )}
          <div className="flex items-center gap-1 border-b border-gray-100 px-2 py-1.5">
            <button
              type="button"
              onClick={() => setAnchor((d) => addDays(d, -1))}
              title={t('calendar:previous')}
              aria-label={t('calendar:previous')}
              className={iconButton}
            >
              <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
                <path d="M10 3L5 8l5 5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </button>
            <button
              type="button"
              onClick={() => setAnchor((d) => addDays(d, 1))}
              title={t('calendar:next')}
              aria-label={t('calendar:next')}
              className={iconButton}
            >
              <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
                <path d="M6 3l5 5-5 5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </button>
            <span className="min-w-0 flex-1 truncate text-xs font-medium text-gray-800 capitalize">{dayLabel}</span>
            <button
              type="button"
              onClick={() => setAnchor(startOfDay(new Date()))}
              className="rounded border border-gray-200 px-2 py-0.5 text-xs text-gray-700 hover:bg-gray-100"
            >
              {t('calendar:today')}
            </button>
          </div>

          {data.reauthAccountIds.map((reauthId) => (
            <div
              key={reauthId}
              className="flex items-center gap-2 border-b border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900"
            >
              <span className="min-w-0 flex-1 break-words">
                {isAllAccounts
                  ? t('calendar:reauthNeededAccount', { email: accountById.get(reauthId)?.email ?? reauthId })
                  : t('calendar:reauthNeeded')}
              </span>
              <button
                type="button"
                onClick={() => void data.runReauth(reauthId)}
                disabled={data.reauthingAccountId !== null}
                className="flex-shrink-0 rounded bg-amber-600 px-2 py-1 text-xs font-medium text-white hover:bg-amber-700 disabled:opacity-60"
              >
                {data.reauthingAccountId === reauthId ? t('calendar:reauthInProgress') : t('calendar:reauthButton')}
              </button>
            </div>
          ))}
          {data.error && (
            <div className="border-b border-red-200 bg-red-50 px-3 py-2 text-xs text-red-800 break-words">
              {t('calendar:syncError', { message: data.error })}
            </div>
          )}
          {data.syncErrors.map((e) => (
            <div
              key={e.accountId}
              className="border-b border-red-200 bg-red-50 px-3 py-2 text-xs text-red-800 break-words"
            >
              {t('calendar:syncError', {
                message: isAllAccounts
                  ? `${accountById.get(e.accountId)?.email ?? e.accountId}: ${e.message}`
                  : e.message,
              })}
            </div>
          ))}

          <TimeGrid
            days={days}
            events={data.shownEvents}
            colorFor={colorFor}
            onSelectEvent={setSelectedEvent}
            onCreateSlot={openCreateSlot}
          />
        </>
      )}

      {selectedEvent && (
        <EventDetailDialog
          event={selectedEvent}
          accountId={selectedEvent.accountId}
          accountEmail={isAllAccounts ? accountById.get(selectedEvent.accountId)?.email : undefined}
          provider={accountById.get(selectedEvent.accountId)?.provider ?? ''}
          readOnly={data.calendars.some(
            (c) =>
              c.accountId === selectedEvent.accountId &&
              c.providerCalendarId === selectedEvent.calendarId &&
              isReadOnlyCalendar(c),
          )}
          onClose={() => setSelectedEvent(null)}
          onDeleted={(eventId, scope, recurringEventId, startTime) => {
            data.setEvents((prev) => eventsAfterDelete(prev, { id: eventId, recurringEventId, startTime }, scope));
            setSelectedEvent(null);
          }}
          onAuthError={() => {
            setSelectedEvent(null);
            data.markNeedsReauth(selectedEvent.accountId);
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
              void data.runSync();
            } else {
              data.setEvents((prev) => [...prev.filter((e) => e.id !== created.id), created]);
            }
          }}
          onAuthError={(accountId) => {
            setCreateSlot(null);
            data.markNeedsReauth(accountId);
          }}
        />
      )}
    </div>
  );
}
