import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import * as api from '@/lib/api';
import { calendarColor } from '@/lib/calendarColor';
import { errorText, isAuthError } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { calendarCapableAccounts, useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { Calendar } from '@/types';

/** Lead-time choices for the upcoming-meeting notification (minutes). */
const LEAD_TIME_OPTIONS = [1, 5, 10, 15, 30, 60] as const;

const DEFAULT_LEAD_MINUTES = 10;

/**
 * Calendar settings panel: per-account calendar-integration toggles (the
 * master switch for all calendar features — sidebar view, invite cards, chat
 * tool, sync and meeting notifications), plus the meeting-notification enable
 * toggle and lead-time selector. Everything is stored via the backend prefs
 * commands (`calendar.enabled:<account_id>`, `calendar_notifications_enabled`,
 * `calendar_notify_minutes` — the backend validates the values).
 */
export function CalendarSettings() {
  const { t } = useTranslation(['common', 'settings', 'calendar']);
  const addLog = useLogStore((s) => s.addLog);
  const accounts = useAccountStore((s) => s.accounts);
  const integrationIds = useCalendarIntegrationStore((s) => s.enabledIds);
  const integrationLoaded = useCalendarIntegrationStore((s) => s.isLoaded);
  const setIntegrationEnabled = useCalendarIntegrationStore((s) => s.setEnabled);
  const capableAccounts = calendarCapableAccounts(accounts);
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  /** The OS is refusing notifications — the toggle is on but nothing arrives. */
  const [notificationsBlocked, setNotificationsBlocked] = useState(false);
  const [leadMinutes, setLeadMinutes] = useState<number>(DEFAULT_LEAD_MINUTES);
  const [isLoaded, setIsLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Account whose enable-sync failed with an auth/permission error — shows
   *  the inline "sign in again" banner so re-consent doesn't require leaving
   *  Settings. */
  const [reauthAccountId, setReauthAccountId] = useState<string | null>(null);
  const [isReauthing, setIsReauthing] = useState(false);
  /** Calendars per account id, for the per-calendar show/hide list. Loaded
   *  only for accounts whose integration is on — a disabled account has no
   *  registry to show. */
  const [calendarsByAccount, setCalendarsByAccount] = useState<Record<string, Calendar[]>>({});

  const runEnableSync = (accountId: string) => {
    // First sync right away so the calendar fills in without waiting for the
    // 5-minute poll tick. Permission/auth failures surface as the inline
    // re-auth banner; anything else goes to the output panel.
    api
      .syncCalendarNow(accountId)
      .then((stored) => {
        setReauthAccountId((current) => (current === accountId ? null : current));
        addLog('success', 'sync', `Calendar synced (${stored} events)`);
      })
      .catch((e) => {
        const msg = errorText(e);
        addLog('error', 'sync', `Calendar sync failed: ${msg}`);
        if (isAuthError(e, msg)) setReauthAccountId(accountId);
      });
  };

  const toggleAccountIntegration = (accountId: string, next: boolean) => {
    setError(null);
    if (!next) setReauthAccountId((current) => (current === accountId ? null : current));
    setIntegrationEnabled(accountId, next)
      .then(() => {
        if (!next) return;
        addLog('info', 'sync', 'Calendar integration enabled — syncing…');
        runEnableSync(accountId);
      })
      .catch((e) => {
        setError(errorText(e));
        addLog('error', 'system', `Failed to save calendar integration pref: ${errorText(e)}`);
      });
  };

  const runReauth = (accountId: string) => {
    setIsReauthing(true);
    setError(null);
    api
      .reauthenticateAccount(accountId)
      .then(() => {
        addLog('success', 'account', 'Account re-authenticated');
        // The account may have been auto-disabled while permission was
        // missing — re-enable now that consent was granted, then sync.
        return setIntegrationEnabled(accountId, true).then(() => {
          setReauthAccountId(null);
          runEnableSync(accountId);
        });
      })
      .catch((e) => {
        const msg = errorText(e);
        setError(msg);
        addLog('error', 'account', `Re-authentication failed: ${msg}`);
      })
      .finally(() => setIsReauthing(false));
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [enabledRaw, minutesRaw] = await Promise.all([
          api.getPref('calendar_notifications_enabled'),
          api.getPref('calendar_notify_minutes'),
        ]);
        if (cancelled) return;
        setNotificationsEnabled(enabledRaw !== 'false'); // default: on
        const parsed = minutesRaw != null ? Number.parseInt(minutesRaw, 10) : Number.NaN;
        setLeadMinutes(Number.isFinite(parsed) && parsed >= 1 && parsed <= 120 ? parsed : DEFAULT_LEAD_MINUTES);
        setIsLoaded(true);
      } catch (e) {
        if (cancelled) return;
        setError(errorText(e));
        setIsLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Load the calendar registry for every account whose integration is on.
  // Keyed on the id list rather than the Set itself so enabling an account
  // fetches its calendars without refetching the others on every render.
  const enabledAccountKey = capableAccounts
    .filter((a) => integrationIds.has(a.id))
    .map((a) => a.id)
    .sort()
    .join(',');
  useEffect(() => {
    if (!integrationLoaded) return;
    let cancelled = false;
    const ids = enabledAccountKey ? enabledAccountKey.split(',') : [];
    void (async () => {
      const entries = await Promise.all(
        ids.map(async (id) => {
          try {
            return [id, await api.getCalendars(id)] as const;
          } catch (e) {
            addLog('error', 'sync', `Failed to load calendars: ${errorText(e)}`);
            return [id, [] as Calendar[]] as const;
          }
        }),
      );
      if (cancelled) return;
      setCalendarsByAccount(Object.fromEntries(entries));
    })();
    return () => {
      cancelled = true;
    };
  }, [enabledAccountKey, integrationLoaded, addLog]);

  const persistEnabled = (next: boolean) => {
    const previous = notificationsEnabled;
    setNotificationsEnabled(next);
    setError(null);
    setNotificationsBlocked(false);
    api.setPref('calendar_notifications_enabled', next ? 'true' : 'false').catch((e) => {
      // Revert the optimistic flip and surface the failure.
      setNotificationsEnabled(previous);
      setError(errorText(e));
      addLog('error', 'system', `Failed to save calendar notification pref: ${errorText(e)}`);
    });
    if (!next) return;
    // Ask the OS only now. iOS raises its prompt once and a denial is
    // permanent, so it must land on a screen where the user has just said they
    // want reminders — not at startup, where the ask has no context. A no-op
    // on desktop, where the plugin always reports granted.
    api
      .ensureNotificationPermission()
      .then((state) => {
        setNotificationsBlocked(state === 'denied');
        if (state === 'denied') {
          addLog('info', 'system', 'Meeting reminders are enabled but the OS is blocking notifications');
        }
      })
      .catch((e) => {
        // Not fatal: the in-app reminder banner still fires. Only the OS
        // notification is lost, so this is logged rather than raised.
        addLog('error', 'system', `Notification permission check failed: ${errorText(e)}`);
      });
  };

  const loadCalendarsFor = (accountId: string) => {
    api
      .getCalendars(accountId)
      .then((list) => setCalendarsByAccount((current) => ({ ...current, [accountId]: list })))
      .catch((e) => {
        // Non-fatal: the account toggle above still works, only the
        // per-calendar list is missing.
        addLog('error', 'sync', `Failed to load calendars: ${errorText(e)}`);
      });
  };

  const toggleCalendarVisible = (accountId: string, calendar: Calendar, next: boolean) => {
    setError(null);
    setCalendarsByAccount((current) => ({
      ...current,
      [accountId]: (current[accountId] ?? []).map((c) =>
        c.providerCalendarId === calendar.providerCalendarId ? { ...c, isVisible: next } : c,
      ),
    }));
    api.setCalendarVisible(accountId, calendar.providerCalendarId, next).catch((e) => {
      // Revert the optimistic flip by reloading the authoritative list.
      setError(errorText(e));
      addLog('error', 'system', `Failed to save calendar visibility: ${errorText(e)}`);
      loadCalendarsFor(accountId);
    });
  };

  const persistLeadMinutes = (next: number) => {
    const previous = leadMinutes;
    setLeadMinutes(next);
    setError(null);
    api.setPref('calendar_notify_minutes', String(next)).catch((e) => {
      setLeadMinutes(previous);
      setError(errorText(e));
      addLog('error', 'system', `Failed to save calendar lead-time pref: ${errorText(e)}`);
    });
  };

  return (
    <div className="flex flex-col flex-1 min-h-0">
      {/* Error banner — pinned above the scroll container, always visible. */}
      {error && (
        <div className="flex-shrink-0 mx-6 mt-4 border border-red-800 bg-red-950 text-red-200 text-sm rounded p-3">
          {error}
        </div>
      )}
      <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
        <section>
          <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:calendar.accountsLabel')}</h3>
          <p className="text-xs text-gray-500 mb-2">{t('settings:calendar.accountsDesc')}</p>
          {capableAccounts.length === 0 ? (
            <p className="text-xs text-gray-400">{t('settings:calendar.noCapableAccounts')}</p>
          ) : (
            <div className="rounded-lg border border-gray-700 bg-[#1f1f20] divide-y divide-gray-700">
              {capableAccounts.map((account) => {
                const enabled = integrationIds.has(account.id);
                const accountCalendars = calendarsByAccount[account.id] ?? [];
                return (
                  <div key={account.id}>
                    <div className="flex items-center justify-between gap-4 px-4 py-3">
                      <div className="min-w-0">
                        <span className="text-sm text-gray-100 block truncate">{account.email}</span>
                        <span className="text-xs text-gray-500 capitalize">{account.provider}</span>
                      </div>
                      <button
                        type="button"
                        role="switch"
                        aria-checked={enabled}
                        aria-label={t('settings:calendar.accountToggleAria', { email: account.email })}
                        disabled={!integrationLoaded}
                        onClick={() => toggleAccountIntegration(account.id, !enabled)}
                        className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors disabled:opacity-50 ${
                          enabled ? 'bg-primary-600' : 'bg-neutral-600'
                        }`}
                      >
                        <span
                          className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                            enabled ? 'translate-x-5' : 'translate-x-1'
                          }`}
                        />
                      </button>
                    </div>
                    {/* Which of the account's calendars appear in the calendar
                        view. Hidden ones keep syncing, so re-showing one is
                        instant. Only rendered when there is a real choice. */}
                    {enabled && accountCalendars.length > 1 && (
                      <div className="px-4 pb-3 -mt-1 space-y-1.5">
                        {accountCalendars.map((calendar) => (
                          <label
                            key={calendar.providerCalendarId}
                            className="flex items-center gap-2.5 cursor-pointer group"
                          >
                            <input
                              type="checkbox"
                              checked={calendar.isVisible}
                              onChange={(e) => toggleCalendarVisible(account.id, calendar, e.target.checked)}
                              className="rounded border-gray-600 bg-transparent text-primary-600 focus:ring-primary-600 focus:ring-offset-0"
                            />
                            <span
                              className="w-2.5 h-2.5 rounded-full flex-shrink-0 border border-black/20"
                              style={{
                                backgroundColor: calendarColor(calendar.color, calendar.providerCalendarId),
                              }}
                            />
                            <span className="text-xs text-gray-300 truncate group-hover:text-gray-100">
                              {calendar.name || calendar.providerCalendarId}
                            </span>
                            {calendar.isPrimary && (
                              <span className="text-[10px] text-gray-500 flex-shrink-0">
                                {t('settings:calendar.primaryCalendar')}
                              </span>
                            )}
                          </label>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
          {reauthAccountId && (
            <div className="mt-2 rounded-lg border border-amber-700 bg-amber-950 px-4 py-3">
              <p className="text-xs text-amber-200">{t('calendar:reauthNeeded')}</p>
              <button
                type="button"
                disabled={isReauthing}
                onClick={() => runReauth(reauthAccountId)}
                className="mt-2 px-3 py-1.5 rounded bg-amber-600 hover:bg-amber-500 text-white text-xs font-medium transition-colors disabled:opacity-60"
              >
                {isReauthing ? t('calendar:reauthInProgress') : t('calendar:reauthButton')}
              </button>
            </div>
          )}
        </section>

        <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <span className="text-sm font-medium text-gray-100">{t('settings:calendar.notificationsLabel')}</span>
              <p className="text-xs text-gray-400 mt-1">{t('settings:calendar.notificationsDesc')}</p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={notificationsEnabled}
              disabled={!isLoaded}
              onClick={() => persistEnabled(!notificationsEnabled)}
              className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors mt-0.5 disabled:opacity-50 ${
                notificationsEnabled ? 'bg-primary-600' : 'bg-neutral-600'
              }`}
            >
              <span
                className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                  notificationsEnabled ? 'translate-x-5' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
          {notificationsBlocked && (
            <p className="mt-2 text-xs text-amber-400">{t('settings:calendar.notificationsBlocked')}</p>
          )}
        </section>

        <section>
          <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:calendar.leadTimeLabel')}</h3>
          <p className="text-xs text-gray-500 mb-2">{t('settings:calendar.leadTimeDesc')}</p>
          <Select
            value={String(leadMinutes)}
            disabled={!isLoaded || !notificationsEnabled}
            onChange={(value) => persistLeadMinutes(Number.parseInt(value, 10))}
            ariaLabel={t('settings:calendar.leadTimeLabel')}
            options={
              // Include an out-of-list stored value (backend accepts 1–120) so the
              // select never silently shows the wrong lead time.
              (LEAD_TIME_OPTIONS.includes(leadMinutes as (typeof LEAD_TIME_OPTIONS)[number])
                ? [...LEAD_TIME_OPTIONS]
                : [...LEAD_TIME_OPTIONS, leadMinutes].sort((a, b) => a - b)
              ).map((minutes) => ({
                value: String(minutes),
                label: t('settings:calendar.minutesOption', { n: minutes }),
              }))
            }
          />
        </section>
      </div>
    </div>
  );
}
