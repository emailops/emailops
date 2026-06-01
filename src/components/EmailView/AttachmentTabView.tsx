import { useTranslation } from 'react-i18next';
import type { AttachmentViewTab } from '@/stores/emailStore';

interface AttachmentTabViewProps {
  tab: AttachmentViewTab;
  onClose: () => void;
}

export function AttachmentTabView({ tab, onClose }: AttachmentTabViewProps) {
  const { t } = useTranslation(['attachments', 'common']);
  const iframeSandbox = getAttachmentIframeSandbox(tab.mimeType);
  return (
    <div className="flex-1 flex flex-col overflow-hidden bg-white">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-200 flex-shrink-0 bg-gray-50">
        <AttachmentIcon mimeType={tab.mimeType} />
        <span className="text-sm font-medium text-gray-700 truncate flex-1">{tab.filename}</span>
        <button
          onClick={onClose}
          className="p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-200 rounded transition-colors flex-shrink-0"
          title={t('common:actions.close')}
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div className="flex-1 overflow-hidden">
        {tab.isLoading ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600 mx-auto" />
              <p className="mt-2 text-sm text-gray-500">Loading {tab.filename}…</p>
            </div>
          </div>
        ) : !tab.dataUrl ? (
          <div className="flex items-center justify-center h-full">
            <p className="text-sm text-gray-500">{t('attachments:viewer.loadFailed')}</p>
          </div>
        ) : tab.mimeType === 'text/plain' || tab.mimeType === 'text/markdown' ? (
          <pre className="w-full h-full overflow-auto p-6 text-sm text-gray-700 whitespace-pre-wrap font-mono leading-relaxed">
            {decodeBase64Utf8(tab.dataUrl)}
          </pre>
        ) : tab.mimeType === 'text/html' ? (
          <iframe
            src={tab.dataUrl}
            title={tab.filename}
            className="w-full h-full border-none"
            sandbox={iframeSandbox}
            referrerPolicy="no-referrer"
          />
        ) : (
          <iframe
            src={tab.dataUrl}
            title={tab.filename}
            className="w-full h-full border-none"
            sandbox={iframeSandbox}
            referrerPolicy="no-referrer"
          />
        )}
      </div>
    </div>
  );
}

export function getAttachmentIframeSandbox(mimeType: string): string | undefined {
  // HTML attachments are untrusted markup: sandbox them so scripts, forms,
  // popups, and same-origin access are all blocked (XSS prevention). An empty
  // sandbox value applies every restriction yet still renders static HTML.
  if (mimeType === 'text/html') return '';
  // Binary previews (PDF, etc.) are drawn by the WebView's built-in viewer,
  // which a restrictive sandbox blocks — leaving the tab blank. They carry no
  // scripts and load from an opaque-origin data: URI, so no sandbox is needed.
  return undefined;
}

function AttachmentIcon({ mimeType }: { mimeType: string }) {
  if (mimeType === 'application/pdf') {
    return (
      <svg className="w-4 h-4 flex-shrink-0 text-red-500" fill="currentColor" viewBox="0 0 20 20">
        <path
          fillRule="evenodd"
          d="M4 4a2 2 0 012-2h4.586A2 2 0 0112 2.586L15.414 6A2 2 0 0116 7.414V16a2 2 0 01-2 2H6a2 2 0 01-2-2V4zm2 6a1 1 0 011-1h6a1 1 0 110 2H7a1 1 0 01-1-1zm1 3a1 1 0 100 2h6a1 1 0 100-2H7z"
          clipRule="evenodd"
        />
      </svg>
    );
  }
  if (mimeType === 'text/html') {
    return (
      <svg className="w-4 h-4 flex-shrink-0 text-orange-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
      </svg>
    );
  }
  return (
    <svg className="w-4 h-4 flex-shrink-0 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
      />
    </svg>
  );
}

function decodeBase64Utf8(dataUrl: string): string {
  const base64 = dataUrl.split(',')[1] ?? '';
  try {
    const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    return new TextDecoder('utf-8').decode(bytes);
  } catch {
    return '';
  }
}
