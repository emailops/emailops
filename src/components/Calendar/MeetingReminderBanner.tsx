import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { startsIn } from '@/lib/calendarGrid';
import { getSafeExternalUrl } from '@/lib/emailFormatting';
import { useLogStore } from '@/stores/logStore';
import { useReminderStore } from '@/stores/reminderStore';

/** Auto-dismiss the banner ~5 minutes after it appeared. */
const AUTO_DISMISS_MS = 5 * 60 * 1000;
/** Re-evaluate the countdown / auto-dismiss conditions every 15 s. */
const TICK_MS = 15 * 1000;

/**
 * Dismissible top-of-window banner for the `meeting-reminder` backend event.
 * Rendered above all views in App.tsx; shows the meeting title, how soon it
 * starts, and a Join button when a meeting link exists. Auto-dismisses after
 * ~5 minutes or once the meeting start time has passed.
 */
export function MeetingReminderBanner() {
  const { t } = useTranslation(['calendar']);
  const { reminder, shownAtMs, dismiss } = useReminderStore();
  const addLog = useLogStore((s) => s.addLog);
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (!reminder) return;
    setNowMs(Date.now());
    const timer = setInterval(() => setNowMs(Date.now()), TICK_MS);
    return () => clearInterval(timer);
  }, [reminder]);

  if (!reminder || shownAtMs === null) return null;

  const nowSec = Math.floor(nowMs / 1000);
  if (nowSec >= reminder.startTime || nowMs - shownAtMs > AUTO_DISMISS_MS) {
    // Meeting already started or the banner outlived its usefulness. Defer the
    // store update out of render.
    queueMicrotask(dismiss);
    return null;
  }

  const eta = startsIn(reminder.startTime, nowSec);
  const etaLabel =
    eta.kind === 'minutes'
      ? t('calendar:reminder.startsInMinutes', { minutes: eta.minutes })
      : t('calendar:reminder.startsNow');

  const joinUrl = reminder.meetingLink ? getSafeExternalUrl(reminder.meetingLink) : null;

  const handleJoin = () => {
    if (!joinUrl) return;
    openExternal(joinUrl).catch((err) => {
      addLog('error', 'system', `Failed to open meeting link: ${err}`);
    });
  };

  return (
    <div role="alert" className="bg-primary-600 text-white px-4 py-2 flex items-center gap-3 text-sm shadow-md z-40">
      <svg className="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
        />
      </svg>
      <span className="min-w-0 truncate">
        <strong className="font-semibold">{reminder.title || t('calendar:reminder.title')}</strong>
        <span className="mx-2 opacity-80">·</span>
        <span>{etaLabel}</span>
      </span>
      <span className="flex-1" />
      {joinUrl && (
        <button
          onClick={handleJoin}
          className="flex-shrink-0 px-3 py-1 rounded bg-white text-primary-700 text-sm font-semibold hover:bg-primary-50 transition-colors"
        >
          {t('calendar:reminder.join')}
        </button>
      )}
      <button
        onClick={dismiss}
        title={t('calendar:reminder.dismiss')}
        className="flex-shrink-0 p-1 rounded hover:bg-primary-500 transition-colors"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
