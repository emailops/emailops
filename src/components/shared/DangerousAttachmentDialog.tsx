// Asked before an attachment that can run code or open another location is
// handed to the default app. The backend refuses such a file until the call
// carries the confirmation this dialog collects.

import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';

/** The kinds the backend reports (`DangerKind` in attachment_safety.rs). */
const KNOWN_KINDS = ['program', 'script', 'installer', 'shortcut', 'web_page'] as const;

interface DangerousAttachmentDialogProps {
  filename: string;
  /** Backend kind identifier; an unknown one gets the generic description. */
  kind: string;
  /** Why the confirmed open failed, shown pinned above the explanation. */
  error: string | null;
  isOpening: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function DangerousAttachmentDialog({
  filename,
  kind,
  error,
  isOpening,
  onConfirm,
  onCancel,
}: DangerousAttachmentDialogProps) {
  useOverlay();
  const { t } = useTranslation(['common', 'attachments']);
  const kindKey = KNOWN_KINDS.find((known) => known === kind) ?? 'other';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="open-attachment-title"
        className="bg-white rounded-xl shadow-xl p-6 max-w-md w-full mx-4"
      >
        <h3 id="open-attachment-title" className="text-lg font-semibold text-gray-900">
          {t('attachments:openConfirm.title')}
        </h3>
        {error && (
          <div role="alert" className="mt-3 p-3 bg-red-50 border border-red-200 rounded-lg text-sm text-red-700">
            {error}
          </div>
        )}
        <p className="mt-2 text-sm text-gray-600 break-words">{t('attachments:openConfirm.fromEmail', { filename })}</p>
        <p className="mt-2 text-sm text-gray-600">{t(`attachments:openConfirm.kinds.${kindKey}` as const)}</p>
        <p className="mt-2 text-sm text-gray-600">{t('attachments:openConfirm.advice')}</p>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            data-testid="cancel-open-attachment"
            onClick={onCancel}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            type="button"
            data-testid="confirm-open-attachment"
            onClick={onConfirm}
            disabled={isOpening}
            className="px-4 py-2 text-sm font-medium text-white bg-red-600 hover:bg-red-700 rounded-lg transition-colors disabled:opacity-50"
          >
            {t('attachments:openConfirm.open')}
          </button>
        </div>
      </div>
    </div>
  );
}
