import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { useFormatters } from '@/hooks/useFormatters';
import type { RecipientSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import { type CalendarRecurrence, isValidInviteeEmail, recurrenceOptions } from '@/lib/calendarEvent';
import { extractEmail } from '@/lib/composeRecipients';
import { errorText, isAuthError } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { CalendarEvent } from '@/types';

/** Start/end pickers move in 30-minute steps. */
const STEP_MIN = 30;
const MINUTES_PER_DAY = 24 * 60;
/** Invitee autocomplete debounce. */
const SUGGEST_DEBOUNCE_MS = 150;

function pad2(n: number): string {
  return String(n).padStart(2, '0');
}

/** Local `YYYY-MM-DD` for an `<input type="date">`. */
function toDateInputValue(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/** Unix seconds at `minutes` past local midnight of `day` (may cross into the next day). */
function epochAt(day: Date, minutes: number): number {
  return Math.floor(new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes).getTime() / 1000);
}

interface NewEventDialogProps {
  accountId: string;
  /** Gmail accounts get a Google Meet link auto-added backend-side — show the note. */
  isGmail: boolean;
  /** Proposed slot, unix seconds (from the double-clicked grid position). */
  initialStart: number;
  initialEnd: number;
  onClose: () => void;
  /** Created on the provider — parent inserts it (or, for a recurring master,
   *  syncs so the expanded per-occurrence instances appear). */
  onCreated: (event: CalendarEvent, recurrence: CalendarRecurrence) => void;
  /** Auth-class failure — parent closes the dialog and shows its re-auth banner. */
  onAuthError: () => void;
}

/** "New event" dialog opened by double-clicking an empty calendar slot. */
export function NewEventDialog({
  accountId,
  isGmail,
  initialStart,
  initialEnd,
  onClose,
  onCreated,
  onAuthError,
}: NewEventDialogProps) {
  const { t, i18n } = useTranslation(['calendar', 'common']);
  const { time } = useFormatters();
  const addLog = useLogStore((s) => s.addLog);

  const initial = useMemo(() => {
    const s = new Date(initialStart * 1000);
    const startMin = s.getHours() * 60 + s.getMinutes();
    const durationMin = Math.max(Math.round((initialEnd - initialStart) / 60), STEP_MIN);
    return {
      day: new Date(s.getFullYear(), s.getMonth(), s.getDate()),
      startMin,
      endMin: Math.min(startMin + durationMin, MINUTES_PER_DAY),
    };
  }, [initialStart, initialEnd]);

  const [title, setTitle] = useState('');
  const [day, setDay] = useState<Date>(initial.day);
  const [startMin, setStartMin] = useState(initial.startMin);
  const [endMin, setEndMin] = useState(initial.endMin);
  const [description, setDescription] = useState('');
  const [recurrence, setRecurrence] = useState<CalendarRecurrence>('none');
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Invitee chip input + autocomplete state.
  const [invitees, setInvitees] = useState<string[]>([]);
  const [inviteeInput, setInviteeInput] = useState('');
  const [suggestions, setSuggestions] = useState<RecipientSuggestion[]>([]);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [inviteeFocused, setInviteeFocused] = useState(false);
  const [inviteeInvalid, setInviteeInvalid] = useState(false);
  const [inviteeShaking, setInviteeShaking] = useState(false);
  const suggestionsReqRef = useRef(0);

  // 00:00 … 23:30 starts; ends are strictly after the chosen start (up to 24:00).
  const startOptions = useMemo(() => Array.from({ length: MINUTES_PER_DAY / STEP_MIN }, (_, i) => i * STEP_MIN), []);
  const endOptions = useMemo(
    () => startOptions.map((m) => m + STEP_MIN).filter((m) => m > startMin),
    [startOptions, startMin],
  );

  const timeLabel = (minutes: number) => time(epochAt(day, minutes));

  /** Moving the start keeps the duration and always leaves end > start. */
  const handleStartChange = (newStart: number) => {
    const duration = Math.max(endMin - startMin, STEP_MIN);
    setStartMin(newStart);
    setEndMin(Math.min(newStart + duration, MINUTES_PER_DAY));
  };

  const handleDateChange = (value: string) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
    if (!m) return; // ignore in-progress typing / cleared input
    setDay(new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  };

  // Recurrence options are anchored to the chosen date's weekday ("Weekly on
  // Wednesday") and re-derive when the date or UI language changes.
  const recurrenceOpts = useMemo(() => recurrenceOptions(day, i18n.language || 'en'), [day, i18n.language]);

  // Debounced invitee autocomplete against the selected account's contacts.
  // Guarded against stale responses via the request id.
  useEffect(() => {
    const prefix = inviteeInput.trim();
    if (prefix.length < 2) {
      setSuggestions([]);
      return;
    }
    const reqId = ++suggestionsReqRef.current;
    const handle = window.setTimeout(() => {
      api
        .autocompleteRecipients(accountId, prefix, undefined, 8)
        .then((results) => {
          if (suggestionsReqRef.current !== reqId) return;
          const existing = new Set(invitees);
          setSuggestions(results.filter((r) => !existing.has(r.email.toLowerCase())));
          setSelectedIdx(0);
        })
        .catch((err) => {
          if (suggestionsReqRef.current !== reqId) return;
          setSuggestions([]);
          addLog('debug', 'system', `Invitee autocomplete failed: ${errorText(err)}`);
        });
    }, SUGGEST_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [inviteeInput, accountId, invitees, addLog]);

  /** Add a chip if `raw` has a valid email shape; report whether it did. */
  const addInvitee = (raw: string): boolean => {
    const email = extractEmail(raw);
    if (!isValidInviteeEmail(email)) return false;
    setInvitees((prev) => (prev.includes(email) ? prev : [...prev, email]));
    setInviteeInput('');
    setSuggestions([]);
    setInviteeInvalid(false);
    return true;
  };

  /** Tokenize the typed text (Enter/comma); invalid input marks red + shakes. */
  const commitInviteeInput = () => {
    if (inviteeInput.trim() === '') return;
    if (!addInvitee(inviteeInput)) {
      setInviteeInvalid(true);
      setInviteeShaking(true);
    }
  };

  const removeInvitee = (email: string) => {
    setInvitees((prev) => prev.filter((r) => r !== email));
  };

  const handleInviteeKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' && suggestions.length > 0) {
      e.preventDefault();
      setSelectedIdx((i) => Math.min(i + 1, suggestions.length - 1));
    } else if (e.key === 'ArrowUp' && suggestions.length > 0) {
      e.preventDefault();
      setSelectedIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      // Never submit the form from the chip input.
      e.preventDefault();
      if (suggestions.length > 0) addInvitee(suggestions[selectedIdx].email);
      else commitInviteeInput();
    } else if (e.key === ',') {
      e.preventDefault();
      commitInviteeInput();
    } else if (e.key === 'Escape' && suggestions.length > 0) {
      e.stopPropagation();
      setSuggestions([]);
    } else if (e.key === 'Backspace' && inviteeInput === '' && invitees.length > 0) {
      e.preventDefault();
      setInvitees((prev) => prev.slice(0, -1));
    }
  };

  const trimmedTitle = title.trim();

  const handleCreate = async () => {
    if (!trimmedTitle || isSaving) return;
    // A valid address still sitting in the input box (typed but not
    // tokenized) counts too, so it isn't silently dropped.
    const pending = extractEmail(inviteeInput);
    const attendees = isValidInviteeEmail(pending) && !invitees.includes(pending) ? [...invitees, pending] : invitees;
    setIsSaving(true);
    setError(null);
    try {
      const created = await api.createCalendarEvent(
        accountId,
        trimmedTitle,
        description.trim(),
        attendees,
        epochAt(day, startMin),
        epochAt(day, endMin),
        recurrence,
        Intl.DateTimeFormat().resolvedOptions().timeZone,
      );
      addLog('success', 'sync', 'Calendar event created');
      onCreated(created, recurrence);
    } catch (e) {
      const msg = errorText(e);
      addLog('error', 'sync', `Failed to create calendar event: ${msg}`);
      if (isAuthError(e, msg)) {
        onAuthError();
      } else {
        setError(msg);
      }
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-white border border-gray-200 rounded-lg w-full max-w-md max-h-[85vh] shadow-xl flex flex-col overflow-hidden mx-4 dark:bg-surface dark:border-gray-700">
        {/* Error banner — pinned at the very top of the dialog, never below the fields. */}
        {error && (
          <div className="flex-shrink-0 border-b border-red-200 bg-red-50 px-4 py-2 flex items-start gap-2 text-sm text-red-800 dark:border-red-800 dark:bg-red-900/20 dark:text-red-300">
            <svg className="w-4 h-4 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
              <path
                fillRule="evenodd"
                d="M10 18a8 8 0 100-16 8 8 0 000 16zM9 9a1 1 0 012 0v4a1 1 0 11-2 0V9zm1-5a1 1 0 100 2 1 1 0 000-2z"
                clipRule="evenodd"
              />
            </svg>
            <span className="min-w-0 flex-1 break-words">{t('calendar:create.error', { message: error })}</span>
          </div>
        )}

        <div className="flex items-center justify-between gap-3 px-5 pt-4 pb-3 border-b border-gray-100 flex-shrink-0 dark:border-gray-800">
          <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">{t('calendar:create.title')}</h3>
          <button
            onClick={onClose}
            title={t('common:actions.close')}
            className="flex-shrink-0 p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors dark:text-gray-500 dark:hover:text-gray-400 dark:hover:bg-surface-hover"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleCreate();
          }}
          className="px-5 py-4 space-y-4 overflow-y-auto"
        >
          <div>
            <label
              htmlFor="new-event-title"
              className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400"
            >
              {t('calendar:create.eventTitle')}
            </label>
            <input
              id="new-event-title"
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t('calendar:create.eventTitlePlaceholder')}
              // biome-ignore lint/a11y/noAutofocus: dialog opens explicitly for typing a title
              autoFocus
              required
              className="w-full border border-gray-300 rounded-md px-3 py-2 text-sm text-gray-900 focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-gray-600 dark:text-gray-100"
            />
          </div>

          <div>
            <label
              htmlFor="new-event-date"
              className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400"
            >
              {t('calendar:create.date')}
            </label>
            <input
              id="new-event-date"
              type="date"
              value={toDateInputValue(day)}
              onChange={(e) => handleDateChange(e.target.value)}
              className="w-full border border-gray-300 rounded-md px-3 py-2 text-sm text-gray-900 bg-white focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-gray-600 dark:text-gray-100 dark:bg-surface"
            />
          </div>

          <div className="flex gap-3">
            <div className="flex-1 min-w-0">
              <label className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400">
                {t('calendar:create.startTime')}
              </label>
              <Select
                value={String(startMin)}
                onChange={(value) => handleStartChange(Number(value))}
                options={startOptions.map((m) => ({ value: String(m), label: timeLabel(m) }))}
                ariaLabel={t('calendar:create.startTime')}
                fullWidth
                variant="light"
              />
            </div>
            <div className="flex-1 min-w-0">
              <label className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400">
                {t('calendar:create.endTime')}
              </label>
              <Select
                value={String(endMin)}
                onChange={(value) => setEndMin(Number(value))}
                options={endOptions.map((m) => ({ value: String(m), label: timeLabel(m) }))}
                ariaLabel={t('calendar:create.endTime')}
                fullWidth
                variant="light"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400">
              {t('calendar:create.recurrence.label')}
            </label>
            <Select
              value={recurrence}
              onChange={setRecurrence}
              options={recurrenceOpts.map((o) => ({
                value: o.value,
                label: t(`calendar:create.recurrence.${o.labelKey}`, o.params),
              }))}
              ariaLabel={t('calendar:create.recurrence.label')}
              fullWidth
              variant="light"
            />
          </div>

          <div>
            <label
              htmlFor="new-event-invitees"
              className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400"
            >
              {t('calendar:create.invitees')}
            </label>
            <div
              className={`flex flex-wrap gap-1 items-center border rounded-md px-2 py-1.5 bg-white min-h-[38px] focus-within:ring-1 focus-within:ring-primary-500 dark:bg-surface ${
                inviteeInvalid ? 'border-red-400' : 'border-gray-300 dark:border-gray-600'
              } ${inviteeShaking ? 'animate-shake' : ''}`}
              onAnimationEnd={() => setInviteeShaking(false)}
              onClick={() => document.getElementById('new-event-invitees')?.focus()}
            >
              {invitees.map((email) => (
                <span
                  key={email}
                  className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-700 dark:bg-surface-hover dark:text-gray-300"
                >
                  {email}
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      removeInvitee(email);
                    }}
                    className="hover:text-red-500"
                  >
                    ×
                  </button>
                </span>
              ))}
              <div className="relative flex-1 min-w-[140px]">
                <input
                  id="new-event-invitees"
                  type="text"
                  value={inviteeInput}
                  onChange={(e) => {
                    setInviteeInput(e.target.value);
                    setInviteeInvalid(false);
                  }}
                  onFocus={() => setInviteeFocused(true)}
                  onBlur={() => setTimeout(() => setInviteeFocused(false), 200)}
                  onKeyDown={handleInviteeKeyDown}
                  title={inviteeInvalid ? t('calendar:create.invalidEmail') : undefined}
                  className={`w-full text-sm outline-none bg-transparent py-0.5 ${
                    inviteeInvalid ? 'text-red-700 dark:text-red-300' : 'text-gray-900 dark:text-gray-100'
                  }`}
                  placeholder={invitees.length === 0 ? t('calendar:create.inviteesPlaceholder') : ''}
                />
                {inviteeFocused && suggestions.length > 0 && (
                  <div className="absolute top-full left-0 mt-1 w-72 bg-white border border-gray-200 rounded-lg shadow-lg z-50 max-h-48 overflow-y-auto dark:bg-surface dark:border-gray-700">
                    {suggestions.map((s, i) => (
                      <button
                        key={s.email}
                        type="button"
                        className={`w-full text-left px-3 py-2 text-sm hover:bg-gray-50 dark:hover:bg-surface-raised ${
                          i === selectedIdx ? 'bg-primary-50 dark:bg-primary-900/20' : ''
                        }`}
                        onMouseDown={(e) => {
                          e.preventDefault();
                          addInvitee(s.email);
                        }}
                      >
                        <div className="truncate text-gray-900 dark:text-gray-100">{s.email}</div>
                        {s.name && <div className="truncate text-xs text-gray-500 dark:text-gray-400">{s.name}</div>}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>

          <div>
            <label
              htmlFor="new-event-description"
              className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1 dark:text-gray-400"
            >
              {t('calendar:create.description')}
            </label>
            <textarea
              id="new-event-description"
              rows={3}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t('calendar:create.descriptionPlaceholder')}
              className="w-full border border-gray-300 rounded-md px-3 py-2 text-sm text-gray-900 focus:outline-none focus:ring-1 focus:ring-primary-500 resize-y dark:border-gray-600 dark:text-gray-100"
            />
          </div>

          {isGmail && (
            <p className="flex items-start gap-2 text-xs text-gray-500 dark:text-gray-400">
              <svg
                className="w-4 h-4 flex-shrink-0 text-gray-400 dark:text-gray-500"
                viewBox="0 0 20 20"
                fill="currentColor"
              >
                <path
                  fillRule="evenodd"
                  d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                  clipRule="evenodd"
                />
              </svg>
              {t('calendar:create.meetNote')}
            </p>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onClose}
              disabled={isSaving}
              className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 transition-colors disabled:opacity-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-surface-raised"
            >
              {t('common:actions.cancel')}
            </button>
            <button
              type="submit"
              disabled={isSaving || !trimmedTitle}
              className="px-4 py-1.5 text-sm rounded-md bg-primary-600 text-white font-medium hover:bg-primary-700 transition-colors disabled:opacity-50 flex items-center gap-1.5"
            >
              {isSaving && (
                <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
                </svg>
              )}
              {isSaving ? t('calendar:create.creating') : t('calendar:create.createButton')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
