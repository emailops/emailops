import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import { formatBytes } from '@/lib/intl';
import type { DataMoveState } from './dataMoveFlow';

interface DataMoveDialogProps {
  state: DataMoveState;
  onConfirm: () => void;
  onCancel: () => void;
}

/** Above the onboarding wizard (z-100), so the first-run flow can use it too. */
const Z_ABOVE_ONBOARDING = 110;

export function DataMoveDialog({ state, onConfirm, onCancel }: DataMoveDialogProps) {
  const { t, i18n } = useTranslation(['common', 'settings']);
  if (state.phase === 'idle') return null;
  const moving = state.phase === 'moving';
  const progress = moving ? state.progress : null;
  const percent =
    progress && progress.totalBytes > 0 ? Math.floor((progress.copiedBytes / progress.totalBytes) * 100) : 0;

  return (
    <Modal
      open
      onClose={onCancel}
      size="md"
      zIndex={Z_ABOVE_ONBOARDING}
      disableBackdropClose
      disableEscape={moving}
      title={t('settings:privacy.dataLocation.confirmTitle')}
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={moving}
            className="px-3 py-1.5 text-sm text-gray-700 hover:text-gray-900 hover:bg-gray-100 rounded transition-colors disabled:opacity-50"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            type="button"
            data-testid="data-move-confirm"
            onClick={onConfirm}
            disabled={moving}
            className="px-3 py-1.5 text-sm text-white rounded bg-primary-600 hover:bg-primary-500 transition-colors disabled:opacity-50"
          >
            {moving ? t('settings:privacy.dataLocation.moving') : t('settings:privacy.dataLocation.moveButton')}
          </button>
        </div>
      }
    >
      {state.error && (
        <div className="mb-3 p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm" role="alert">
          {state.error}
        </div>
      )}
      <p className="text-sm text-gray-700">
        {t('settings:privacy.dataLocation.confirmBody', { size: formatBytes(state.neededBytes, i18n.language) })}
      </p>
      <p className="mt-2 break-all rounded bg-gray-100 px-3 py-2 font-mono text-xs text-gray-800">{state.target}</p>
      <p className="mt-2 text-xs text-gray-500">{t('settings:privacy.dataLocation.confirmRestart')}</p>
      {moving && (
        <div className="mt-4">
          <div className="h-2 w-full overflow-hidden rounded bg-gray-200">
            <div className="h-full bg-primary-600 transition-all" style={{ width: `${percent}%` }} />
          </div>
          <p className="mt-1 text-xs text-gray-500">
            {progress
              ? t('settings:privacy.dataLocation.progress', {
                  copied: formatBytes(progress.copiedBytes, i18n.language),
                  total: formatBytes(progress.totalBytes, i18n.language),
                })
              : t('settings:privacy.dataLocation.starting')}
          </p>
        </div>
      )}
    </Modal>
  );
}
