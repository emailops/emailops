import { type ReactNode, useCallback, useEffect, useId, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useOverlay } from '@/stores/overlayStore';

/**
 * Generic modal scaffold used across settings/dialogs/confirms.
 *
 * Centralizes overlay + size + a11y wiring (Escape to close, click-outside to
 * close, focus kept inside while open and handed back on close, a name from
 * the title, header/body/footer slots) so callers stop duplicating the same
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

/** What Tab can land on. `.hidden` is the composers' invisible file input. */
const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"]):not(.hidden)',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
  '[contenteditable="true"]',
].join(', ');

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
  /**
   * Leave the rest of the app visible and usable: no dim backdrop, no pointer
   * trap, and the dialog stays clear of the right-docked chat panel.
   *
   * Used when the chat is driving the dialog — filling a form from chat is a
   * conversation, so the panel has to stay on screen and accept the next
   * message ("no, esa columna es un importe"). A normal modal would cover it
   * and swallow every click.
   *
   * The clearance comes from the `--chat-dock-width` CSS variable that `App`
   * keeps in sync with the dock's persisted width, so the dialog never has to
   * know whether the panel is open or how wide the user dragged it.
   */
  nonBlocking?: boolean;
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
  nonBlocking = false,
}: ModalProps) {
  const isTopOverlay = useOverlay(open);
  const { t } = useTranslation(['common']);
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  // Read while rendering: a field with autoFocus takes the focus on commit,
  // before any effect here could see where it was.
  const openerRef = useRef<Element | null>(null);
  const wasOpenRef = useRef(false);
  if (open && !wasOpenRef.current) openerRef.current = document.activeElement;
  wasOpenRef.current = open;
  // Escape-to-close. Bound to the document so focused inputs don't swallow it,
  // and it runs before the app-wide handler on the window, which consumes every
  // Escape. Only the top overlay answers, and it marks the key as used for the
  // one below.
  useEffect(() => {
    if (!open || disableEscape) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || !isTopOverlay()) return;
      e.preventDefault();
      onClose();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, disableEscape, onClose, isTopOverlay]);

  // Focus moves in on open (unless a field already took it) and back to where
  // it was on close. A non-blocking dialog leaves it where it is: the chat
  // driving the dialog keeps the keyboard.
  useEffect(() => {
    if (!open || nonBlocking) return;
    if (!panelRef.current?.contains(document.activeElement)) panelRef.current?.focus();
    return () => {
      if (openerRef.current instanceof HTMLElement) openerRef.current.focus();
    };
  }, [open, nonBlocking]);

  const handleTab = (e: React.KeyboardEvent) => {
    if (e.key !== 'Tab' || nonBlocking || !isTopOverlay() || !panelRef.current) return;
    const focusable = [...panelRef.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (!first) {
      e.preventDefault();
    } else if (e.shiftKey && (active === first || active === panelRef.current)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const handleBackdrop = useCallback(
    (e: React.MouseEvent) => {
      // A non-blocking dialog has no backdrop to click: the pointer goes
      // through to whatever is behind it, which is the point.
      if (disableBackdropClose || nonBlocking) return;
      if (e.target === e.currentTarget) onClose();
    },
    [disableBackdropClose, nonBlocking, onClose],
  );

  if (!open) return null;

  return (
    <div
      data-testid="modal-overlay"
      className={
        nonBlocking
          ? 'pointer-events-none fixed inset-y-0 left-0 flex items-center justify-center p-4'
          : 'fixed inset-0 flex items-center justify-center bg-black/60 p-4'
      }
      // `right` clears the docked chat panel so it stays fully visible; 0 when
      // the panel is closed (the variable is unset) or the modal is blocking.
      style={nonBlocking ? { zIndex, right: 'var(--chat-dock-width, 0px)' } : { zIndex }}
      onClick={handleBackdrop}
      onKeyDown={handleTab}
      role="dialog"
      aria-modal={nonBlocking ? undefined : 'true'}
      aria-labelledby={title ? titleId : undefined}
    >
      <div
        ref={panelRef}
        tabIndex={-1}
        className={`theme-chrome ${SIZE_CLASS[size]} flex max-h-[90vh] w-full flex-col overflow-hidden rounded-lg border border-gray-200 bg-white shadow-2xl focus:outline-none${
          nonBlocking ? ' pointer-events-auto' : ''
        }`}
      >
        {title !== undefined && (
          <div className="flex items-start justify-between gap-4 border-b border-gray-200 px-6 py-4">
            <div className="min-w-0 flex-1">
              {title && (
                <h2 id={titleId} className="text-sm font-semibold text-gray-900">
                  {title}
                </h2>
              )}
              {subtitle && <p className="mt-0.5 text-xs text-gray-500">{subtitle}</p>}
            </div>
            <button
              onClick={onClose}
              className="rounded p-1 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600"
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
          <div className="flex items-center justify-end gap-3 border-t border-gray-200 px-6 py-4">{footer}</div>
        )}
      </div>
    </div>
  );
}
