// Confirmation dialog shown when Save would replace the email index: the
// embedding model changed, so every stored vector is deleted and rebuilt.

import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';

interface ConfirmReindexDialogProps {
  /** The embedding model about to be saved. */
  embeddingModel: string;
  /** OpenRouter computes the index with it: the OpenRouter tab, or an
   *  OpenAI-compatible server whose index OpenRouter computes. */
  sendsToOpenRouter: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmReindexDialog({
  embeddingModel,
  sendsToOpenRouter,
  onCancel,
  onConfirm,
}: ConfirmReindexDialogProps) {
  useOverlay();
  const { t } = useTranslation(['common', 'settings']);
  const none = embeddingModel === '';
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60">
      <div className="bg-white border border-gray-300 rounded-lg p-6 shadow-xl max-w-md w-full mx-4">
        <h3 className="text-base font-semibold text-gray-900 mb-2">{t('settings:confirmReindex.title')}</h3>
        <p className="text-sm text-gray-700 mb-3">
          {none ? t('settings:confirmReindex.bodyNone') : t('settings:confirmReindex.body')}
        </p>
        {sendsToOpenRouter && !none && (
          <p className="text-sm text-amber-700 mb-3">{t('settings:confirmReindex.openRouter')}</p>
        )}
        <div className="flex gap-2 justify-end mt-5">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-sm text-gray-700 hover:text-gray-900 hover:bg-gray-100 rounded transition-colors"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            onClick={onConfirm}
            className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-500 transition-colors"
          >
            {none ? t('settings:confirmReindex.confirmNone') : t('settings:confirmReindex.confirm')}
          </button>
        </div>
      </div>
    </div>
  );
}
