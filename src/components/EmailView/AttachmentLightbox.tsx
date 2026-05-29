import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import type { EmailAttachmentMeta } from '@/types';

interface AttachmentLightboxProps {
  meta: EmailAttachmentMeta;
  onClose: () => void;
}

export function AttachmentLightbox({ meta, onClose }: AttachmentLightboxProps) {
  const { t } = useTranslation(['attachments', 'common']);
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    api
      .fetchEmailAttachmentBytes(meta.accountId, meta.emailId, meta.providerAttachmentId)
      .then((b64) => setDataUrl(`data:${meta.mimeType};base64,${b64}`))
      .catch(() => setError(true));
  }, [meta]);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 bg-black/85 flex items-center justify-center p-8" onClick={onClose}>
      <div className="relative flex flex-col items-center max-w-full max-h-full" onClick={(e) => e.stopPropagation()}>
        <button
          onClick={onClose}
          className="absolute -top-9 right-0 text-white/70 hover:text-white transition-colors"
          title={t('common:actions.close')}
        >
          <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>

        {error ? (
          <div className="text-white text-center p-8 bg-white/10 rounded-xl">
            <svg className="w-10 h-10 mx-auto mb-3 text-white/50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
            <p className="text-sm">{t('attachments:viewer.imageLoadFailed')}</p>
          </div>
        ) : dataUrl ? (
          <img
            src={dataUrl}
            alt={meta.filename}
            className="max-w-[90vw] max-h-[85vh] object-contain rounded-lg shadow-2xl"
          />
        ) : (
          <div className="flex items-center justify-center w-40 h-40">
            <div className="animate-spin rounded-full h-10 w-10 border-b-2 border-white" />
          </div>
        )}

        <p className="mt-3 text-white/60 text-xs truncate max-w-[90vw]">{meta.filename}</p>
      </div>
    </div>
  );
}
