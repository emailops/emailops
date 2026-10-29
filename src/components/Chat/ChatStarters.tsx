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

// With the open thread armed as context every answer is grounded in that thread
// alone, so mailbox-wide suggestions would mislead: these replace them.
type OpenThreadKind = 'summary' | 'asks' | 'reply';
const OPEN_THREAD: { kind: OpenThreadKind; action: ChatStarter['action'] }[] = [
  { kind: 'summary', action: 'send' },
  { kind: 'asks', action: 'send' },
  { kind: 'reply', action: 'prefill' },
];

// With an open PDF armed as context the requests are about the document.
type OpenDocumentKind = 'summary' | 'keyPoints' | 'ask';
const OPEN_DOCUMENT: { kind: OpenDocumentKind; action: ChatStarter['action'] }[] = [
  { kind: 'summary', action: 'send' },
  { kind: 'keyPoints', action: 'send' },
  { kind: 'ask', action: 'prefill' },
];

const ICONS: Record<ChatStarter['kind'], string> = {
  today: '📋',
  reply: '↩️',
  sender: '👤',
  thread: '🧵',
  thisWeek: '📅',
  draft: '✍️',
};

interface ChatStartersProps {
  accountId: string;
  /** What the docked panel's chip has armed as the turn's context, if
   *  anything: the open thread or an open PDF. */
  openContext?: 'thread' | 'document';
  disabled: boolean;
  /** Tighter rows for the narrow side panel. */
  compact?: boolean;
  /** `threadId`: ground the turn in that thread (the unanswered-thread row). */
  onSend: (prompt: string, threadId?: string) => void;
  onPrefill: (text: string) => void;
  /** The last row: the user wants to type their own request. */
  onSomethingElse: () => void;
}

/**
 * The requests an empty conversation suggests, as a vertical list: today's
 * mail, a thread waiting on a reply, a recent correspondent, a recent thread,
 * this week and a draft. The backend picks them from the mailbox; the wording
 * comes from the locales. With the open thread or an open PDF as context, the
 * list is about that instead.
 */
export function ChatStarters({
  accountId,
  openContext,
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
      case 'reply': {
        const params = { name: s.senderName ?? '', subject: s.subject ?? '' };
        return {
          label: t('chat:starters.reply.label', params),
          prompt: t('chat:starters.reply.prompt', params),
        };
      }
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
      {(openContext === 'document'
        ? OPEN_DOCUMENT.map((s) => ({
            key: `openDocument-${s.kind}`,
            icon: '📄',
            action: s.action,
            threadId: undefined,
            label: t(`chat:starters.openDocument.${s.kind}.label`),
            prompt: t(`chat:starters.openDocument.${s.kind}.prompt`),
          }))
        : openContext === 'thread'
          ? OPEN_THREAD.map((s) => ({
              key: `openThread-${s.kind}`,
              icon: '✉️',
              action: s.action,
              threadId: undefined,
              label: t(`chat:starters.openThread.${s.kind}.label`),
              prompt: t(`chat:starters.openThread.${s.kind}.prompt`),
            }))
          : starters.map((s) => {
              const { label, prompt } = text(s);
              return {
                key: `${s.kind}-${label}`,
                icon: ICONS[s.kind],
                action: s.action,
                threadId: s.threadId,
                label,
                prompt,
              };
            })
      ).map(({ key, icon, action, threadId, label, prompt }) => (
        <button
          key={key}
          type="button"
          data-testid="chat-starter"
          disabled={disabled}
          title={label}
          onClick={() =>
            action === 'prefill' ? onPrefill(prompt) : threadId ? onSend(prompt, threadId) : onSend(prompt)
          }
          className={row}
        >
          <span aria-hidden="true">{icon}</span>
          <span className="truncate">{label}</span>
        </button>
      ))}
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
