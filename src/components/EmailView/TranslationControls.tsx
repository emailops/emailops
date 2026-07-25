import { useTranslation } from 'react-i18next';
import { languageDisplayName } from '@/lib/language';
import { useTranslationStore } from '@/stores/translationStore';

interface TranslationControlsProps {
  emailId: string;
}

/**
 * Slim bar shown above an email body when its detected language differs from
 * the user's preferred language: offers Translate, then the original /
 * translation toggle. Errors are pinned here, right next to the button that
 * caused them — never at the bottom of the scroll container.
 */
export function TranslationControls({ emailId }: TranslationControlsProps) {
  const { t, i18n } = useTranslation(['inbox']);
  const detection = useTranslationStore((s) => s.detectedByEmail[emailId]);
  const hasTranslation = useTranslationStore((s) => emailId in s.translations);
  const showTranslated = useTranslationStore((s) => !!s.showTranslated[emailId]);
  const isTranslating = useTranslationStore((s) => emailId in s.pendingTranslate);
  const error = useTranslationStore((s) => s.errorByEmail[emailId] ?? null);

  if (!detection?.needsTranslation) return null;

  const sourceName = languageDisplayName(detection.language, i18n.language);

  return (
    <div className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-primary-100 bg-primary-50/50 px-3 py-2">
      <svg className="w-4 h-4 text-primary-500 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 5h12M9 3v2m1.048 9.5A18.022 18.022 0 016.412 9m6.088 9h7M11 21l5-10 5 10M12.751 5C11.783 10.77 8.07 15.61 3 18.129"
        />
      </svg>
      <span className="text-xs text-gray-600">{t('inbox:translation.detectedLabel', { language: sourceName })}</span>
      {hasTranslation ? (
        <button
          type="button"
          onClick={() => useTranslationStore.getState().toggle(emailId)}
          className="text-xs font-medium text-primary-600 hover:text-primary-700"
        >
          {showTranslated ? t('inbox:translation.showOriginal') : t('inbox:translation.showTranslation')}
        </button>
      ) : (
        <button
          type="button"
          onClick={() => void useTranslationStore.getState().translate(emailId)}
          disabled={isTranslating}
          className="inline-flex items-center gap-1.5 text-xs font-medium text-primary-600 hover:text-primary-700 disabled:opacity-60"
        >
          {isTranslating && <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-primary-600" />}
          {isTranslating ? t('inbox:translation.translating') : t('inbox:translation.translate')}
        </button>
      )}
      {error && <span className="basis-full text-xs text-red-600">{t('inbox:translation.failed', { error })}</span>}
    </div>
  );
}

interface TranslatedEmailBodyProps {
  text: string;
  targetLanguage: string;
  truncated: boolean;
}

/**
 * Plain-text rendering of an AI-translated body. No iframe sandbox needed —
 * the content is model-generated plain text, not provider HTML.
 */
export function TranslatedEmailBody({ text, targetLanguage, truncated }: TranslatedEmailBodyProps) {
  const { t } = useTranslation(['inbox']);
  return (
    <div className="rounded-lg border border-primary-100 bg-primary-50/30 p-4">
      <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-primary-600">
        {t('inbox:translation.aiTranslation', { language: targetLanguage })}
      </div>
      {truncated && <div className="mb-2 text-xs text-amber-600">{t('inbox:translation.truncatedNotice')}</div>}
      <div className="whitespace-pre-wrap break-words text-sm text-gray-800">{text}</div>
    </div>
  );
}
