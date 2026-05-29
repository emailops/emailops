import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Attachment } from '@/types';

interface AttachmentViewerProps {
  attachment: Attachment | null;
  onViewEmail?: (emailId: string) => void;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function AttachmentViewer({ attachment, onViewEmail }: AttachmentViewerProps) {
  const { t } = useTranslation(['attachments']);
  const fmt = useFormatters();
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!attachment) {
      setDataUrl(null);
      setError(null);
      return;
    }

    let cancelled = false;

    api
      .getAttachmentData(attachment.accountId, attachment.id)
      .then((base64) => {
        if (cancelled) return;
        // Use correct mime type for the data URL (some PDFs arrive as application/octet-stream)
        let mime = attachment.mimeType;
        if (attachment.filename.toLowerCase().endsWith('.pdf') && mime !== 'application/pdf') {
          mime = 'application/pdf';
        }
        setDataUrl(`data:${mime};base64,${base64}`);
        setError(null);
      })
      .catch((err) => {
        if (!cancelled) {
          setError(errorText(err));
          setDataUrl(null);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [attachment]);

  const handleOpenExternally = async () => {
    if (!attachment) return;
    try {
      await api.openAttachmentExternally(attachment.accountId, attachment.id);
    } catch (err) {
      console.error('Failed to open externally:', err);
    }
  };

  if (!attachment) {
    return (
      <div className="flex-1 flex items-center justify-center bg-gray-50">
        <div className="text-center">
          <svg className="w-16 h-16 text-gray-300 mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.5}
              d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
            />
          </svg>
          <p className="text-sm text-gray-500">{t('attachments:viewer.selectAttachment')}</p>
        </div>
      </div>
    );
  }

  const isPdf =
    attachment.mimeType === 'application/pdf' ||
    attachment.mimeType === 'application/x-pdf' ||
    attachment.filename.toLowerCase().endsWith('.pdf');
  const isImage = attachment.mimeType.startsWith('image/');
  const isHtml =
    attachment.mimeType === 'text/html' ||
    attachment.filename.toLowerCase().endsWith('.html') ||
    attachment.filename.toLowerCase().endsWith('.htm');

  return (
    <div className="flex-1 flex flex-col bg-white overflow-hidden">
      {/* Header */}
      <div className="px-6 py-4 border-b border-gray-200">
        <div className="flex items-center justify-between">
          <div className="min-w-0">
            <h3 className="text-lg font-semibold text-gray-900 truncate">{attachment.filename}</h3>
            <div className="flex items-center gap-3 mt-1 text-xs text-gray-500">
              <span>{formatFileSize(attachment.fileSize)}</span>
              <span>{attachment.mimeType}</span>
              <span>{t('attachments:viewer.fromSender', { email: attachment.senderEmail })}</span>
              <span>{fmt.dateTime(attachment.emailTimestamp)}</span>
            </div>
            {attachment.tags.length > 0 && (
              <div className="flex gap-1.5 mt-2">
                {attachment.tags.map((tag) => (
                  <span
                    key={tag}
                    className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-primary-100 text-primary-700"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            )}
          </div>
          <div className="flex items-center gap-2 flex-shrink-0">
            {onViewEmail && (
              <button
                onClick={() => onViewEmail(attachment.emailId)}
                className="flex items-center gap-2 px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors"
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                  />
                </svg>
                {t('attachments:viewer.viewEmail')}
              </button>
            )}
            <button
              onClick={handleOpenExternally}
              className="flex items-center gap-2 px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"
                />
              </svg>
              {t('attachments:viewer.openExternally')}
            </button>
          </div>
        </div>
      </div>

      {/* Preview */}
      <div className="flex-1 overflow-hidden">
        {error ? (
          <div className="flex items-center justify-center h-full">
            <p className="text-sm text-red-500">{t('attachments:viewer.previewLoadFailed', { error })}</p>
          </div>
        ) : !dataUrl ? (
          <div className="flex items-center justify-center h-full">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary-600" />
          </div>
        ) : isPdf ? (
          <object data={dataUrl} type="application/pdf" className="w-full h-full">
            <div className="flex flex-col items-center justify-center h-full">
              <p className="text-sm text-gray-500 mb-3">{t('attachments:viewer.pdfUnsupported')}</p>
              <button
                onClick={handleOpenExternally}
                className="px-4 py-2 text-sm font-medium text-primary-600 hover:text-primary-700 hover:bg-primary-50 rounded-lg transition-colors"
              >
                {t('attachments:viewer.openInDefaultApp')}
              </button>
            </div>
          </object>
        ) : isHtml ? (
          <iframe
            src={dataUrl}
            className="w-full h-full border-0"
            sandbox="allow-same-origin"
            title={attachment.filename}
          />
        ) : isImage ? (
          <div className="flex items-center justify-center h-full p-6 overflow-auto">
            <img src={dataUrl} alt={attachment.filename} className="max-w-full max-h-full object-contain" />
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center h-full">
            <svg className="w-16 h-16 text-gray-300 mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"
              />
            </svg>
            <p className="text-sm text-gray-500 mb-3">{t('attachments:viewer.previewUnavailable')}</p>
            <button
              onClick={handleOpenExternally}
              className="px-4 py-2 text-sm font-medium text-primary-600 hover:text-primary-700 hover:bg-primary-50 rounded-lg transition-colors"
            >
              {t('attachments:viewer.openInDefaultApp')}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
