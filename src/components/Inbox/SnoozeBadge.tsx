import { useTranslation } from 'react-i18next';
import { ClockIcon } from '@/components/common/MailIcons';
import { shortSnoozeTime } from '@/lib/snooze';
import { formatSnoozeTime, isBackFromSnooze, keyOfThread, useEmailStore } from '@/stores/emailStore';
import type { Email } from '@/types';

/**
 * Snooze state of a list row: "Snoozed until …" in the Snoozed view, and a
 * small "Snoozed" marker on a conversation that came back from snooze, until
 * it is read. Nothing otherwise.
 */
export function SnoozeBadge({ email }: { email: Email }) {
  const { t, i18n } = useTranslation(['inbox']);
  const snoozes = useEmailStore((s) => s.snoozes);
  const listScope = useEmailStore((s) => s.listScope);
  const record = snoozes.get(keyOfThread(email));
  if (!record) return null;
  if (listScope === 'snoozed' && record.wokeAt === null) {
    // Only the time on the chip — the view already says "snoozed" — and the
    // chip may shrink: the subject keeps priority in a narrow row.
    const full = t('inbox:snooze.until', { when: formatSnoozeTime(record.snoozedUntil) });
    return (
      <span
        data-testid="snooze-badge"
        title={full}
        className="inline-flex items-center gap-1 min-w-0 text-xs text-amber-700 bg-amber-50 rounded px-1.5 py-0.5 whitespace-nowrap"
      >
        <ClockIcon className="w-3 h-3 flex-shrink-0" />
        <span className="sr-only">{full}</span>
        <span className="truncate" aria-hidden="true">
          {shortSnoozeTime(new Date(record.snoozedUntil * 1000), new Date(), i18n.language || 'en')}
        </span>
      </span>
    );
  }
  if (isBackFromSnooze(snoozes, email)) {
    return (
      <span
        data-testid="snooze-badge"
        title={t('inbox:snooze.markerTitle')}
        className="inline-flex items-center gap-1 text-xs text-amber-700 bg-amber-50 rounded px-1.5 py-0.5 flex-shrink-0 whitespace-nowrap"
      >
        <ClockIcon className="w-3 h-3" />
        {t('inbox:snooze.marker')}
      </span>
    );
  }
  return null;
}
