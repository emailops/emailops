import { useEffect, useState } from 'react';
import * as api from '@/lib/api';
import type { EmailAttachmentMeta } from '@/types';

function isViewableAttachment(mimeType: string): boolean {
  return (
    mimeType.startsWith('image/') ||
    mimeType === 'application/pdf' ||
    mimeType === 'text/html' ||
    mimeType === 'text/plain' ||
    mimeType === 'text/markdown'
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function EmailAttachments({
  emailId,
  accountId,
  onOpenAttachment,
}: {
  emailId: string;
  accountId: string;
  onOpenAttachment: (meta: EmailAttachmentMeta) => void;
}) {
  const [metas, setMetas] = useState<EmailAttachmentMeta[]>([]);
  const [downloading, setDownloading] = useState<Set<string>>(new Set());

  useEffect(() => {
    api
      .getEmailAttachmentMetas(accountId, emailId)
      .then(setMetas)
      .catch(() => setMetas([]));
  }, [accountId, emailId]);

  if (metas.length === 0) return null;

  const handleClick = async (meta: EmailAttachmentMeta) => {
    if (isViewableAttachment(meta.mimeType)) {
      onOpenAttachment(meta);
      return;
    }
    // Non-viewable: fall back to download / open with OS
    if (downloading.has(meta.id)) return;
    setDownloading((prev) => new Set(prev).add(meta.id));
    try {
      if (meta.filePath) {
        await api.openEmailAttachmentMeta(accountId, meta.id);
      } else {
        const b64 = await api.fetchEmailAttachmentBytes(accountId, emailId, meta.providerAttachmentId);
        const link = document.createElement('a');
        link.href = `data:${meta.mimeType};base64,${b64}`;
        link.download = meta.filename;
        link.click();
      }
    } catch (err) {
      console.error('Failed to download attachment:', err);
    } finally {
      setDownloading((prev) => {
        const next = new Set(prev);
        next.delete(meta.id);
        return next;
      });
    }
  };

  return (
    <div className="mt-4 border-t border-gray-100 pt-3">
      <div className="text-xs font-medium text-gray-500 mb-2">Attachments ({metas.length})</div>
      <div className="flex flex-wrap gap-2">
        {metas.map((meta) => {
          const isLoading = downloading.has(meta.id);
          const isCached = !!meta.filePath;
          const isViewable = isViewableAttachment(meta.mimeType);
          const actionLabel = isViewable ? 'click to view' : isCached ? 'click to open' : 'click to download';
          return (
            <button
              key={meta.id}
              type="button"
              onClick={() => handleClick(meta)}
              disabled={isLoading}
              className="flex items-center gap-2 px-3 py-2 bg-gray-50 border border-gray-200 rounded-lg hover:bg-gray-100 transition-colors text-sm disabled:opacity-60"
              title={`${meta.filename} (${formatFileSize(meta.fileSize)}) — ${actionLabel}`}
            >
              {isLoading ? (
                <svg className="w-4 h-4 text-gray-400 animate-spin flex-shrink-0" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z" />
                </svg>
              ) : isViewable ? (
                <svg
                  className="w-4 h-4 text-primary-500 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                  />
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
                  />
                </svg>
              ) : isCached ? (
                <svg
                  className="w-4 h-4 text-green-500 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
              ) : (
                <svg
                  className="w-4 h-4 text-gray-400 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
                  />
                </svg>
              )}
              <span className="truncate max-w-[200px]">{meta.filename}</span>
              <span className="text-xs text-gray-400 flex-shrink-0">{formatFileSize(meta.fileSize)}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
