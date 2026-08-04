import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import {
  type AttendeeStatusKind,
  attendeeStatusMeta,
  type CalendarDeleteScope,
  cancellationMessage,
  linkifySegments,
  visibleAttendees,
} from '@/lib/calendarEvent';
import { getSafeExternalUrl } from '@/lib/emailFormatting';
import { errorText, isAuthError } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { CalendarEvent } from '@/types';

/**
 * Provider descriptions (Graph) can contain raw HTML — UNTRUSTED input. We
 * never render it as HTML; instead extract the text content via an inert
 * DOMParser document (no script execution, no resource loading) and show it
 * as plain text.
 */
export function htmlToPlainText(html: string): string {
  if (!html) return '';
  const doc = new DOMParser().parseFromString(html, 'text/html');
  return (doc.body.textContent ?? '').replace(/\u00a0/g, ' ').trim();
}

interface EventDetailDialogProps {
  event: CalendarEvent;
  accountId: string;
  /** Provider of the calendar account ('gmail' | 'outlook'). Drives the
   *  delete-confirm notify affordances. */
  provider: string;
  onClose: () => void;
  /** Deleted on the provider — parent removes the affected occurrence(s)
   *  from all views and closes. For recurring instances `scope` widens what
   *  must go (see `eventsAfterDelete`); `recurringEventId`/`startTime` echo
   *  the deleted event so the parent can filter without re-looking it up. */
  onDeleted: (eventId: string, scope: CalendarDeleteScope, recurringEventId: string | null, startTime: number) => void;
  /** Auth-class failure — parent closes the dialog and shows its re-auth banner. */
  onAuthError: () => void;
}

