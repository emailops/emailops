import { useTranslation } from 'react-i18next';
import type { SendWarning } from '@/lib/sendWarnings';

interface SendWarningBannerProps {
  warnings: SendWarning[];
  onSendAnyway: () => void;
  onReview: () => void;
}

/**
 * Inline confirmation shown instead of sending when the message looks
 * unfinished (see `findSendWarnings`). Inline rather than a browser
 * `confirm()`, which blocks the webview.
 */
export function SendWarningBanner({ warnings, onSendAnyway, onReview }: SendWarningBannerProps) {
  const { t } = useTranslation(['compose']);
  const placeholders = warnings.flatMap((w) => (w.kind === 'unfilledPlaceholder' ? [w.text] : []));
  const missingAttachment = warnings.some((w) => w.kind === 'missingAttachment');

  return (
    <div role="alert" className="mb-2 rounded border border-amber-300 bg-amber-50 p-2 text-sm text-amber-800">
      <ul className="list-disc pl-5">
        {missingAttachment && <li>{t('compose:sendWarnings.missingAttachment')}</li>}
        {placeholders.length > 0 && (
          <li>{t('compose:sendWarnings.unfilledPlaceholder', { items: placeholders.join(', ') })}</li>
        )}
      </ul>
      <div className="mt-2 flex justify-end gap-2">
        <button
          type="button"
          onClick={onReview}
          className="rounded border border-amber-300 bg-white px-3 py-1 text-sm hover:bg-amber-100"
        >
          {t('compose:sendWarnings.review')}
        </button>
        <button
          type="button"
          onClick={onSendAnyway}
          className="rounded bg-amber-600 px-3 py-1 text-sm font-medium text-white hover:bg-amber-700"
        >
          {t('compose:sendWarnings.sendAnyway')}
        </button>
      </div>
    </div>
  );
}
