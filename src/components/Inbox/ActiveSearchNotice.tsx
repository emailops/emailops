import { useTranslation } from 'react-i18next';

interface ActiveSearchNoticeProps {
  query: string;
  onClear: () => void;
}

/**
 * Shown over an empty list while a search (or the smart filter it names) is
 * active. The query carries over account switches, so an empty list alone
 * would read as an empty mailbox.
 */
export function ActiveSearchNotice({ query, onClear }: ActiveSearchNoticeProps) {
  const { t } = useTranslation('inbox');
  return (
    <div className="mx-4 mt-3 flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
      <span className="min-w-0 flex-1 truncate">{t('header.search', { query })}</span>
      <button
        type="button"
        onClick={onClear}
        className="flex-shrink-0 rounded px-2 py-1 font-medium text-amber-900 hover:bg-amber-100"
      >
        {t('header.clearSearch')}
      </button>
    </div>
  );
}
