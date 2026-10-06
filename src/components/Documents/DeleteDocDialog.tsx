import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import { errorText } from '@/lib/errors';

const DANGER = 'rounded bg-red-700 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-600 disabled:opacity-50';
const SECONDARY = 'rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700';

interface DeleteDocDialogProps {
  title: string;
  /** How many other people share the document (0: only this account). */
  sharedWith: number;
  onDelete: () => Promise<void>;
  onClose: () => void;
}

/** Asks before a document is deleted from this install, and says what that
 *  means for the people it is shared with. */
export function DeleteDocDialog({ title, sharedWith, onDelete, onClose }: DeleteDocDialogProps) {
  const { t } = useTranslation(['documents']);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleDelete = async () => {
    setBusy(true);
    setError(null);
    try {
      await onDelete();
      onClose();
    } catch (err) {
      setError(errorText(err));
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={onClose}
      size="sm"
      title={t('documents:deleteDialog.title', { title })}
      // The error sits in the fixed header, never below the fold.
      subtitle={
        error ? (
          <span data-testid="delete-doc-error" className="text-red-300">
            {error}
          </span>
        ) : undefined
      }
      footer={
        <>
          <button type="button" onClick={onClose} className={SECONDARY}>
            {t('documents:cancel')}
          </button>
          <button
            type="button"
            data-testid="delete-doc-confirm"
            disabled={busy}
            onClick={() => void handleDelete()}
            className={DANGER}
          >
            {t('documents:deleteDialog.confirm')}
          </button>
        </>
      }
    >
      <p className="text-sm text-gray-300">{t('documents:deleteDialog.body')}</p>
      {sharedWith > 0 && (
        <p className="mt-2 text-sm text-gray-300">{t('documents:deleteDialog.shared', { count: sharedWith })}</p>
      )}
    </Modal>
  );
}
