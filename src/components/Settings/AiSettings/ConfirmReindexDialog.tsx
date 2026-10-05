// Confirmation dialog shown when Save would replace the email index: the
// embedding model changed, so every stored vector is deleted and rebuilt.

import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';
import type { AiConfigState } from './types';

interface ConfirmReindexDialogProps {
  /** The provider and embedding model about to be saved. */
  provider: AiConfigState['provider'];
  embeddingModel: string;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmReindexDialog({ provider, embeddingModel, onCancel, onConfirm }: ConfirmReindexDialogProps) {
  useOverlay();
  const { t } = useTranslation(['common', 'settings']);
  const none = embeddingModel === '';
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60">
      <div className="bg-[#2d2d2e] border border-gray-600 rounded-lg p-6 shadow-xl max-w-md w-full mx-4">
        <h3 className="text-base font-semibold text-gray-100 mb-2">{t('settings:confirmReindex.title')}</h3>
        <p className="text-sm text-gray-300 mb-3">
          {none ? t('settings:confirmReindex.bodyNone') : t('settings:confirmReindex.body')}
        </p>
        {provider === 'openrouter' && !none && (
          <p className="text-sm text-amber-300 mb-3">{t('settings:confirmReindex.openRouter')}</p>
        )}
        <div className="flex gap-2 justify-end mt-5">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded transition-colors"
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
