import { open } from '@tauri-apps/plugin-shell';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';

interface ExternalLinkDialogProps {
  url: string;
  onClose: () => void;
}

/**
 * The one way out of the app for a link that came from someone else's content:
 * it shows the real address, which link text can hide, and opens it on request.
 */
export function ExternalLinkDialog({ url, onClose }: ExternalLinkDialogProps) {
  const { t } = useTranslation(['inbox', 'common']);
  const isTopOverlay = useOverlay(true);
  const cancelRef = useRef<HTMLButtonElement>(null);

  // Keys typed inside an email frame never reach the app, and the link that
  // opened this dialog can be in one.
  useEffect(() => {
    cancelRef.current?.focus();
  }, []);

  // On the document, as the app-wide handler on the window consumes the key.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || !isTopOverlay()) return;
      e.preventDefault();
      onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose, isTopOverlay]);

  const handleOpen = async () => {
    try {
      await open(url);
    } catch (err) {
      console.error('Failed to open URL:', err);
    }
    onClose();
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white rounded-xl shadow-xl p-6 max-w-md w-full mx-4">
        <h3 className="text-lg font-semibold text-gray-900">{t('inbox:emailView.openLinkTitle')}</h3>
        <p className="mt-2 text-sm text-gray-600 break-all">{url}</p>
        <div className="mt-4 flex justify-end gap-2">
          <button
            ref={cancelRef}
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            type="button"
            data-testid="external-link-open"
            onClick={handleOpen}
            className="px-4 py-2 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors"
          >
            {t('inbox:emailView.openInBrowser')}
          </button>
        </div>
      </div>
    </div>
  );
}