/** Centered detail popover for a clicked calendar event. */
export function EventDetailDialog({
  event,
  accountId,
  provider,
  onClose,
  onDeleted,
  onAuthError,
}: EventDetailDialogProps) {
  const { t } = useTranslation(['calendar', 'common']);
  const { date, time, dateTime } = useFormatters();
  const addLog = useLogStore((s) => s.addLog);

  const joinUrl = event.meetingLink ? getSafeExternalUrl(event.meetingLink) : null;
  const htmlLinkUrl = event.htmlLink ? getSafeExternalUrl(event.htmlLink) : null;
  const description = htmlToPlainText(event.description);
  // The organizer has their own row above — don't list them again as an attendee.
  const attendeeList = visibleAttendees(event.attendees, event.organizer);

  // Inline delete-confirm state (no browser confirm).
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  // Recurring instances offer instance / following / all; non-recurring
  // events have no radio group and always delete with 'instance'.
  const isRecurringInstance = event.recurringEventId !== null;
  const [deleteScope, setDeleteScope] = useState<CalendarDeleteScope>('instance');
  const [notifyAttendees, setNotifyAttendees] = useState(true);
  const [cancelMessage, setCancelMessage] = useState('');
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const hasOtherAttendees = event.attendees.some((a) => attendeeStatusMeta(a.response) !== 'organizer');

  const openLink = (url: string) => {
    openExternal(url).catch((err) => {
      addLog('error', 'system', `Failed to open link: ${err}`);
    });
  };

  const handleDelete = async () => {
    if (isDeleting) return;
    setIsDeleting(true);
    setDeleteError(null);
    try {
      const notify = hasOtherAttendees && notifyAttendees;
      const scope = isRecurringInstance ? deleteScope : 'instance';
      await api.deleteCalendarEvent(
        accountId,
        event.calendarId,
        event.providerEventId,
        notify,
        cancellationMessage(notify, provider, cancelMessage),
        scope,
      );
      addLog('success', 'sync', 'Calendar event deleted');
      onDeleted(event.id, scope, event.recurringEventId, event.startTime);
    } catch (e) {
      const msg = errorText(e);
      addLog('error', 'sync', `Failed to delete calendar event: ${msg}`);
      if (isAuthError(e, msg)) {
        onAuthError();
      } else {
        setDeleteError(msg);
      }
    } finally {
      setIsDeleting(false);
    }
  };

  const sameDay = new Date(event.startTime * 1000).toDateString() === new Date(event.endTime * 1000).toDateString();
  const timeRange = event.isAllDay
    ? `${date(event.startTime)} · ${t('calendar:allDay')}`
    : sameDay
      ? `${date(event.startTime)} · ${time(event.startTime)} – ${time(event.endTime)}`
      : `${dateTime(event.startTime)} – ${dateTime(event.endTime)}`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-white border border-gray-200 rounded-lg w-full max-w-md max-h-[80vh] shadow-xl flex flex-col overflow-hidden mx-4">
        {/* Delete-error banner — pinned at the very top, above all scrollable content. */}
        {deleteError && (
          <div className="flex-shrink-0 border-b border-red-200 bg-red-50 px-4 py-2 flex items-start gap-2 text-sm text-red-800">
            <svg className="w-4 h-4 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
              <path
                fillRule="evenodd"
                d="M10 18a8 8 0 100-16 8 8 0 000 16zM9 9a1 1 0 012 0v4a1 1 0 11-2 0V9zm1-5a1 1 0 100 2 1 1 0 000-2z"
                clipRule="evenodd"
              />
            </svg>
            <span className="min-w-0 flex-1 break-words">
              {t('calendar:detail.deleteError', { message: deleteError })}
            </span>
          </div>
        )}

        {/* Header — title + close, always visible (never scrolls away). */}
        <div className="flex items-start justify-between gap-3 px-5 pt-4 pb-3 border-b border-gray-100 flex-shrink-0">
          <div className="min-w-0">
            <h3 className="text-base font-semibold text-gray-900 break-words">
              {event.title || t('common:labels.noSubject')}
            </h3>
            <p className="text-sm text-gray-600 mt-0.5">{timeRange}</p>
            {event.status === 'tentative' && (
              <span className="inline-block mt-1 px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-amber-100 text-amber-800 border border-amber-200">
                {t('calendar:detail.tentative')}
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            title={t('common:actions.close')}
            className="flex-shrink-0 p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Prominent Join action — pinned above the scrollable details. */}
        {(joinUrl || htmlLinkUrl) && (
          <div className="flex items-center gap-3 px-5 py-3 border-b border-gray-100 flex-shrink-0">
            {joinUrl && (
              <button
                onClick={() => openLink(joinUrl)}
                className="px-4 py-2 rounded-lg bg-primary-600 text-white text-sm font-semibold hover:bg-primary-700 transition-colors"
              >
                {t('calendar:detail.join')}
              </button>
            )}
            {htmlLinkUrl && (
              <button
                onClick={() => openLink(htmlLinkUrl)}
                className="text-sm text-primary-600 hover:text-primary-700 hover:underline"
              >
                {t('calendar:detail.openInProvider')}
              </button>
            )}
          </div>
        )}

        <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4 text-sm">
          {event.location && (
            <DetailRow label={t('calendar:detail.location')}>
              <span className="text-gray-800 break-words">{event.location}</span>
            </DetailRow>
          )}
          {event.organizer && (
            <DetailRow label={t('calendar:detail.organizer')}>
              <span className="text-gray-800 break-all">{event.organizer}</span>
            </DetailRow>
          )}
          {attendeeList.length > 0 && (
            <DetailRow label={t('calendar:detail.attendees')}>
              <ul className="space-y-1">
                {attendeeList.map((attendee) => {
                  const status = attendeeStatusMeta(attendee.response);
                  return (
                    <li key={attendee.email} className="flex items-center gap-1.5 text-gray-800">
                      {status !== 'organizer' && (
                        <AttendeeStatusIcon status={status} label={t(`calendar:detail.attendeeStatus.${status}`)} />
                      )}
                      <span className="break-all">{attendee.email}</span>
                      {status === 'organizer' && (
                        <span
                          title={t('calendar:detail.attendeeStatus.organizer')}
                          className="flex-shrink-0 px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-gray-100 text-gray-600 border border-gray-200 select-none"
                        >
                          {t('calendar:detail.attendeeStatus.organizer')}
                        </span>
                      )}
                    </li>
                  );
                })}
              </ul>
            </DetailRow>
          )}
          {description && (
            <DetailRow label={t('calendar:detail.description')}>
              <p className="text-gray-800 whitespace-pre-wrap break-words">
                {linkifySegments(description).map((segment, index) => {
                  // Index keys are safe: the segment list is derived, static per render.
                  const key = `${index}-${segment.value.slice(0, 16)}`;
                  if (segment.kind === 'link') {
                    // Still gate through the safe-URL check before opening.
                    const safeUrl = getSafeExternalUrl(segment.value);
                    if (safeUrl) {
                      return (
                        <button
                          key={key}
                          onClick={() => openLink(safeUrl)}
                          className="text-primary-600 hover:text-primary-700 hover:underline break-all text-left"
                        >
                          {segment.value}
                        </button>
                      );
                    }
                  }
                  return <span key={key}>{segment.value}</span>;
                })}
              </p>
            </DetailRow>
          )}
        </div>

        {/* Footer — delete action with inline confirm (never a browser confirm). */}
        <div className="flex-shrink-0 border-t border-gray-100 px-5 py-3">
          {!confirmingDelete ? (
            <div className="flex justify-end">
              <button
                onClick={() => setConfirmingDelete(true)}
                className="flex items-center gap-1.5 px-3 py-1.5 text-sm border border-red-200 text-red-600 rounded-md hover:bg-red-50 transition-colors"
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                  />
                </svg>
                {t('calendar:detail.delete')}
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <p className="text-sm font-medium text-gray-900">{t('calendar:detail.deleteConfirmTitle')}</p>
              {isRecurringInstance && (
                <div className="space-y-1.5" role="radiogroup" aria-label={t('calendar:detail.deleteConfirmTitle')}>
                  {(['instance', 'following', 'all'] as const).map((scope) => (
                    <label key={scope} className="flex items-center gap-2 text-sm text-gray-700">
                      <input
                        type="radio"
                        name="event-delete-scope"
                        value={scope}
                        checked={deleteScope === scope}
                        onChange={() => setDeleteScope(scope)}
                        className="border-gray-300 text-primary-600 focus:ring-primary-500"
                      />
                      {t(`calendar:detail.deleteScope.${scope}` as const)}
                      {scope === 'all' && (
                        <span className="text-xs text-gray-400">{t('calendar:detail.deleteScope.allHint')}</span>
                      )}
                    </label>
                  ))}
                </div>
              )}
              {hasOtherAttendees && (
                <>
                  <label className="flex items-center gap-2 text-sm text-gray-700">
                    <input
                      type="checkbox"
                      checked={notifyAttendees}
                      onChange={(e) => setNotifyAttendees(e.target.checked)}
                      className="rounded border-gray-300 text-primary-600 focus:ring-primary-500"
                    />
                    {t('calendar:detail.notifyAttendees')}
                  </label>
                  {notifyAttendees && provider === 'outlook' && (
                    <div>
                      <label
                        htmlFor="event-cancel-message"
                        className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1"
                      >
                        {t('calendar:detail.cancellationMessage')}
                      </label>
                      <textarea
                        id="event-cancel-message"
                        rows={2}
                        value={cancelMessage}
                        onChange={(e) => setCancelMessage(e.target.value)}
                        placeholder={t('calendar:detail.cancellationMessagePlaceholder')}
                        className="w-full border border-gray-300 rounded-md px-3 py-2 text-sm text-gray-900 focus:outline-none focus:ring-1 focus:ring-primary-500 resize-y"
                      />
                    </div>
                  )}
                  {notifyAttendees && provider === 'gmail' && (
                    <p className="text-xs text-gray-500">{t('calendar:detail.googleCancellationHint')}</p>
                  )}
                </>
              )}
              <div className="flex justify-end gap-2">
                <button
                  onClick={() => setConfirmingDelete(false)}
                  disabled={isDeleting}
                  className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors disabled:opacity-50"
                >
                  {t('common:actions.cancel')}
                </button>
                <button
                  onClick={() => void handleDelete()}
                  disabled={isDeleting}
                  className="px-4 py-1.5 text-sm rounded-md bg-red-600 text-white font-medium hover:bg-red-700 transition-colors disabled:opacity-50 flex items-center gap-1.5"
                >
                  {isDeleting && (
                    <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
                    </svg>
                  )}
                  {isDeleting ? t('calendar:detail.deleting') : t('calendar:detail.deleteButton')}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** RSVP status glyph with a localized tooltip. `organizer` renders as a badge, not here. */
function AttendeeStatusIcon({ status, label }: { status: Exclude<AttendeeStatusKind, 'organizer'>; label: string }) {
  const common = 'w-4 h-4 flex-shrink-0 select-none';
  switch (status) {
    case 'accepted':
      return (
        <span title={label} aria-label={label} role="img">
          <svg className={`${common} text-green-600`} viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z"
              clipRule="evenodd"
            />
          </svg>
        </span>
      );
    case 'declined':
      return (
        <span title={label} aria-label={label} role="img">
          <svg className={`${common} text-red-600`} viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z"
              clipRule="evenodd"
            />
          </svg>
        </span>
      );
    case 'tentative':
      return (
        <span title={label} aria-label={label} role="img">
          <svg className={`${common} text-amber-500`} viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zM8.94 6.94a1.5 1.5 0 112.598 1.06c-.317.317-.858.55-1.288.775-.62.325-1.25.789-1.25 1.725v.25a.75.75 0 001.5 0c0-.235.093-.362.446-.567.086-.05.187-.103.297-.16.454-.238 1.1-.577 1.558-1.035a3 3 0 10-5.196-2.121.75.75 0 001.335.073zM10 15a1 1 0 100-2 1 1 0 000 2z"
              clipRule="evenodd"
            />
          </svg>
        </span>
      );
    default:
      return (
        <span title={label} aria-label={label} role="img">
          <svg className={`${common} text-gray-400`} viewBox="0 0 20 20" fill="none">
            <circle cx="10" cy="10" r="6" stroke="currentColor" strokeWidth="2" />
          </svg>
        </span>
      );
  }
}

function DetailRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1">{label}</div>
      {children}
    </div>
  );
}
