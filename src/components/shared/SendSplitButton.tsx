import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import { scheduleSendPresets } from '@/lib/outbox';
import { parseCustomSnooze, toDatetimeLocalValue, toUnixSeconds } from '@/lib/snooze';

interface SendSplitButtonProps {
  /** Text of the main button (Send, Send Reply, Forward, Sending…). */
  label: string;
  onSend: () => void;
  /** A schedule-send choice: a preset or a custom time in the future. */
  onSchedule: (at: Date) => void;
  disabled: boolean;
  /** Prefix for the data-testids (`<id>`, `<id>-schedule`, …). */
  testId: string;
}

const itemClass = 'w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2';

/**
 * The composer's Send control: the Send button plus a chevron opening
 * "Schedule send" — tomorrow morning / afternoon, Monday morning (local time,
 * `scheduleSendPresets`) or a custom date & time in the future — with a note
 * that the app must be open then. Shared by every composer.
 */
export function SendSplitButton({ label, onSend, onSchedule, disabled, testId }: SendSplitButtonProps) {
  const { t } = useTranslation(['compose']);
  const fmt = useFormatters();
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(() => new Date());
  const [customOpen, setCustomOpen] = useState(false);
  const [custom, setCustom] = useState('');
  const [error, setError] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);

  const toggle = () => {
    if (!open) {
      setNow(new Date());
      setCustomOpen(false);
      setCustom('');
      setError(false);
    }
    setOpen(!open);
  };

  const pick = (at: Date) => {
    setOpen(false);
    onSchedule(at);
  };

  const confirmCustom = () => {
    // Re-read the clock: the menu may have stayed open for a while.
    const at = parseCustomSnooze(custom, new Date());
    if (!at) {
      setError(true);
      return;
    }
    pick(at);
  };

  return (
    <div ref={ref} className="relative flex">
      <button
        type="button"
        data-testid={testId}
        onClick={onSend}
        disabled={disabled}
        className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-l-lg hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {label}
      </button>
      <button
        type="button"
        data-testid={`${testId}-schedule`}
        onClick={toggle}
        disabled={disabled}
        aria-label={t('compose:schedule.menuLabel')}
        title={t('compose:schedule.menuLabel')}
        aria-expanded={open}
        className="px-2 py-2 text-white bg-primary-600 rounded-r-lg border-l border-primary-700 hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
      >
        <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      {open && (
        <div
          data-testid={`${testId}-schedule-menu`}
          className="absolute right-0 bottom-full mb-1 w-72 bg-white rounded-lg shadow-lg border border-gray-200 py-1 z-50"
        >
          <div className="px-3 py-1.5 text-xs font-medium text-gray-500">{t('compose:schedule.scheduleSend')}</div>
          {scheduleSendPresets(now).map((preset) => (
            <button
              key={preset.id}
              type="button"
              data-testid={`${testId}-preset-${preset.id}`}
              onClick={() => pick(preset.at)}
              className={`${itemClass} justify-between`}
            >
              <span>{t(`compose:schedule.${preset.id}`)}</span>
              <span className="text-xs text-gray-400 whitespace-nowrap">
                {fmt.dateTime(toUnixSeconds(preset.at), { weekday: 'short', hour: '2-digit', minute: '2-digit' })}
              </span>
            </button>
          ))}
          <div className="border-t border-gray-100 my-1" />
          {customOpen ? (
            <div className="px-3 py-2 space-y-2">
              <label className="block text-xs text-gray-500">
                {t('compose:schedule.customLabel')}
                <input
                  type="datetime-local"
                  data-testid={`${testId}-custom-input`}
                  className="mt-1 w-full border border-gray-300 rounded px-2 py-1 text-sm text-gray-800"
                  min={toDatetimeLocalValue(now)}
                  value={custom}
                  onChange={(e) => {
                    setCustom(e.target.value);
                    setError(false);
                  }}
                />
              </label>
              {error && (
                <p data-testid={`${testId}-custom-error`} className="text-xs text-red-600">
                  {t('compose:schedule.invalid')}
                </p>
              )}
              <button
                type="button"
                data-testid={`${testId}-custom-confirm`}
                onClick={confirmCustom}
                className="w-full px-2 py-1 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded"
              >
                {t('compose:schedule.confirm')}
              </button>
            </div>
          ) : (
            <button
              type="button"
              data-testid={`${testId}-custom`}
              onClick={() => setCustomOpen(true)}
              className={itemClass}
            >
              {t('compose:schedule.custom')}
            </button>
          )}
          <p className="px-3 pt-1 pb-2 text-[11px] leading-snug text-gray-400">{t('compose:schedule.appOpenNote')}</p>
        </div>
      )}
    </div>
  );
}
