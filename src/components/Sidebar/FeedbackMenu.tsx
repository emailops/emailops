import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FEEDBACK_TYPES, type FeedbackType } from '@/lib/feedback';

interface FeedbackMenuProps {
  /** Called with the chosen feedback kind when the user picks an option. */
  onSelect: (type: FeedbackType) => void;
}

/**
 * Sidebar "Give feedback" button. Clicking opens a small popover listing the
 * feedback kinds (general / bug / idea); picking one delegates to `onSelect`,
 * which opens a pre-filled compose tab. Labels come from the `sidebar` i18n
 * namespace, keyed by feedback type.
 */
export function FeedbackMenu({ onSelect }: FeedbackMenuProps) {
  const { t } = useTranslation(['sidebar']);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on outside click or Escape while the popover is open.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  return (
    <div ref={containerRef} className="relative mt-3">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        className="w-full flex items-center gap-2 px-3 py-2 text-sm font-medium text-primary-300 bg-primary-500/10 hover:bg-primary-500/20 border border-primary-500/30 rounded-lg transition-colors"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
          />
        </svg>
        <span>{t('sidebar:feedback.button')}</span>
      </button>

      {open && (
        <div
          role="menu"
          className="absolute left-0 right-0 mt-1 z-20 py-1 bg-gray-800 border border-gray-700 rounded-lg shadow-lg overflow-hidden"
        >
          {FEEDBACK_TYPES.map((type) => (
            <button
              key={type}
              type="button"
              role="menuitem"
              onClick={() => {
                onSelect(type);
                setOpen(false);
              }}
              className="w-full text-left px-3 py-2 text-sm text-gray-300 hover:bg-gray-700 transition-colors"
            >
              {t(`sidebar:feedback.${type}` as const)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
