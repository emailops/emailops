// Confirmation dialog shown when the user clicks the AI master-toggle off.
// Warns about hidden features while reassuring that local data is preserved.

import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';

interface ConfirmDisableDialogProps {
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmDisableDialog({ onCancel, onConfirm }: ConfirmDisableDialogProps) {
  useOverlay();
  const { t } = useTranslation(['common', 'settings']);
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60">
      <div className="bg-white border border-gray-300 rounded-lg p-6 shadow-xl max-w-md w-full mx-4">
        <h3 className="text-base font-semibold text-gray-900 mb-2">{t('settings:confirmDisable.title')}</h3>
        <p className="text-sm text-gray-700 mb-3">{t('settings:confirmDisable.body')}</p>
        <p className="text-sm text-gray-500 mb-5">
          <span className="text-gray-800 font-medium">{t('settings:confirmDisable.preserved')}</span>{' '}
          {t('settings:confirmDisable.preservedDetail')}
        </p>
        <div className="flex gap-2 justify-end">
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
            {t('settings:confirmDisable.disable')}
          </button>
        </div>
      </div>
    </div>
  );
}
