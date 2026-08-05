import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useResponsiveLayout } from '@/hooks/useResponsiveLayout';
import { prewarmChat } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useChatStore } from '@/stores/chatStore';
import { useLogStore } from '@/stores/logStore';
import { ChatAccountPicker } from './ChatAccountPicker';
import { ChatInput } from './ChatInput';
import { ConversationList } from './ConversationList';
import { MessageList } from './MessageList';

// Each shortcut chip either auto-sends a fully-formed prompt (action: 'send'
// — the model picks the right retrieval path and renders a scannable answer)
// or seeds the chat input with a sentence stem the user finishes themselves
// (action: 'prefill' — for open-ended starters like "Write a draft for …").
// Labels + prompts come from the `chat:shortcuts.*` locale keys; only the
// icon and the dispatch action stay inline.
const SHORTCUTS = [
  { id: 'today', icon: '📋', action: 'send' },
  { id: 'thisWeek', icon: '📅', action: 'send' },
  { id: 'draft', icon: '✍️', action: 'prefill' },
] as const;

interface ChatViewProps {
  accountId: string | null;
  /** Retarget which account the chat searches. */
  onAccountChange: (accountId: string) => void;
  /** Called when a citation opens an email — lets the parent switch to the inbox view. */
  onNavigateToInbox?: () => void;
}

