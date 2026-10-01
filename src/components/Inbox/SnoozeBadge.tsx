import { useTranslation } from 'react-i18next';
import { ClockIcon } from '@/components/common/MailIcons';
import { formatSnoozeTime, isBackFromSnooze, keyOfThread, useEmailStore } from '@/stores/emailStore';
import type { Email } from '@/types';

/**
 * Snooze state of a list row: "Snoozed until …" in the Snoozed view, and a
 * small "Snoozed" marker on a conversation that came back from snooze, until
 * it is read. Nothing otherwise.
 */
export function SnoozeBadge({ email }: { email: Email }) {
  const { t } = useTranslation(['inbox']);
  const snoozes = useEmailStore((s) => s.snoozes);
  const listScope = useEmailStore((s) => s.listScope);
  const record = snoozes.get(keyOfThread(email));
  if (!record) return null;
  if (listScope === 'snoozed' && record.wokeAt === null) {
    return (
      <span
        data-testid="snooze-badge"
        className="inline-flex items-center gap-1 text-xs text-amber-700 bg-amber-50 rounded px-1.5 py-0.5 flex-shrink-0 whitespace-nowrap"
      >
        <ClockIcon className="w-3 h-3" />
        {t('inbox:snooze.until', { when: formatSnoozeTime(record.snoozedUntil) })}
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
