// Full-screen viewer for a single image, opened by tapping one in a message.
//
// Deliberately edge to edge rather than a padded card: on a phone the padding
// is most of the screen, and a picture the user asked to see up close should
// get all of it. Only the close button is inset, far enough to clear the notch.

import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ZoomableImage } from './ZoomableImage';

export interface ImageLightboxProps {
  src: string;
  /** The image's alt text. Shown as a caption when the sender wrote one. */
  alt?: string;
  onClose: () => void;
}

export function ImageLightbox({ src, alt, onClose }: ImageLightboxProps) {
  const { t } = useTranslation(['common', 'attachments']);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [src]);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [onClose]);

  // Only a click that lands on the backdrop itself dismisses. A drag that pans
  // the image ends on the image (it holds the pointer capture), so a pan that
  // happens to finish over empty space does not close the viewer under it.
  const handleBackdrop = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.target === e.currentTarget) onClose();
    },
    [onClose],
  );

  return (
    <div
      // Modals own their drags: without this the phone's back-swipe would
      // compete with panning a zoomed image. See `swipeGesture.ts`.
      data-no-swipe
      role="dialog"
      aria-modal="true"
      aria-label={t('common:labels.imageViewer')}
      className="fixed inset-0 z-50 flex flex-col bg-black/95"
      onClick={handleBackdrop}
    >
      {/* A dark chip on a near-black backdrop is invisible; the button has to
          read against the picture *and* against the empty space beside it. */}
      <button
        type="button"
        onClick={onClose}
        aria-label={t('common:actions.close')}
        title={t('common:actions.close')}
        className="absolute z-10 rounded-full bg-white/15 p-2 text-white transition-colors hover:bg-white/30 top-[calc(env(safe-area-inset-top)+0.75rem)] right-[calc(env(safe-area-inset-right)+0.75rem)]"
      >
        <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>

      {failed ? (
        <p className="m-auto px-8 text-center text-sm text-white/70">{t('attachments:viewer.imageLoadFailed')}</p>
      ) : (
        <ZoomableImage src={src} alt={alt ?? ''} onError={() => setFailed(true)} className="flex-1 min-h-0" />
      )}

      {alt && !failed && (
        <p className="shrink-0 truncate px-6 pb-[calc(env(safe-area-inset-bottom)+0.75rem)] pt-3 text-center text-xs text-white/60">
          {alt}
        </p>
      )}
    </div>
  );
}
