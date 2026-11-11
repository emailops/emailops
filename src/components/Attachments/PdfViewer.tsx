import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy } from 'pdfjs-dist/legacy/build/pdf.mjs';
import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { errorText } from '@/lib/errors';
import {
  canZoomIn,
  canZoomOut,
  effectiveScale,
  FIT_WIDTH,
  fitWidthScale,
  type PdfZoom,
  zoomIn,
  zoomKeyAction,
  zoomOut,
} from '@/lib/pdfZoom';
import { useLogStore } from '@/stores/logStore';

// The worker is a bundled same-origin script, which the CSP's script-src
// 'self' already allows.
GlobalWorkerOptions.workerSrc = workerUrl;

/** Horizontal padding of the page column (p-3 on both sides), left out of fit width. */
const PAGE_PADDING = 24;

interface PdfViewerProps {
  dataUrl: string;
  title: string;
}

/**
 * Renders a PDF attachment page by page onto canvases with pdf.js, instead of
 * the WebView's built-in viewer, so the zoom is ours and the same on every
 * platform. pdf.js runs no document script: that lives in its viewer's
 * scripting sandbox, which this does not load.
 */
export function PdfViewer({ dataUrl, title }: PdfViewerProps) {
  const { t } = useTranslation(['attachments']);
  const addLog = useLogStore((s) => s.addLog);
  const paneRef = useRef<HTMLDivElement>(null);
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [pageWidth, setPageWidth] = useState(0);
  const [paneWidth, setPaneWidth] = useState(0);
  const [zoom, setZoom] = useState<PdfZoom>(FIT_WIDTH);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const base64 = dataUrl.slice(dataUrl.indexOf(',') + 1);
    const data = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    const task = getDocument({ data });
    let cancelled = false;
    task.promise
      .then(async (loaded) => {
        const first = await loaded.getPage(1);
        if (cancelled) return;
        setPageWidth(first.getViewport({ scale: 1 }).width);
        setDoc(loaded);
      })
      .catch((err) => {
        if (cancelled) return;
        addLog('error', 'system', `Failed to render ${title}: ${errorText(err)}`);
        setError(errorText(err));
      });
    return () => {
      cancelled = true;
      void task.destroy();
    };
  }, [dataUrl, title, addLog]);

  useEffect(() => {
    const el = paneRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setPaneWidth(entry.contentRect.width));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const fitScale = fitWidthScale(pageWidth, paneWidth, PAGE_PADDING);
  const scale = effectiveScale(zoom, fitScale);

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const action = zoomKeyAction(e);
    if (!action) return;
    e.preventDefault();
    if (action === 'in') setZoom(zoomIn(zoom, fitScale));
    else if (action === 'out') setZoom(zoomOut(zoom, fitScale));
    else setZoom(FIT_WIDTH);
  };

  return (
    <div
      data-testid="pdf-viewer"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      className="flex h-full flex-col focus:outline-none"
    >
      <div className="flex flex-shrink-0 items-center justify-center gap-1 border-b border-gray-200 bg-gray-50 px-3 py-1">
        <ToolbarButton
          label={t('attachments:viewer.zoomOut')}
          disabled={!doc || !canZoomOut(zoom, fitScale)}
          onClick={() => setZoom(zoomOut(zoom, fitScale))}
          path="M20 12H4"
        />
        <span data-testid="pdf-zoom-level" className="w-12 text-center text-xs tabular-nums text-gray-600">
          {Math.round(scale * 100)}%
        </span>
        <ToolbarButton
          label={t('attachments:viewer.zoomIn')}
          disabled={!doc || !canZoomIn(zoom, fitScale)}
          onClick={() => setZoom(zoomIn(zoom, fitScale))}
          path="M12 4v16m8-8H4"
        />
        <ToolbarButton
          label={t('attachments:viewer.fitWidth')}
          disabled={!doc || zoom === FIT_WIDTH}
          onClick={() => setZoom(FIT_WIDTH)}
          path="M4 8V4h4M20 8V4h-4M4 16v4h4M20 16v4h-4"
        />
      </div>
      {/* The scroller is absolutely positioned so a zoomed page never feeds its
          width into the host layout: a flex ancestor without min-w-0 would
          otherwise grow to fit the canvas and push the whole view off-screen. */}
      <div ref={paneRef} className="relative flex-1">
        <div className="absolute inset-0 overflow-auto bg-gray-100 p-3">
          {error ? (
            <div className="flex h-full items-center justify-center">
              <p className="text-sm text-red-500">{t('attachments:viewer.previewLoadFailed', { error })}</p>
            </div>
          ) : !doc ? (
            <div className="flex h-full items-center justify-center">
              <div className="h-6 w-6 animate-spin rounded-full border-b-2 border-primary-600" />
            </div>
          ) : (
            <div className="flex w-max min-w-full flex-col items-center gap-3">
              {Array.from({ length: doc.numPages }, (_, i) => i + 1).map((pageNumber) => (
                <PdfPage key={pageNumber} doc={doc} pageNumber={pageNumber} scale={scale} title={title} />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

interface ToolbarButtonProps {
  label: string;
  disabled: boolean;
  onClick: () => void;
  path: string;
}

function ToolbarButton({ label, disabled, onClick, path }: ToolbarButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className="rounded p-1 text-gray-500 transition-colors hover:bg-gray-200 hover:text-gray-700 disabled:opacity-40 disabled:hover:bg-transparent"
    >
      <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={path} />
      </svg>
    </button>
  );
}

interface PdfPageProps {
  doc: PDFDocumentProxy;
  pageNumber: number;
  scale: number;
  title: string;
}

/** One page, re-rendered at device resolution whenever the scale changes so text stays sharp. */
function PdfPage({ doc, pageNumber, scale, title }: PdfPageProps) {
  const addLog = useLogStore((s) => s.addLog);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let cancelled = false;
    let cancelRender = () => {};
    doc
      .getPage(pageNumber)
      .then((page) => {
        const canvas = canvasRef.current;
        if (cancelled || !canvas) return;
        const viewport = page.getViewport({ scale });
        const ratio = window.devicePixelRatio || 1;
        canvas.width = Math.floor(viewport.width * ratio);
        canvas.height = Math.floor(viewport.height * ratio);
        canvas.style.width = `${Math.floor(viewport.width)}px`;
        canvas.style.height = `${Math.floor(viewport.height)}px`;
        const task = page.render({
          canvas,
          viewport,
          transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0],
        });
        cancelRender = () => task.cancel();
        return task.promise;
      })
      .catch((err) => {
        // A zoom change mid-render cancels the stale pass; that is not a failure.
        if (cancelled || (err as Error)?.name === 'RenderingCancelledException') return;
        addLog('error', 'system', `Failed to render page ${pageNumber} of ${title}: ${errorText(err)}`);
      });
    return () => {
      cancelled = true;
      cancelRender();
    };
  }, [doc, pageNumber, scale, title, addLog]);

  return <canvas ref={canvasRef} className="bg-white shadow" />;
}
