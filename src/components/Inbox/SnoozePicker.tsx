import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ClockIcon } from '@/components/common/MailIcons';
import { useFormatters } from '@/hooks/useFormatters';
import { parseCustomSnooze, snoozePresets, toDatetimeLocalValue, toUnixSeconds } from '@/lib/snooze';
import { useOverlay } from '@/stores/overlayStore';

interface SnoozeOptionsProps {
  /** The chosen wake time, in unix seconds. */
  onPick: (until: number) => void;
}

/**
 * The snooze choices as a menu body: the presets for the current time
 * (`snoozePresets`, in the user's time zone) and a custom date & time, which
 * must be in the future. Shared by the row ⋮ menu, the reading-pane toolbar
 * and the bulk toolbar.
 */
export function SnoozeOptions({ onPick }: SnoozeOptionsProps) {
  const { t } = useTranslation(['inbox']);
  const fmt = useFormatters();
  const [now] = useState(() => new Date());
  const [customOpen, setCustomOpen] = useState(false);
  const [custom, setCustom] = useState('');
  const [error, setError] = useState(false);
  const itemClass = 'w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-50 flex items-center gap-2';

  const confirmCustom = () => {
    // Re-read the clock: the menu may have stayed open for a while.
    const at = parseCustomSnooze(custom, new Date());
    if (!at) {
      setError(true);
      return;
    }
    onPick(toUnixSeconds(at));
  };

  return (
    <div data-testid="snooze-options">
      <div className="px-3 py-1.5 text-xs font-medium text-gray-500">{t('inbox:snooze.menuTitle')}</div>
      {snoozePresets(now).map((preset) => (
        <button
          key={preset.id}
          type="button"
          data-testid={`snooze-preset-${preset.id}`}
          onClick={(e) => {
            e.stopPropagation();
            onPick(toUnixSeconds(preset.at));
          }}
          className={`${itemClass} justify-between`}
        >
          <span>{t(`inbox:snooze.presets.${preset.id}`)}</span>
          <span className="text-xs text-gray-400 whitespace-nowrap">
            {fmt.dateTime(toUnixSeconds(preset.at), {
              weekday: 'short',
              hour: '2-digit',
              minute: '2-digit',
            })}
          </span>
        </button>
      ))}
      <div className="border-t border-gray-100 my-1" />
      {customOpen ? (
        <div className="px-3 py-2 space-y-2">
          <label className="block text-xs text-gray-500">
            {t('inbox:snooze.customLabel')}
            <input
              type="datetime-local"
              data-testid="snooze-custom-input"
              className="mt-1 w-full border border-gray-300 rounded px-2 py-1 text-sm text-gray-800"
              min={toDatetimeLocalValue(now)}
              value={custom}
              onClick={(e) => e.stopPropagation()}
              onChange={(e) => {
                setCustom(e.target.value);
                setError(false);
              }}
            />
          </label>
          {error && (
            <p data-testid="snooze-custom-error" className="text-xs text-red-600">
              {t('inbox:snooze.mustBeFuture')}
            </p>
          )}
          <button
            type="button"
            data-testid="snooze-custom-confirm"
            onClick={(e) => {
              e.stopPropagation();
              confirmCustom();
            }}
            className="w-full px-2 py-1 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded"
          >
            {t('inbox:snooze.confirm')}
          </button>
        </div>
      ) : (
        <button
          type="button"
          data-testid="snooze-custom"
          onClick={(e) => {
            e.stopPropagation();
            setCustomOpen(true);
          }}
          className={itemClass}
        >
          {t('inbox:snooze.pickDateTime')}
        </button>
      )}
    </div>
  );
}

interface SnoozeMenuButtonProps extends SnoozeOptionsProps {
  testId: string;
  /** Button classes; the toolbars style their buttons differently. */
  className?: string;
  /** Where the panel opens relative to the button. */
  align?: 'left' | 'right';
  /** Opens the panel each time it changes after mount (keyboard `b`). */
  openSignal?: number;
}

/** A clock button opening the snooze choices in a dropdown. */
export function SnoozeMenuButton({
  onPick,
  testId,
  className = 'p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors',
  align = 'left',
  openSignal = 0,
}: SnoozeMenuButtonProps) {
  const { t } = useTranslation(['inbox']);
  const [open, setOpen] = useState(false);
  useOverlay(open);
  const ref = useRef<HTMLDivElement>(null);
  const seenSignal = useRef(openSignal);
  useEffect(() => {
    if (openSignal === seenSignal.current) return;
    seenSignal.current = openSignal;
    setOpen(true);
  }, [openSignal]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    // The global shortcuts stand down while the picker is open, so Escape
    // has to be handled here.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', close);
    window.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', close);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        data-testid={testId}
        onClick={() => setOpen(!open)}
        className={className}
        title={t('inbox:snooze.button')}
        aria-label={t('inbox:snooze.button')}
        aria-expanded={open}
      >
        <ClockIcon className="w-4 h-4" />
      </button>
      {open && (
        <div
          className={`absolute ${align === 'right' ? 'right-0' : 'left-0'} top-full mt-1 w-64 bg-white rounded-lg shadow-lg border border-gray-200 py-1 z-50`}
        >
          <SnoozeOptions
            onPick={(until) => {
              setOpen(false);
              onPick(until);
            }}
          />
        </div>
      )}
    </div>
  );
}
