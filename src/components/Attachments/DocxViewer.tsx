import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EmailBody } from '@/components/EmailView/EmailBody';
import { docxToPreviewHtml } from '@/lib/docxPreview';
import { dataUrlToBase64 } from '@/lib/download';
import { errorText } from '@/lib/errors';
import { bytesFromBase64 } from '@/lib/yjsBytes';
import { useLogStore } from '@/stores/logStore';

interface DocxViewerProps {
  /** The .docx as a `data:` URL. */
  dataUrl: string;
  title: string;
  accountId: string;
  senderEmail: string;
}

type Preview = { kind: 'loading' } | { kind: 'ready'; html: string } | { kind: 'failed'; error: string };

/**
 * A Word (.docx) attachment, read-only. The file is laid out as HTML
 * (`docxToPreviewHtml`) and shown through `EmailBody`, so it gets the email
 * frame's sanitizing, sandbox and remote-content consent. The page is never
 * inverted for the dark theme: a document keeps its own colours.
 */
export function DocxViewer({ dataUrl, title, accountId, senderEmail }: DocxViewerProps) {
  const { t } = useTranslation(['attachments']);
  const addLog = useLogStore((s) => s.addLog);
  const [preview, setPreview] = useState<Preview>({ kind: 'loading' });

  useEffect(() => {
    let cancelled = false;
    setPreview({ kind: 'loading' });
    const bytes = bytesFromBase64(dataUrlToBase64(dataUrl));
    docxToPreviewHtml(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length) as ArrayBuffer)
      .then((html) => {
        if (!cancelled) setPreview({ kind: 'ready', html });
      })
      .catch((err) => {
        if (cancelled) return;
        const error = errorText(err);
        setPreview({ kind: 'failed', error });
        addLog('error', 'system', `Failed to preview ${title}: ${error}`);
      });
    return () => {
      cancelled = true;
    };
  }, [dataUrl, title, addLog]);

  if (preview.kind === 'loading') {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary-600" />
      </div>
    );
  }
  if (preview.kind === 'failed') {
    return (
      <div className="flex items-center justify-center h-full">
        <p className="text-sm text-red-500">{t('attachments:viewer.previewLoadFailed', { error: preview.error })}</p>
      </div>
    );
  }
  return (
    <div className="h-full overflow-auto p-4">
      {/* Paper stays white on the dark theme, as in the PDF viewer. */}
      <EmailBody html={preview.html} accountId={accountId} senderEmail={senderEmail} colorScheme="light" />
    </div>
  );
}
