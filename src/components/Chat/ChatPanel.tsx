import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { prewarmChat } from '@/lib/api';
import { type ChatContext, chatContextKey, chatTurnContext, isConversationThreadBound } from '@/lib/chatContext';
import { errorText } from '@/lib/errors';
import { useChatStore } from '@/stores/chatStore';
import { useLogStore } from '@/stores/logStore';
import { ChatAccountPicker } from './ChatAccountPicker';
import { ChatInput } from './ChatInput';
import { MessageList } from './MessageList';
import { ThreadContextChip } from './ThreadContextChip';

interface ChatPanelProps {
  accountId: string | null;
  /** Retarget which account the chat searches. */
  onAccountChange: (accountId: string) => void;
  /** Thread the main view currently shows, or null. */
  context: ChatContext | null;
  onClose: () => void;
  /** Switch to the roomy full-page chat view (same conversation). */
  onExpand: () => void;
  /** A citation was clicked — the parent navigates the main view to it. */
  onNavigateToInbox?: () => void;
}

/**
 * Right-docked chat panel: the same conversation as the full-page view (both
 * read `chatStore`), in a narrow column beside the mail content, with the open
 * thread offered as ambient context.
 */
export function ChatPanel({
  accountId,
  onAccountChange,
  context,
  onClose,
  onExpand,
  onNavigateToInbox,
}: ChatPanelProps) {
  const { t } = useTranslation(['chat', 'common']);
  const {
    conversations,
    activeConversationId,
    messages,
    streamingMessageId,
    streamingPhase,
    isSending,
    isLoadingMessages,
    error,
    fetchConversations,
    createConversation,
    selectConversation,
    sendMessage,
    loadCategoriesPref,
    categoriesLoaded,
  } = useChatStore();
  const addLog = useLogStore((s) => s.addLog);

  // Whether the offered context is armed. Keyed by thread so moving to another
  // thread re-arms it — a dismissal applies to the thread it was made on, not
  // to every thread the user visits afterwards.
  const [dismissedContextKey, setDismissedContextKey] = useState<string | null>(null);
  const contextKey = chatContextKey(context);
  // A conversation seeded via "Chat about this thread" already owns its
  // grounding — the backend ignores ambient context for it, so don't offer any.
  const offeredContext = isConversationThreadBound(messages) ? null : context;
  const contextActive = offeredContext !== null && dismissedContextKey !== contextKey;

  useEffect(() => {
    if (!categoriesLoaded) void loadCategoriesPref();
  }, [categoriesLoaded, loadCategoriesPref]);

  // Mirror ChatView's account plumbing: (re)load the conversation list and seed
  // the local model's prompt-prefix cache. Both surfaces share one store, so
  // whichever mounts first does the work and the other reuses it.
  const lastAccountIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (!accountId) return;
    void fetchConversations(accountId);
    if (lastAccountIdRef.current !== null && lastAccountIdRef.current !== accountId) {
      void selectConversation(null);
    }
    lastAccountIdRef.current = accountId;
    prewarmChat(accountId).catch(() => {});
  }, [accountId, fetchConversations, selectConversation]);

  const handleSend = async (content: string) => {
    if (!activeConversationId) {
      if (!accountId) return;
      try {
        await createConversation(accountId);
      } catch (e) {
        addLog('error', 'ai', `Failed to create conversation: ${errorText(e)}`);
        return;
      }
    }
    addLog('info', 'ai', `Sent: ${content.slice(0, 60)}${content.length > 60 ? '…' : ''}`);
    // Only pass the thread when the chip is armed — a dismissed chip must
    // behave exactly like having nothing open (normal retrieval).
    // The thread's OWN account travels with it — in unified mode it differs
    // from the chat's account, and grounding looked the thread up under the
    // chat's account and found nothing.
    const turnContext = chatTurnContext(offeredContext, contextActive);
    await sendMessage(content, turnContext?.threadId ?? null, turnContext?.accountId ?? null);
  };

  const header = (
    <div className="flex items-center gap-1 border-b border-gray-200 bg-white px-2 py-1.5">
      <ChatAccountPicker accountId={accountId} onChange={onAccountChange} compact />
      <select
        value={activeConversationId ?? ''}
        onChange={(e) => void selectConversation(e.target.value || null)}
        title={t('chat:panel.conversationPicker')}
        aria-label={t('chat:panel.conversationPicker')}
        className="min-w-0 flex-1 truncate rounded border-none bg-transparent px-1 py-0.5 text-xs font-medium text-gray-700 hover:bg-gray-100 focus:outline-none"
      >
        <option value="">{t('chat:conversations.newChat')}</option>
        {conversations.map((c) => (
          <option key={c.id} value={c.id}>
            {c.title}
          </option>
        ))}
      </select>
      <button
        type="button"
        onClick={() => void (accountId && createConversation(accountId))}
        title={t('chat:newConversation')}
        aria-label={t('chat:newConversation')}
        className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
      >
        <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
          <path d="M8 3v10M3 8h10" strokeLinecap="round" />
        </svg>
      </button>
      <button
        type="button"
        onClick={onExpand}
        title={t('chat:panel.expand')}
        aria-label={t('chat:panel.expand')}
        className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
      >
        <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5}>
          <path d="M6 2H2v4M10 14h4v-4" strokeLinecap="round" strokeLinejoin="round" />
          <path d="M2 2l5 5M14 14l-5-5" strokeLinecap="round" />
        </svg>
      </button>
      <button
        type="button"
        onClick={onClose}
        title={t('chat:panel.close')}
        aria-label={t('chat:panel.close')}
        className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
      >
        <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
          <path d="M4 4l8 8M12 4l-8 8" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );

  if (!accountId) {
    return (
      <div className="flex h-full flex-col bg-white">
        {header}
        <div className="flex flex-1 items-center justify-center px-4 text-center text-xs text-gray-500">
          {t('chat:noAccount')}
        </div>
      </div>
    );
  }

  const showEmpty = !activeConversationId || (!isLoadingMessages && messages.length === 0);

  return (
    <div className="flex h-full flex-col bg-white">
      {header}

      <div className="flex flex-1 flex-col overflow-hidden">
        {showEmpty ? (
          <div className="flex flex-1 items-center justify-center px-4 text-center text-xs text-gray-500">
            {t('chat:panel.emptyHint')}
          </div>
        ) : isLoadingMessages ? (
          <div className="flex flex-1 items-center justify-center text-xs text-gray-500">
            {t('common:state.loading')}
          </div>
        ) : (
          <MessageList
            messages={messages}
            streamingMessageId={streamingMessageId}
            streamingPhase={streamingPhase}
            accountId={accountId}
            onOpenEmail={onNavigateToInbox}
          />
        )}
        {error && <div className="border-t border-red-200 bg-red-50 px-3 py-2 text-xs text-red-600">{error}</div>}
      </div>

      <ChatInput
        compact
        onSend={handleSend}
        disabled={isSending || streamingMessageId !== null}
        placeholder={streamingMessageId ? t('chat:input.waitingReply') : t('chat:input.placeholderEmails')}
        contextSlot={
          offeredContext ? (
            <ThreadContextChip
              context={offeredContext}
              active={contextActive}
              onToggle={(next) => setDismissedContextKey(next ? null : contextKey)}
            />
          ) : undefined
        }
      />
    </div>
  );
}
