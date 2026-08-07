import { useTranslation } from 'react-i18next';
import type { ChatContext } from '@/lib/chatContext';

interface ThreadContextChipProps {
  context: ChatContext;
  /** False once the user dismissed it — the chip then offers to restore. */
  active: boolean;
  onToggle: (active: boolean) => void;
}

/**
 * The "what the model will see" chip above the panel's input, mirroring the
 * current-file chip in Cursor/Copilot.
 *
 * Dismissing it does not hide the chip — it flips to a muted "restore" state.
 * Hiding outright would leave the user with no way back to thread grounding
 * without navigating away and returning, and no way to tell whether the next
 * answer will come from the thread or from a whole-inbox search.
 */
export function ThreadContextChip({ context, active, onToggle }: ThreadContextChipProps) {
  const { t } = useTranslation('chat');
  const subject = context.subject.trim() || t('panel.context.noSubject');

  return (
    <div className="mb-2">
      <button
        type="button"
        onClick={() => onToggle(!active)}
        title={active ? t('panel.context.hint') : t('panel.context.restore')}
        aria-pressed={active}
        className={`group flex w-full items-center gap-1.5 rounded-md border px-2 py-1 text-left text-xs transition-colors ${
          active
            ? 'border-primary-200 bg-primary-50 text-primary-800 hover:border-primary-300 dark:border-primary-800 dark:bg-primary-900/20 dark:text-primary-300'
            : 'border-gray-200 bg-gray-50 text-gray-400 hover:border-gray-300 hover:text-gray-600 dark:border-gray-700 dark:bg-surface-raised dark:text-gray-500 dark:hover:border-gray-600 dark:hover:text-gray-400'
        }`}
      >
        <svg
          className="h-3 w-3 flex-shrink-0"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.5}
          aria-hidden="true"
        >
          <rect x="1.5" y="3" width="13" height="10" rx="1.5" />
          <path d="M2 4.5l6 4 6-4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        <span className="truncate font-medium">{subject}</span>
        <span className="ml-auto flex-shrink-0 pl-1 text-[10px] uppercase tracking-wide opacity-70">
          {active ? t('panel.context.using') : t('panel.context.restore')}
        </span>
        {active && (
          <svg
            className="h-3 w-3 flex-shrink-0 opacity-50 group-hover:opacity-100"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            aria-hidden="true"
          >
            <path d="M4 4l8 8M12 4l-8 8" strokeLinecap="round" />
          </svg>
        )}
      </button>
    </div>
  );
}