export function ChatView({ accountId, onAccountChange, onNavigateToInbox }: ChatViewProps) {
  const { t } = useTranslation(['chat', 'common']);
  // Chat conversations are hard-scoped to one account. In unified
  const {
    conversations,
    activeConversationId,
    messages,
    streamingMessageId,
    streamingPhase,
    isSending,
    isLoadingConversations,
    isLoadingMessages,
    error,
    selectAccount,
    createConversation,
    selectConversation,
    renameConversation,
    deleteConversation,
    sendMessage,
    loadCategoriesPref,
    categoriesLoaded,
    selectedCategories,
  } = useChatStore();
  const addLog = useLogStore((s) => s.addLog);
  // Prefill plumbing for shortcut chips that ask the user to finish the
  // sentence (e.g. "Write a draft for …") instead of auto-sending. The
  // nonce lets us re-apply the same text after another click — useState
  // skips re-renders when the literal value is unchanged.
  const [inputPrefillText, setInputPrefillText] = useState<string | undefined>(undefined);
  const [inputPrefillNonce, setInputPrefillNonce] = useState(0);

  // Load the persisted category filter once, so the dropdown reflects the
  // user's last choice immediately on mount.
  useEffect(() => {
    if (!categoriesLoaded) void loadCategoriesPref();
  }, [categoriesLoaded, loadCategoriesPref]);

  // Load conversations whenever the active account changes. Track the previous
  // accountId so that re-mounting the view (e.g. when navigating from inbox
  // back to chat, or via "Chat about this thread") does NOT wipe the currently
  // active conversation — only an actual account switch should clear it.
  // App.tsx already resets the entire chat store on account switch, so this
  // effect just needs to (re)load the conversation list for the current
  // account.
  useEffect(() => {
    if (!accountId) return;
    // `selectAccount` owns the conversation swap: it remembers where we were
    // and restores the conversation last open for this account this session,
    // falling back to a fresh chat.
    void selectAccount(accountId);
    // Seed the local model's prompt-prefix cache for this account so the
    // first turn skips most of its prefill (also re-seeds after the 30-min
    // idle eviction). Fire-and-forget: a failure just means a cold prefill.
    prewarmChat(accountId).catch(() => {});
  }, [accountId, selectAccount]);

  const handleCreate = async () => {
    if (!accountId) return;
    try {
      await createConversation(accountId);
    } catch (e) {
      addLog('error', 'ai', `Failed to create conversation: ${errorText(e)}`);
    }
  };

  const handleSend = async (content: string) => {
    // Auto-create a conversation if none is active (first message in a session).
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
    await sendMessage(content);
  };

  // Stacked (phone) layout shows exactly one of the two panes at a time. Side
  // by side, the conversation list eats most of a 390px viewport and squeezes
  // the message column so hard the input placeholder wraps to one character
  // per line. Local state rather than a store field: which pane a phone is
  // showing is view-local navigation, not app state the backend or desktop
  // cares about.
  const { isStacked } = useResponsiveLayout();
  const [mobileShowList, setMobileShowList] = useState(false);

  // Opening Chat on a phone starts a blank conversation instead of resuming
  // the last one. The store outlives this component, so without it tapping
  // Chat reopened whatever was last active — and on a phone the history that
  // would explain *why* is on a screen of its own. Desktop keeps resuming
  // (see the fetchConversations effect above): the list is visible there.
  //
  // Read through a ref so this is mount-only. Reacting to `isStacked` would
  // wipe a live conversation the moment a desktop window is dragged narrow.
  const entry = useRef({ isStacked, accountId });
  useEffect(() => {
    if (entry.current.isStacked && entry.current.accountId) void selectConversation(null);
  }, [selectConversation]);

  if (!accountId) {
    return (
      <div className="flex-1 flex items-center justify-center bg-white text-gray-500 text-sm">
        {t('chat:noAccount')}
      </div>
    );
  }

  // Show the intro + shortcut chips whenever the main pane is otherwise empty —
  // either no conversation is selected yet, or the selected conversation has no
  // messages. That way a user who created a fresh "New chat" still gets the
  // same one-click prompts as the first-ever visit.
  const showIntro = !activeConversationId || (!isLoadingMessages && messages.length === 0);

  const showList = !isStacked || mobileShowList;
  const showPane = !isStacked || !mobileShowList;

  return (
    <div className="flex flex-1 overflow-hidden bg-white">
      {showList && (
        <ConversationList
          conversations={conversations}
          activeId={activeConversationId}
          isLoading={isLoadingConversations}
          onSelect={(id) => {
            void selectConversation(id);
            setMobileShowList(false);
          }}
          onCreate={() => {
            void handleCreate();
            setMobileShowList(false);
          }}
          onRename={(id, title) => void renameConversation(id, title)}
          onDelete={(id) => void deleteConversation(id)}
        />
      )}

      {showPane && (
        <div className="flex-1 min-w-0 flex flex-col overflow-hidden">
          {isStacked && (
            <button
              type="button"
              onClick={() => setMobileShowList(true)}
              className="flex h-11 items-center gap-2 border-b border-gray-200 px-3 text-sm text-gray-600 active:bg-gray-100"
            >
              <svg className="h-4 w-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={2}>
                <path d="M10 3L5 8l5 5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
              {t('chat:conversations.title')}
            </button>
          )}
          {/* Not gated on unified mode: chat answers from ONE account whatever
            the mail view is doing, so which account that is matters just as
            much when the sidebar is on a single account. The picker hides
            itself when there is only one to choose from. */}
          <ChatAccountPicker accountId={accountId} onChange={onAccountChange} />
          {showIntro ? (
            <>
              <div className="flex-1 flex flex-col items-center justify-center text-center px-6 gap-5">
                <div>
                  <h2 className="text-lg font-medium text-gray-900 mb-1">{t('chat:intro.title')}</h2>
                  <p className="text-sm text-gray-500 max-w-md">{t('chat:intro.body')}</p>
                </div>
                <div className="text-xs text-gray-500 max-w-md">
                  {t('chat:intro.retrievalRestricted', {
                    categories: selectedCategories.length > 0 ? selectedCategories.join(', ') : 'primary',
                  })}
                </div>
                <div className="flex flex-wrap justify-center gap-2 max-w-lg">
                  {SHORTCUTS.map((s) => (
                    <button
                      key={s.id}
                      onClick={() => {
                        const prompt = t(`chat:shortcuts.${s.id}.prompt` as const);
                        if (s.action === 'prefill') {
                          setInputPrefillText(prompt);
                          // Bumping the nonce re-fires the prefill effect even
                          // when the prompt text is unchanged (otherwise a
                          // second click of the same chip would be a no-op).
                          setInputPrefillNonce((n) => n + 1);
                        } else {
                          void handleSend(prompt);
                        }
                      }}
                      disabled={isSending}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded-full border border-gray-200 bg-white text-sm text-gray-700 hover:border-primary-400 hover:text-primary-700 hover:bg-primary-50 transition-colors shadow-sm disabled:opacity-50"
                    >
                      <span>{s.icon}</span>
                      <span>{t(`chat:shortcuts.${s.id}.label` as const)}</span>
                    </button>
                  ))}
                </div>
              </div>
              <ChatInput
                onSend={handleSend}
                disabled={isSending}
                placeholder={t('chat:input.placeholderEmails')}
                prefillText={inputPrefillText}
                prefillNonce={inputPrefillNonce}
              />
            </>
          ) : (
            <>
              <div className="flex-1 flex flex-col overflow-hidden">
                {isLoadingMessages ? (
                  <div className="flex-1 flex items-center justify-center text-sm text-gray-500">
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
                {error && (
                  <div className="px-6 py-2 text-xs text-red-600 bg-red-50 border-t border-red-200">{error}</div>
                )}
              </div>
              <ChatInput
                onSend={handleSend}
                disabled={isSending || streamingMessageId !== null}
                placeholder={streamingMessageId ? t('chat:input.waitingReply') : t('chat:input.placeholderEmails')}
              />
            </>
          )}
        </div>
      )}
    </div>
  );
}
