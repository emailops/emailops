import { type ReactNode, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * Generic dark-themed modal scaffold used across settings/dialogs/confirms.
 *
 * Centralizes overlay + size + a11y wiring (Escape to close, click-outside to
 * close, header/body/footer slots) so callers stop duplicating the same
 * `fixed inset-0 z-50 flex items-center justify-center bg-black/...` shell.
 *
 * Use the dedicated slots for clearer call-sites:
 *   - `header` is rendered in a sticky title bar with a close button
 *   - `footer` is rendered in a sticky button row at the bottom
 *   - `children` is the scrollable body
 *
 * Pass `unstyledBody` to opt out of the default body padding (useful when the
 * caller renders its own grid/columns).
 */

type ModalSize = 'sm' | 'md' | 'lg' | 'xl' | '2xl' | '5xl';

const SIZE_CLASS: Record<ModalSize, string> = {
  sm: 'max-w-sm',
  md: 'max-w-md',
  lg: 'max-w-lg',
  xl: 'max-w-xl',
  '2xl': 'max-w-2xl',
  '5xl': 'max-w-5xl',
};

export interface ModalProps {
  open: boolean;
  onClose: () => void;
  /** Title shown in the sticky header. Pass `null` to omit the header entirely. */
  title?: ReactNode;
  /** Optional sub-text under the title. */
  subtitle?: ReactNode;
  /** Footer slot (typically a button row). Omitted when undefined. */
  footer?: ReactNode;
  size?: ModalSize;
  /** Disables click-outside-to-close. Use for destructive flows where a stray
   * click shouldn't dismiss. */
  disableBackdropClose?: boolean;
  /** Disables the Escape-to-close shortcut. */
  disableEscape?: boolean;
  /** Skip the default body padding. */
  unstyledBody?: boolean;
  /** Body content. */
  children: ReactNode;
  /** Optional class on the body (in addition to default padding). */
  bodyClassName?: string;
  /** z-index for stacking; defaults to 50. Pass 60 to overlay another modal. */
  zIndex?: number;
}

export function Modal({
  open,
  onClose,
  title,
  subtitle,
  footer,
  size = 'lg',
  disableBackdropClose,
  disableEscape,
  unstyledBody,
  children,
  bodyClassName,
  zIndex = 50,
}: ModalProps) {
  const { t } = useTranslation(['common']);
  // Escape-to-close. Bound to window so focused inputs don't swallow it.
  useEffect(() => {
    if (!open || disableEscape) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, disableEscape, onClose]);

  const handleBackdrop = useCallback(
    (e: React.MouseEvent) => {
      if (disableBackdropClose) return;
      if (e.target === e.currentTarget) onClose();
    },
    [disableBackdropClose, onClose],
  );

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 flex items-center justify-center bg-black/60 p-4"
      style={{ zIndex }}
      onClick={handleBackdrop}
      role="dialog"
      aria-modal="true"
    >
      <div
        className={`${SIZE_CLASS[size]} flex max-h-[90vh] w-full flex-col overflow-hidden rounded-lg border border-gray-700 bg-[#252526] shadow-2xl`}
      >
        {title !== undefined && (
          <div className="flex items-start justify-between gap-4 border-b border-gray-700 px-6 py-4">
            <div className="min-w-0 flex-1">
              {title && <h2 className="text-sm font-semibold text-gray-100">{title}</h2>}
              {subtitle && <p className="mt-0.5 text-xs text-gray-400">{subtitle}</p>}
            </div>
            <button
              onClick={onClose}
              className="rounded p-1 text-gray-400 transition-colors hover:bg-gray-700 hover:text-white"
              title={t('common:actions.close')}
              aria-label={t('common:actions.closeDialog')}
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        )}

        <div className={`min-h-0 flex-1 overflow-y-auto ${unstyledBody ? '' : 'px-6 py-5'} ${bodyClassName ?? ''}`}>
          {children}
        </div>

        {footer && (
          <div className="flex items-center justify-end gap-3 border-t border-gray-700 px-6 py-4">{footer}</div>
        )}
      </div>
    </div>
  );
}
