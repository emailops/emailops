import { useTranslation } from 'react-i18next';
import type { DraftChipScope } from '@/lib/chatDraft';

interface DraftContextChipProps {
  /** The draft's subject as the composer has it now. */
  subject: string;
  /** What travels with the turn: the reply and its thread, the reply alone, or a new email. */
  scope: DraftChipScope;
  /** False once the user switched it off — the chip then offers to restore. */
  active: boolean;
  /** The next message is a research turn, which reads the mailbox, not the draft. */
  research: boolean;
  onToggle: (active: boolean) => void;
}

/**
 * The draft the user is writing, offered to the chat above the panel's input.
 * A toggle like `ThreadContextChip`: switching it off mutes it instead of
 * hiding it, so the way back stays on screen.
 */
export function DraftContextChip({ subject, scope, active, research, onToggle }: DraftContextChipProps) {
  const { t } = useTranslation('chat');
  const title = subject.trim() || t('panel.context.noSubject');

  return (
    <div className="mb-2">
      <button
        type="button"
        onClick={() => onToggle(!active)}
        title={active ? t('panel.draft.hint') : t('panel.draft.restore')}
        aria-pressed={active}
        className={`group flex w-full items-center gap-1.5 rounded-md border px-2 py-1 text-left text-xs transition-colors ${
          active
            ? 'border-primary-200 bg-primary-50 text-primary-800 hover:border-primary-300'
            : 'border-gray-200 bg-gray-50 text-gray-400 hover:border-gray-300 hover:text-gray-600'
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
          <path d="M11 2.5l2.5 2.5L6 12.5H3.5V10z" strokeLinejoin="round" />
        </svg>
        {/* The subject keeps a minimum width: the state label on the right is
            what gives way, or a long "restore" label squeezed it to nothing. */}
        <span className="min-w-12 truncate font-medium">{title}</span>
        <span className="flex-shrink-0 opacity-70">· {t(`panel.draft.${scope}`)}</span>
        <span className="ml-auto min-w-0 truncate pl-1 text-[10px] uppercase tracking-wide opacity-70">
          {active ? t('panel.context.using') : t('panel.draft.restore')}
        </span>
      </button>
      {research && active && <p className="mt-1 text-[11px] text-gray-500">{t('panel.draft.research')}</p>}
    </div>
  );
}
