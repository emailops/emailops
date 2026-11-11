import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import { errorText } from '@/lib/errors';
import { FEEDBACK_TYPES, type FeedbackType } from '@/lib/feedback';
import { useLogStore } from '@/stores/logStore';

/** The project repository the star button sends the user to. */
export const GITHUB_REPO_URL = 'https://github.com/emailops/emailops';

interface FeedbackMenuProps {
  /** Called with the chosen feedback kind when the user picks an option. */
  onSelect: (type: FeedbackType) => void;
}

/**
 * Sidebar "Give feedback" row. The main button opens a small popover listing
 * the feedback kinds (general / bug / idea); picking one delegates to
 * `onSelect`, which opens a pre-filled compose tab. The star button next to it
 * asks, in a dialog, whether to open the GitHub repository in the system
 * browser so the user can star the project. Labels come from the `sidebar`
 * i18n namespace.
 */
export function FeedbackMenu({ onSelect }: FeedbackMenuProps) {
  const { t } = useTranslation(['sidebar', 'common']);
  const { addLog } = useLogStore();
  const [open, setOpen] = useState(false);
  const [starOpen, setStarOpen] = useState(false);
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

  const visitRepository = () => {
    setStarOpen(false);
    void openExternal(GITHUB_REPO_URL).catch((e) => {
      addLog('error', 'system', `Failed to open ${GITHUB_REPO_URL}: ${errorText(e)}`);
    });
  };

  return (
    <div ref={containerRef} className="relative mt-3 flex items-stretch gap-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        className="min-w-0 flex-1 flex items-center gap-2 px-3 py-2 text-sm font-medium text-primary-700 bg-primary-50 hover:bg-primary-100 border border-primary-200 rounded-lg transition-colors"
      >
        <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
          />
        </svg>
        <span className="truncate">{t('sidebar:feedback.button')}</span>
      </button>

      <button
        type="button"
        onClick={() => setStarOpen(true)}
        title={t('sidebar:feedback.star')}
        aria-label={t('sidebar:feedback.star')}
        className="shrink-0 flex items-center justify-center px-2.5 text-primary-700 bg-primary-50 hover:bg-primary-100 border border-primary-200 rounded-lg transition-colors"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z"
          />
        </svg>
      </button>

      {open && (
        <div
          role="menu"
          className="absolute left-0 right-0 top-full mt-1 z-20 py-1 bg-white border border-gray-200 rounded-lg shadow-lg overflow-hidden"
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
              className="w-full text-left px-3 py-2 text-sm text-gray-700 hover:bg-gray-100 transition-colors"
            >
              {t(`sidebar:feedback.${type}` as const)}
            </button>
          ))}
        </div>
      )}

      <Modal
        open={starOpen}
        onClose={() => setStarOpen(false)}
        size="sm"
        title={t('sidebar:feedback.starDialog.title')}
        footer={
          <>
            <button
              type="button"
              onClick={() => setStarOpen(false)}
              className="px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors"
            >
              {t('common:actions.cancel')}
            </button>
            <button
              type="button"
              onClick={visitRepository}
              className="px-4 py-2 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors"
            >
              {t('sidebar:feedback.starDialog.visit')}
            </button>
          </>
        }
      >
        <p className="text-sm text-gray-700 break-words">
          {t('sidebar:feedback.starDialog.body', { url: GITHUB_REPO_URL })}
        </p>
      </Modal>
    </div>
  );
}
