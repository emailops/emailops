import { type KeyboardEvent, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getChatStarters } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { ChatStarter } from '@/types';

// Shown until the backend answers, and kept if it fails: the starters that need
// nothing from the mailbox.
const FIXED: ChatStarter[] = [
  { kind: 'today', action: 'send' },
  { kind: 'thisWeek', action: 'send' },
  { kind: 'draft', action: 'prefill' },
];

const ICONS: Record<ChatStarter['kind'], string> = {
  today: '📋',
  sender: '👤',
  thread: '🧵',
  thisWeek: '📅',
  draft: '✍️',
};

interface ChatStartersProps {
  accountId: string;
  disabled: boolean;
  /** Tighter rows for the narrow side panel. */
  compact?: boolean;
  onSend: (prompt: string) => void;
  onPrefill: (text: string) => void;
  /** The last row: the user wants to type their own request. */
  onSomethingElse: () => void;
}

/**
 * The requests an empty conversation suggests, as a vertical list: today's
 * mail, a recent correspondent, a recent thread, this week and a draft. The
 * backend picks them from the mailbox; the wording comes from the locales.
 */
export function ChatStarters({
  accountId,
  disabled,
  compact = false,
  onSend,
  onPrefill,
  onSomethingElse,
}: ChatStartersProps) {
  const { t } = useTranslation(['chat']);
  const addLog = useLogStore((s) => s.addLog);
  const [starters, setStarters] = useState<ChatStarter[]>(FIXED);
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let current = true;
    setStarters(FIXED);
    getChatStarters(accountId)
      .then((list) => {
        if (current) setStarters(list);
      })
      .catch((e) => {
        if (current) addLog('error', 'ai', `Failed to load chat suggestions: ${errorText(e)}`);
      });
    return () => {
      current = false;
    };
  }, [accountId, addLog]);

  const text = (s: ChatStarter): { label: string; prompt: string } => {
    switch (s.kind) {
      case 'sender': {
        const name = s.senderName ?? '';
        return {
          label: t('chat:starters.sender.label', { name }),
          prompt: t('chat:starters.sender.prompt', { name }),
        };
      }
      case 'thread': {
        const subject = s.subject ?? '';
        return {
          label: t('chat:starters.thread.label', { subject }),
          prompt: t('chat:starters.thread.prompt', { subject }),
        };
      }
      default:
        return { label: t(`chat:shortcuts.${s.kind}.label`), prompt: t(`chat:shortcuts.${s.kind}.prompt`) };
    }
  };

  // ↑/↓ move between rows; Enter and Space activate the focused button natively.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const buttons = [...(listRef.current?.querySelectorAll<HTMLButtonElement>('button') ?? [])];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (at === -1) return;
    e.preventDefault();
    const next = e.key === 'ArrowDown' ? Math.min(at + 1, buttons.length - 1) : Math.max(at - 1, 0);
    buttons[next]?.focus();
  };

  const row = `flex w-full items-center gap-2 rounded-lg border border-gray-200 bg-white text-left text-gray-700 shadow-sm transition-colors hover:border-primary-400 hover:bg-primary-50 hover:text-primary-700 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-400 disabled:opacity-50 ${
    compact ? 'px-2.5 py-1.5 text-xs' : 'px-3 py-2 text-sm'
  }`;

  return (
    <div
      ref={listRef}
      role="group"
      aria-label={t('chat:starters.title')}
      onKeyDown={onKeyDown}
      className="flex w-full max-w-md flex-col gap-1.5"
    >
      {starters.map((s) => {
        const { label, prompt } = text(s);
        return (
          <button
            key={`${s.kind}-${label}`}
            type="button"
            data-testid="chat-starter"
            disabled={disabled}
            onClick={() => (s.action === 'prefill' ? onPrefill(prompt) : onSend(prompt))}
            className={row}
          >
            <span aria-hidden="true">{ICONS[s.kind]}</span>
            <span className="truncate">{label}</span>
          </button>
        );
      })}
      <button
        type="button"
        data-testid="chat-starter-other"
        disabled={disabled}
        onClick={onSomethingElse}
        className={`${row} text-gray-500`}
      >
        <span aria-hidden="true">✎</span>
        <span>{t('chat:starters.somethingElse')}</span>
      </button>
    </div>
  );
}
