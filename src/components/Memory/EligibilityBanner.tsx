import { useTranslation } from 'react-i18next';
import type { MemoryConfig } from '@/types';

/**
 * Surfaces which emails the extractor will actually consider, so users wondering
 * "why isn't this email producing facts?" can see the configured filters at a
 * glance and link out to MemorySettings to change them.
 *
 * The memory (fact) pipeline has no time window today — it scans the full
 * mailbox subject to category and tag filters. Task extraction has its own
 * backfill window, surfaced in the AI Tasks settings panel.
 */
export function EligibilityBanner({ cfg }: { cfg: MemoryConfig }) {
  const { t } = useTranslation(['memory']);
  return (
    <div className="px-6 py-2 bg-blue-50 border-b border-blue-100 text-xs text-blue-900">
      <div className="font-semibold mb-0.5">{t('memory:eligibility.title')}</div>
      <ul className="space-y-0.5 text-blue-800">
        <li>
          {t('memory:eligibility.categories')}{' '}
          {cfg.categories.length > 0 ? (
            <span className="font-mono">{cfg.categories.join(', ')}</span>
          ) : (
            <span className="italic">{t('memory:eligibility.all')}</span>
          )}
        </li>
        {cfg.extractFromSelfOnly && <li>{t('memory:eligibility.selfOnly')}</li>}
        {cfg.excludedSenders.length > 0 && (
          <li>
            {t('memory:eligibility.excludesSenders')}{' '}
            <span className="font-mono">{cfg.excludedSenders.join(', ')}</span>
          </li>
        )}
        {cfg.excludedTags.length > 0 && (
          <li>
            {t('memory:eligibility.excludesTags')} <span className="font-mono">{cfg.excludedTags.join(', ')}</span>
          </li>
        )}
      </ul>
      <div className="text-[11px] text-blue-700 mt-1">{t('memory:eligibility.changeHint')}</div>
    </div>
  );
}
