// Shown before an OpenAI-compatible server on another machine is saved, and
// again when its host changes: email content will be sent there, and EmailOps
// cannot control what that server keeps or trains on. The backend refuses the
// save until this is confirmed for the host (`plan_server_settings`).

import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';

interface ServerDisclosureDialogProps {
  host: string;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ServerDisclosureDialog({ host, onCancel, onConfirm }: ServerDisclosureDialogProps) {
  useOverlay();
  const { t } = useTranslation(['common', 'settings']);
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60">
      <div className="bg-white border border-gray-300 rounded-lg p-6 shadow-xl max-w-md w-full mx-4">
        <h3 className="text-base font-semibold text-gray-900 mb-2">{t('settings:server.disclosureTitle')}</h3>
        <p className="text-sm text-gray-700 mb-3">{t('settings:server.disclosureBody', { host })}</p>
        <p className="text-sm text-gray-700 mb-3">{t('settings:server.disclosureRetention')}</p>
        <p className="text-xs text-gray-500">{t('settings:server.disclosureLocalHint')}</p>
        <div className="flex gap-2 justify-end mt-5">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-sm text-gray-700 hover:text-gray-900 hover:bg-gray-100 rounded transition-colors"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            onClick={onConfirm}
            className="px-3 py-1.5 text-sm bg-primary-600 text-white rounded hover:bg-primary-500 transition-colors"
          >
            {t('settings:server.disclosureConfirm')}
          </button>
        </div>
      </div>
    </div>
  );
}
