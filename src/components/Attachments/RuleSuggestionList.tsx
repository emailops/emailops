import { useTranslation } from 'react-i18next';
import type { AttachmentRuleSuggestion } from '@/types';

interface RuleSuggestionListProps {
  suggestions: AttachmentRuleSuggestion[];
  onReview: (suggestion: AttachmentRuleSuggestion) => void;
  onDismiss: (suggestionId: string) => void;
}

/**
 * Candidate rules mined from recurring document attachments. Reviewing one
 * opens the regular rule form prefilled; nothing is created without the
 * user saving that form.
 */
export function RuleSuggestionList({ suggestions, onReview, onDismiss }: RuleSuggestionListProps) {
  const { t, i18n } = useTranslation(['attachments']);
  if (suggestions.length === 0) return null;

  const formatDate = (secs: number) =>
    new Date(secs * 1000).toLocaleDateString(i18n.language, { year: 'numeric', month: 'short' });

  return (
    <section className="space-y-2">
      <div>
        <h3 className="text-sm font-medium text-gray-900">{t('attachments:suggestions.title')}</h3>
        <p className="text-xs text-gray-500">{t('attachments:suggestions.subtitle')}</p>
      </div>
      {suggestions.map((s) => (
        <div key={s.id} className="border border-amber-200 bg-amber-50/40 rounded-lg p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 text-xs text-gray-500 space-y-0.5">
              <div className="text-sm font-medium text-gray-900">{s.name}</div>
              <div>
                {t('attachments:rules.rowSender')}{' '}
                <code className="bg-gray-100 px-1 py-0.5 rounded">{s.senderEmailPattern}</code>
              </div>
              {s.filenamePattern && (
                <div>
                  {t('attachments:rules.rowFilename')}{' '}
                  <code className="bg-gray-100 px-1 py-0.5 rounded">{s.filenamePattern}</code>
                </div>
              )}
              <div>
                {t('attachments:suggestions.recurrence', {
                  count: s.emailCount,
                  since: formatDate(s.firstSeen),
                  last: formatDate(s.lastSeen),
                })}
              </div>
              {s.sampleFilenames.length > 0 && (
                <div className="truncate" title={s.sampleFilenames.join(', ')}>
                  {t('attachments:suggestions.examples')} {s.sampleFilenames.join(', ')}
                </div>
              )}
            </div>
            <div className="flex flex-col items-end gap-1 flex-shrink-0">
              <button
                onClick={() => onReview(s)}
                className="px-3 py-1 text-xs font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors"
              >
                {t('attachments:suggestions.review')}
              </button>
              <button
                onClick={() => onDismiss(s.id)}
                className="px-3 py-1 text-xs font-medium text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg transition-colors"
              >
                {t('attachments:suggestions.dismiss')}
              </button>
            </div>
          </div>
        </div>
      ))}
    </section>
  );
}
