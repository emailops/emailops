import { type ReactNode, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { DangerousAttachmentDialog } from '@/components/shared/DangerousAttachmentDialog';
import { errorText, isAppErrorPayload } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

/** One of the `api.open…` calls, given whether the user has confirmed. */
type OpenCall = (confirmed: boolean) => Promise<void>;

interface PendingOpen {
  filename: string;
  kind: string;
  open: OpenCall;
}

/**
 * Open an attachment with the default app, asking first when the backend says
 * its type can run code (`attachment_confirmation_required`). Render
 * `confirmDialog` next to the control that opens.
 *
 * `openAttachment` rejects with any other failure of the first call, so the
 * caller reports it as before. A failure after the user confirmed is shown in
 * the dialog and logged.
 */
export function useOpenAttachment(): {
  openAttachment: (open: OpenCall) => Promise<void>;
  confirmDialog: ReactNode;
} {
  const { t } = useTranslation(['attachments']);
  const addLog = useLogStore((s) => s.addLog);
  const [pending, setPending] = useState<PendingOpen | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isOpening, setIsOpening] = useState(false);

  const openAttachment = async (open: OpenCall) => {
    try {
      await open(false);
    } catch (err) {
      if (!isAppErrorPayload(err) || err.code !== 'attachment_confirmation_required') throw err;
      setError(null);
      setPending({ filename: err.params?.filename ?? '', kind: err.params?.kind ?? '', open });
    }
  };

  const confirm = async () => {
    if (!pending) return;
    setIsOpening(true);
    try {
      await pending.open(true);
      setPending(null);
    } catch (err) {
      const message = t('attachments:openConfirm.failed', { filename: pending.filename, error: errorText(err) });
      setError(message);
      addLog('error', 'system', message);
    } finally {
      setIsOpening(false);
    }
  };

  const confirmDialog = pending ? (
    <DangerousAttachmentDialog
      filename={pending.filename}
      kind={pending.kind}
      error={error}
      isOpening={isOpening}
      onConfirm={() => void confirm()}
      onCancel={() => setPending(null)}
    />
  ) : null;

  return { openAttachment, confirmDialog };
}
