import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import { errorText } from '@/lib/errors';

const PRIMARY =
  'rounded bg-primary-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-primary-700 disabled:opacity-50';
const SECONDARY = 'rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700';

/** Pure: the addresses typed in the recipients box (commas, semicolons or new lines). */
export function parseRecipients(text: string): string[] {
  return text
    .split(/[,;\n]/)
    .map((a) => a.trim())
    .filter(Boolean);
}

interface ShareDialogProps {
  title: string;
  /** The account the document's mail goes out from. */
  fromAddress: string;
  onShare: (recipients: string[]) => Promise<void>;
  onClose: () => void;
}

/**
 * Sharing is the consent to automatic mail: the dialog names the recipients
 * and the sending account, and Share stays disabled until the user ticks that
 * they understand their changes will be emailed to those addresses.
 */
export function ShareDialog({ title, fromAddress, onShare, onClose }: ShareDialogProps) {
  const { t } = useTranslation(['documents']);
  const [text, setText] = useState('');
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const recipients = parseRecipients(text);

  const handleShare = async () => {
    setBusy(true);
    setError(null);
    try {
      await onShare(recipients);
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
      size="md"
      title={t('documents:shareDialog.title', { title })}
      // The error sits in the fixed header, never below the fold.
      subtitle={
        error ? (
          <span data-testid="share-error" className="text-red-300">
            {error}
          </span>
        ) : (
          t('documents:shareDialog.from', { address: fromAddress })
        )
      }
      footer={
        <>
          <button type="button" onClick={onClose} className={SECONDARY}>
            {t('documents:cancel')}
          </button>
          <button
            type="button"
            data-testid="share-submit"
            disabled={busy || !consent || recipients.length === 0}
            onClick={() => void handleShare()}
            className={PRIMARY}
          >
            {busy ? t('documents:shareDialog.sharing') : t('documents:shareDialog.submit')}
          </button>
        </>
      }
    >
      <label className="block text-sm font-medium text-gray-300" htmlFor="share-recipients">
        {t('documents:shareDialog.recipients')}
      </label>
      <textarea
        id="share-recipients"
        data-testid="share-recipients"
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={2}
        className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1 text-sm text-gray-200"
      />
      <p className="mt-1 text-xs text-gray-500">{t('documents:shareDialog.recipientsHint')}</p>
      <label className="mt-4 flex items-start gap-2 text-sm text-gray-300">
        <input
          type="checkbox"
          data-testid="share-consent"
          checked={consent}
          onChange={(e) => setConsent(e.target.checked)}
          className="mt-0.5"
        />
        <span>{t('documents:shareDialog.consent')}</span>
      </label>
    </Modal>
  );
}
