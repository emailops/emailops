import { useTranslation } from 'react-i18next';
import type { RightDockContent } from '@/lib/rightDockLayout';

interface RightDockRailProps {
  /** Contents that can be docked, in display order (see `railButtons`). */
  buttons: RightDockContent[];
  onOpen: (content: RightDockContent) => void;
}

/**
 * Slim bar left on the right edge when the dock is collapsed: one button per
 * dockable content — chat, and the calendar directly below it — so either
 * can be reopened from every view.
 */
export function RightDockRail({ buttons, onOpen }: RightDockRailProps) {
  const { t } = useTranslation(['chat', 'calendar']);
  return (
    <div className="flex w-10 flex-shrink-0 flex-col items-center gap-1 border-l border-gray-200 bg-white pt-3">
      {buttons.map((content) => {
        const label = content === 'chat' ? t('chat:panel.open') : t('calendar:panel.open');
        return (
          <button
            key={content}
            type="button"
            onClick={() => onOpen(content)}
            title={label}
            aria-label={label}
            className="rounded p-1.5 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600"
          >
            {content === 'chat' ? (
              <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                />
              </svg>
            ) : (
              <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"
                />
              </svg>
            )}
          </button>
        );
      })}
    </div>
  );
}
