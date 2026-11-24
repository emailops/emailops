import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import { describeRecurrence } from '@/lib/calendarEvent';
import { errorText, isAuthError } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useCalendarChangeStore } from '@/stores/calendarChangeStore';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { CalendarInvite, Email, InviteResponseOutcome, RsvpChoice } from '@/types';

/** Inline note under the RSVP buttons. `info` covers the "invite hasn't
 *  reached your calendar yet" case, `auth` suggests re-authenticating. */
interface RsvpNote {
  kind: 'info' | 'auth' | 'error' | 'warn';
  message: string;
}

/** What the card shows once an answer (or "Add to calendar") went through.
 *  `choice` is null for a publish, which has no answer to confirm. */
interface InviteDone {
  choice: RsvpChoice | null;
  added: boolean;
  alreadyInCalendar: boolean;
}

/**
 * Gmail-style calendar invite card rendered above the email body when the
 * email carries a .ics invite. Only loads for accounts with calendar
 * integration (Gmail / Outlook — never IMAP). An answer that changed the
 * calendar marks it changed, so an open calendar view or panel re-syncs.
 */
export function CalendarInviteCard({ email }: { email: Email }) {
  const { t, i18n } = useTranslation(['calendar']);
  const { date, time, dateTime } = useFormatters();
  const addLog = useLogStore((s) => s.addLog);
  const accounts = useAccountStore((s) => s.accounts);
  const markCalendarChanged = useCalendarChangeStore((s) => s.markChanged);

  const account = accounts.find((a) => a.id === email.accountId);
  // Only for accounts with calendar integration enabled (Settings →
  // Calendar) — don't even probe for an invite otherwise. Waiting for
  // `isLoaded` avoids probing before the prefs arrive.
  const calendarIntegrationIds = useCalendarIntegrationStore((s) => s.enabledIds);
  const calendarIntegrationLoaded = useCalendarIntegrationStore((s) => s.isLoaded);
  const hasCalendar =
    !!account && account.provider !== 'imap' && calendarIntegrationLoaded && calendarIntegrationIds.has(account.id);

  // Invite cache, keyed by email id so a stale response for a previous email
  // can never render against the current one.
  const [loaded, setLoaded] = useState<{ emailId: string; invite: CalendarInvite | null } | null>(null);

  const [rsvpBusy, setRsvpBusy] = useState<RsvpChoice | null>(null);
  const [rsvpDone, setRsvpDone] = useState<InviteDone | null>(null);
  const [addBusy, setAddBusy] = useState(false);
  const [rsvpNote, setRsvpNote] = useState<RsvpNote | null>(null);

  useEffect(() => {
    if (!hasCalendar || loaded?.emailId === email.id) return;
    let cancelled = false;
    api
      .getCalendarInvite(email.accountId, email.id)
      .then((invite) => {
        if (cancelled) return;
        setLoaded({ emailId: email.id, invite });
      })
      .catch((e) => {
        if (cancelled) return;
        // Surface the failure in the output panel; the card simply stays hidden.
        addLog('error', 'sync', `Failed to load calendar invite: ${errorText(e)}`);
        setLoaded({ emailId: email.id, invite: null });
      });
    return () => {
      cancelled = true;
    };
  }, [hasCalendar, email.id, loaded, addLog]);

  const invite = loaded?.emailId === email.id ? loaded.invite : null;
  if (!hasCalendar || !invite) return null;

  const applyOutcome = (outcome: InviteResponseOutcome, choice: RsvpChoice | null) => {
    switch (outcome.kind) {
      case 'notYetInCalendar':
        // The invite can take a minute to land in the provider calendar.
        setRsvpNote({ kind: 'info', message: t('calendar:invite.notYetInCalendar') });
        return;
      case 'cannotImport': {
        const text = t(`calendar:import.reason.${outcome.reason}` as const, { defaultValue: outcome.reason });
        const reason = outcome.reasonDetail ? `${text} (${outcome.reasonDetail})` : text;
        setRsvpNote({ kind: 'error', message: t('calendar:invite.addError', { message: reason }) });
        return;
      }
      case 'alreadyInCalendar':
        setRsvpDone({ choice, added: false, alreadyInCalendar: true });
        return;
      case 'responded':
        setRsvpDone({ choice, added: false, alreadyInCalendar: false });
        markCalendarChanged(email.accountId);
        addLog('success', 'sync', `Calendar invite response sent (${choice})`);
        return;
      case 'imported':
      case 'removed':
        setRsvpDone({ choice, added: outcome.kind === 'imported', alreadyInCalendar: false });
        markCalendarChanged(email.accountId);
        if (outcome.replyError) {
          setRsvpNote({ kind: 'warn', message: t(`calendar:invite.replyNotSent.${outcome.kind}` as const) });
          // Fixed text: the error can name the organizer; the backend logged its code.
          addLog('error', 'sync', 'Calendar invite reply failed');
        } else {
          addLog('success', 'sync', `Calendar invite ${outcome.kind} (${choice ?? 'publish'})`);
        }
        return;
    }
  };

  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone;

  const handleRsvp = async (choice: RsvpChoice) => {
    if (rsvpBusy) return;
    setRsvpBusy(choice);
    setRsvpNote(null);
    try {
      applyOutcome(await api.rsvpCalendarInvite(email.accountId, email.id, choice, timeZone), choice);
    } catch (e) {
      const msg = errorText(e);
      addLog('error', 'sync', `Failed to respond to calendar invite: ${msg}`);
      if (isAuthError(e, msg)) {
        setRsvpNote({ kind: 'auth', message: t('calendar:invite.authError') });
      } else {
        setRsvpNote({ kind: 'error', message: t('calendar:invite.error', { message: msg }) });
      }
    } finally {
      setRsvpBusy(null);
    }
  };

  const handleAdd = async () => {
    if (addBusy) return;
    setAddBusy(true);
    setRsvpNote(null);
    try {
      applyOutcome(await api.addInviteToCalendar(email.accountId, email.id, timeZone), null);
    } catch (e) {
      const msg = errorText(e);
      addLog('error', 'sync', `Failed to add calendar invite: ${msg}`);
      if (isAuthError(e, msg)) {
        setRsvpNote({ kind: 'auth', message: t('calendar:invite.authError') });
      } else {
        setRsvpNote({ kind: 'error', message: t('calendar:invite.addError', { message: msg }) });
      }
    } finally {
      setAddBusy(false);
    }
  };

  // "Tue 28 Jul · 07:30 – 08:30" — same shape the event-detail dialog uses.
  const dayOptions: Intl.DateTimeFormatOptions = { weekday: 'short', day: 'numeric', month: 'short' };
  const sameDay = new Date(invite.startTime * 1000).toDateString() === new Date(invite.endTime * 1000).toDateString();
  const dateLine = invite.isAllDay
    ? `${date(invite.startTime, dayOptions)} · ${t('calendar:allDay')}`
    : sameDay
      ? `${date(invite.startTime, dayOptions)} · ${time(invite.startTime)} – ${time(invite.endTime)}`
      : `${dateTime(invite.startTime)} – ${dateTime(invite.endTime)}`;

  const recurrence = invite.recurrence
    ? describeRecurrence(invite.recurrence, new Date(invite.startTime * 1000), i18n.language || 'en')
    : null;
  const recurrenceLine = recurrence
    ? recurrence.labelKey
      ? t(`calendar:create.recurrence.${recurrence.labelKey}`, recurrence.params)
      : recurrence.raw
    : null;

  const isCancelled = invite.method === 'CANCEL';

  return (
    <div className="mb-4 border border-gray-200 rounded-lg bg-white shadow-sm p-4 flex items-start gap-3">
      <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-primary-50 flex items-center justify-center">
        <svg className="w-5 h-5 text-primary-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"
          />
        </svg>
      </div>

      <div className="min-w-0 flex-1 space-y-1">
        <p className="text-sm text-gray-600">{dateLine}</p>
        <h4 className="text-base font-semibold text-gray-900 break-words">
          {invite.summary || t('calendar:invite.noTitle')}
        </h4>
        {recurrenceLine && <p className="text-sm text-gray-600 break-words">{recurrenceLine}</p>}
        {invite.organizer && (
          <p className="text-sm text-gray-500 break-all">
            {t('calendar:invite.organizer', { organizer: invite.organizer })}
          </p>
        )}
        {invite.location && <p className="text-sm text-gray-500 break-words">{invite.location}</p>}

        {isCancelled ? (
          <p className="pt-1 text-sm text-gray-500 italic">{t('calendar:invite.cancelled')}</p>
        ) : rsvpDone ? (
          <p className="pt-1 text-sm text-gray-700">
            {rsvpDone.alreadyInCalendar
              ? t('calendar:invite.alreadyInCalendar')
              : rsvpDone.choice
                ? t(`calendar:invite.confirmed.${rsvpDone.choice}` as const)
                : t('calendar:invite.addedToCalendar')}
            {rsvpDone.choice && rsvpDone.added && <> {t('calendar:invite.addedToCalendar')}</>}{' '}
            {rsvpDone.choice && (
              <button
                type="button"
                onClick={() => {
                  setRsvpDone(null);
                  setRsvpNote(null);
                }}
                className="text-primary-600 hover:text-primary-700 hover:underline"
              >
                {t('calendar:invite.change')}
              </button>
            )}
          </p>
        ) : invite.isPublish ? (
          <div className="pt-1.5">
            <RsvpButton
              label={t('calendar:invite.addToCalendar')}
              variant="filled"
              busy={addBusy}
              disabled={addBusy}
              onClick={() => void handleAdd()}
            />
          </div>
        ) : (
          <>
            <div className="pt-1.5 flex items-center gap-2 flex-wrap">
              <span className="text-sm text-gray-700 mr-1">{t('calendar:invite.going')}</span>
              <RsvpButton
                label={t('calendar:invite.yes')}
                variant="filled"
                busy={rsvpBusy === 'accepted'}
                disabled={rsvpBusy !== null}
                onClick={() => void handleRsvp('accepted')}
              />
              <RsvpButton
                label={t('calendar:invite.no')}
                variant="outline"
                busy={rsvpBusy === 'declined'}
                disabled={rsvpBusy !== null}
                onClick={() => void handleRsvp('declined')}
              />
              <RsvpButton
                label={t('calendar:invite.maybe')}
                variant="outline"
                busy={rsvpBusy === 'tentative'}
                disabled={rsvpBusy !== null}
                onClick={() => void handleRsvp('tentative')}
              />
            </div>
            {invite.replyTo && (
              <p className={`pt-1 text-xs break-all ${invite.organizerIsSender ? 'text-gray-500' : 'text-amber-700'}`}>
                {t(invite.organizerIsSender ? 'calendar:invite.replyTo' : 'calendar:invite.replyToNotSender', {
                  address: invite.replyTo,
                })}
              </p>
            )}
          </>
        )}

        {rsvpNote && (
          <p
            className={`pt-1 text-xs break-words ${
              rsvpNote.kind === 'error'
                ? 'text-red-700'
                : rsvpNote.kind === 'auth' || rsvpNote.kind === 'warn'
                  ? 'text-amber-700'
                  : 'text-gray-500'
            }`}
          >
            {rsvpNote.message}
          </p>
        )}
      </div>
    </div>
  );
}

function RsvpButton({
  label,
  variant,
  busy,
  disabled,
  onClick,
}: {
  label: string;
  variant: 'filled' | 'outline';
  busy: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  const base =
    variant === 'filled'
      ? 'bg-primary-600 text-white hover:bg-primary-700'
      : 'border border-gray-300 text-gray-700 hover:bg-gray-50';
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`px-4 py-1.5 text-sm font-medium rounded-md transition-colors disabled:opacity-50 flex items-center gap-1.5 ${base}`}
    >
      {busy && (
        <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
          <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
          <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
        </svg>
      )}
      {label}
    </button>
  );
}
