import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AttachmentRuleSuggestion } from '@/types';

interface RuleSuggestionListProps {
  suggestions: AttachmentRuleSuggestion[];
  /** A re-mine is running. */
  loading: boolean;
  /** Suggestions dismissed earlier, most recent first. */
  dismissed: AttachmentRuleSuggestion[];
  onReview: (suggestion: AttachmentRuleSuggestion) => void;
  /** Resolves to whether the dismissal succeeded (failures are shown by the caller). */
  onDismiss: (suggestionId: string) => Promise<boolean>;
  /** Undo a dismissal; resolves to whether it succeeded. */
  onRestore: (suggestionId: string) => Promise<boolean>;
}

/**
 * Candidate rules mined from recurring document attachments. Reviewing one
 * opens the regular rule form prefilled; nothing is created without the
 * user saving that form. A dismissal can be undone right away, or later
 * from the list of dismissed suggestions.
 */
export function RuleSuggestionList({
  suggestions,
  loading,
  dismissed,
  onReview,
  onDismiss,
  onRestore,
}: RuleSuggestionListProps) {
  const { t, i18n } = useTranslation(['attachments']);
  // Requests in flight, per suggestion id: a second click must not send a second request.
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
  const [lastDismissed, setLastDismissed] = useState<{ id: string; name: string } | null>(null);
  const [showDismissed, setShowDismissed] = useState(false);
  if (suggestions.length === 0 && !loading && dismissed.length === 0 && !lastDismissed) return null;

  const whileBusy = async (id: string, run: () => Promise<boolean>) => {
    setBusy((prev) => new Set(prev).add(id));
    try {
      return await run();
    } finally {
      setBusy((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  };

  const dismiss = async (s: AttachmentRuleSuggestion) => {
    if (await whileBusy(s.id, () => onDismiss(s.id))) setLastDismissed({ id: s.id, name: s.name });
  };

  const restore = async (id: string) => {
    if ((await whileBusy(id, () => onRestore(id))) && lastDismissed?.id === id) setLastDismissed(null);
  };

  const formatDate = (secs: number) =>
    new Date(secs * 1000).toLocaleDateString(i18n.language, { year: 'numeric', month: 'short' });

  return (
    <section className="space-y-2" aria-busy={loading}>
      <div>
        <h3 className="text-sm font-medium text-gray-900">{t('attachments:suggestions.title')}</h3>
        <p className="text-xs text-gray-500">{t('attachments:suggestions.subtitle')}</p>
      </div>
      {loading && suggestions.length === 0 && (
        <p className="text-xs text-gray-500 flex items-center gap-2" role="status">
          <span className="w-3 h-3 animate-spin rounded-full border-2 border-gray-400 border-t-transparent" />
          {t('attachments:suggestions.searching')}
        </p>
      )}
      {lastDismissed && (
        <p className="text-xs text-gray-600 flex items-center gap-2" role="status">
          {t('attachments:suggestions.dismissedNotice', { name: lastDismissed.name })}
          <button
            type="button"
            onClick={() => void restore(lastDismissed.id)}
            disabled={busy.has(lastDismissed.id)}
            className="font-medium text-primary-600 hover:text-primary-700 disabled:opacity-50"
          >
            {t('attachments:suggestions.undo')}
          </button>
        </p>
      )}
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
                  <code className="bg-gray-100 px-1 py-0.5 rounded">{s.filenamePattern}</code>{' '}
                  <span className="text-gray-400">{t('attachments:suggestions.patternHelp')}</span>
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
                onClick={() => void dismiss(s)}
                disabled={busy.has(s.id)}
                className="px-3 py-1 text-xs font-medium text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg transition-colors disabled:opacity-50"
              >
                {t('attachments:suggestions.dismiss')}
              </button>
            </div>
          </div>
        </div>
      ))}
      {dismissed.length > 0 && (
        <div className="pt-1">
          <button
            type="button"
            aria-expanded={showDismissed}
            onClick={() => setShowDismissed((v) => !v)}
            className="text-xs text-gray-500 hover:text-gray-700"
          >
            {t('attachments:suggestions.showDismissed', { count: dismissed.length })}
          </button>
          {showDismissed && (
            <ul className="mt-1 space-y-1">
              {dismissed.map((d) => (
                <li key={d.id} className="flex items-center justify-between gap-3 text-xs text-gray-600">
                  <span className="truncate">
                    {d.name} <code className="bg-gray-100 px-1 py-0.5 rounded">{d.senderEmailPattern}</code>
                  </span>
                  <button
                    type="button"
                    onClick={() => void restore(d.id)}
                    disabled={busy.has(d.id)}
                    className="flex-shrink-0 font-medium text-primary-600 hover:text-primary-700 disabled:opacity-50"
                  >
                    {t('attachments:suggestions.restore')}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
