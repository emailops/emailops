import { useTranslation } from 'react-i18next';

interface ChatPanelRailProps {
  onOpen: () => void;
}

/**
 * Slim bar left on the right edge when the chat panel is collapsed, so the
 * chat can be reopened from every view, not only those with their own chat
 * button.
 */
export function ChatPanelRail({ onOpen }: ChatPanelRailProps) {
  const { t } = useTranslation('chat');
  return (
    <div className="flex w-10 flex-shrink-0 flex-col items-center border-l border-gray-200 bg-white pt-3">
      <button
        type="button"
        onClick={onOpen}
        title={t('panel.open')}
        aria-label={t('panel.open')}
        className="rounded p-1.5 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600"
      >
        <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
          />
        </svg>
      </button>
    </div>
  );
}
