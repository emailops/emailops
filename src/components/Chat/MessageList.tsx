import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import type { ChatMessage } from '@/types';
import { MessageBubble } from './MessageBubble';

interface MessageListProps {
  messages: ChatMessage[];
  streamingMessageId: string | null;
  accountId: string;
  onOpenEmail?: () => void;
}

/**
 * Collapsible card showing the email-thread context that seeded a chat
 * created via "Chat about this thread". The thread is stored as a
 * role='system' message; we render it here once at the top instead of as a
 * regular bubble so the conversation flow stays clean.
 */
function ThreadContextCard({ content }: { content: string }) {
  const { t } = useTranslation(['chat']);
  const fmt = useFormatters();
  const [expanded, setExpanded] = useState(false);
  const charCount = content.length;
  return (
    <div className="mb-4 rounded-lg border border-gray-200 bg-gray-50">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-2 px-3 py-2 text-left text-sm text-gray-700 hover:bg-gray-100 rounded-lg"
      >
        <svg
          className={`w-3.5 h-3.5 text-gray-400 transition-transform ${expanded ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
          />
        </svg>
        <span className="font-medium">{t('chat:threadContext.label')}</span>
        <span className="ml-auto text-xs text-gray-400">{fmt.number(charCount)} chars</span>
      </button>
      {expanded && (
        <div className="px-3 pb-3 text-xs text-gray-600 whitespace-pre-wrap font-mono max-h-96 overflow-y-auto">
          {content}
        </div>
      )}
    </div>
  );
}

export function MessageList({ messages, streamingMessageId, accountId, onOpenEmail }: MessageListProps) {
  const { t } = useTranslation(['chat']);
  const containerRef = useRef<HTMLDivElement>(null);

  // System messages seed the chat with thread content; render them as a
  // collapsible context card, not as bubbles in the conversation flow.
  const systemMessages = messages.filter((m) => m.role === 'system');
  const conversationMessages = messages.filter((m) => m.role !== 'system');

  // biome-ignore lint/correctness/useExhaustiveDependencies: deps act as triggers (new message / streaming tokens) — values themselves aren't read inside the effect.
  useEffect(() => {
    // Scroll to bottom on new message or streaming tokens.
    // Direct scrollTop assignment is more reliable than scrollIntoView during
    // rapid streaming updates, which can interrupt smooth-scroll animations.
    if (containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [messages.length, messages[messages.length - 1]?.content]);

  if (messages.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-400 text-sm">
        {t('chat:emptyState.askToStart')}
      </div>
    );
  }

  return (
    <div ref={containerRef} className="flex-1 overflow-y-auto px-6 py-4">
      {systemMessages.map((m) => (
        <ThreadContextCard key={m.id} content={m.content} />
      ))}
      {conversationMessages.length === 0 && systemMessages.length > 0 && (
        <div className="text-center text-sm text-gray-400 py-4">{t('chat:emptyState.askThreadToStart')}</div>
      )}
      {conversationMessages.map((m) => (
        <MessageBubble
          key={m.id}
          message={m}
          isStreaming={m.id === streamingMessageId}
          accountId={accountId}
          onOpenEmail={onOpenEmail}
        />
      ))}
    </div>
  );
}
