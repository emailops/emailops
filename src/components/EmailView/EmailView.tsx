import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { format } from 'date-fns';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TagChips } from '@/components/common/TagChips';
import type { DraftFailedEvent, DraftGeneratedEvent, DraftSource } from '@/lib/api';
import * as api from '@/lib/api';
import { getThreadViewItems } from '@/lib/threadCollapse';
import { useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';
import { useTagStore } from '@/stores/tagStore';
import type { Account, Email, EmailAttachmentMeta } from '@/types';
import { AttachmentLightbox } from './AttachmentLightbox';
import { ReplyCompose } from './ReplyCompose';
import { ThreadEmailItem } from './ThreadEmailItem';

interface EmailViewProps {
  threadEmails: Email[];
  isLoading: boolean;
  onClose: () => void;
  accounts: Account[];
  activeAccountId: string | null;
  /** When true, renders at full width (used in full-width inbox layout). */
  fullWidth?: boolean;
  onOpenInTab?: () => void;
}

function buildReplyTemplate(email: Email): string {
  const sentAt = format(new Date(email.timestamp * 1000), 'PPpp');
  const quotedLines = (email.snippet || '').split('\n').map((line) => `> ${line}`);
  return ['', '', `On ${sentAt}, ${email.sender} <${email.senderEmail}> wrote:`, ...quotedLines].join('\n');
}

/**
 * Whether the pending chat-generated draft should be consumed into the
 * inline reply on this render. We need the *thread* to be loaded, and the
 * inbound the draft was written for to be inside it. The previous
 * implementation compared only against the latest message in the thread,
 * which silently dropped the body whenever a later reply had arrived
 * between the chat turn and the click. Matching against any message in
 * the loaded thread restores the "click → open reply with body prepended"
 * UX the user expects.
 *
 * Exported (and accepting an `id` getter) so the unit tests in
 * `EmailView.test.ts` can pin the predicate without rendering React.
 */
export function shouldConsumePendingChatDraft(
  pendingChatDraft: { emailId: string } | null,
  threadEmailIds: readonly string[],
): boolean {
  if (!pendingChatDraft) return false;
  if (threadEmailIds.length === 0) return false;
  return threadEmailIds.includes(pendingChatDraft.emailId);
}

// Stable empty reference for the tag selector's missing-key fallback.
// zustand 5 dropped auto-shallow on selector results, so returning `|| []`
// inline produces a new array every render and trips React's
// useSyncExternalStore "getSnapshot should be cached" guard.
const EMPTY_TAGS: readonly string[] = Object.freeze([]);

export function EmailView({
  threadEmails,
  isLoading,
  onClose,
  accounts,
  activeAccountId,
  fullWidth,
  onOpenInTab,
}: EmailViewProps) {
  const { t } = useTranslation(['inbox']);
  const [expandedEmails, setExpandedEmails] = useState<Set<string>>(new Set());
  const [threadExpanded, setThreadExpanded] = useState(false);
  const [lightboxMeta, setLightboxMeta] = useState<EmailAttachmentMeta | null>(null);
  const focusEmailId = useEmailStore((s) => s.focusEmailId);
  const searchQuery = useEmailStore((s) => s.searchQuery);
  const deleteEmailFromStore = useEmailStore((s) => s.deleteEmail);
  const openAttachmentTab = useEmailStore((s) => s.openAttachmentTab);
  // Chat-generated reply draft waiting for its thread to mount. The chat
  // dispatcher seeds this before navigating; consuming it here is what
  // makes the chat draft land inside the inline ReplyCompose (same shape
  // as the AI Draft button) instead of a standalone compose tab.
  const pendingChatDraft = useEmailStore((s) => s.pendingChatDraft);
  const consumePendingChatDraft = useEmailStore((s) => s.consumePendingChatDraft);
  const [isReplyOpen, setIsReplyOpen] = useState(false);
  const [replyMode, setReplyMode] = useState<'reply' | 'reply-all'>('reply');
  const [replyBody, setReplyBody] = useState('');
  const [isDeleting, setIsDeleting] = useState(false);
  const addLog = useLogStore((s) => s.addLog);
  // AI draft state. The request id is held in a ref so the event listener
  // (registered once on mount) can match incoming events without re-binding
  // every time a draft is requested.
  const draftRequestIdRef = useRef<string | null>(null);
  const [isGeneratingDraft, setIsGeneratingDraft] = useState(false);
  const [draftSources, setDraftSources] = useState<DraftSource[]>([]);
  const [aiDraftsEnabled, setAiDraftsEnabled] = useState(true);

  useEffect(() => {
    api
      .getPref('ai_drafts_enabled')
      .then((val) => setAiDraftsEnabled(val !== 'false'))
      .catch(() => setAiDraftsEnabled(true));
  }, []);

  // Subscribe once to draft-generated / draft-failed. Filter on the current
  // request id so an event from a previous click that landed after the user
  // dismissed the compose doesn't mutate the textarea unexpectedly.
  useEffect(() => {
    let unlistenGen: UnlistenFn | undefined;
    let unlistenFail: UnlistenFn | undefined;
    void (async () => {
      unlistenGen = await listen<DraftGeneratedEvent>('draft-generated', (event) => {
        if (event.payload.requestId !== draftRequestIdRef.current) return;
        draftRequestIdRef.current = null;
        setIsGeneratingDraft(false);
        setDraftSources(event.payload.sources ?? []);
        // Prepend the AI body above the existing quoted template so the
        // user sees the suggested reply at the top and can still review
        // the quoted history below.
        setReplyBody((existing) => `${event.payload.body}\n\n${existing}`);
        addLog('success', 'ai', `AI draft ready (${event.payload.sources?.length ?? 0} sources)`);
      });
      unlistenFail = await listen<DraftFailedEvent>('draft-failed', (event) => {
        if (event.payload.requestId !== draftRequestIdRef.current) return;
        draftRequestIdRef.current = null;
        setIsGeneratingDraft(false);
        setDraftSources([]);
        addLog('error', 'ai', `AI draft failed: ${event.payload.error}`);
      });
    })();
    return () => {
      unlistenGen?.();
      unlistenFail?.();
    };
  }, [addLog]);

  const handleOpenAttachment = useCallback(
    (meta: EmailAttachmentMeta) => {
      if (meta.mimeType.startsWith('image/')) {
        setLightboxMeta(meta);
      } else {
        openAttachmentTab(meta);
      }
    },
    [openAttachmentTab],
  );

  const latestEmailId = threadEmails.length > 0 ? threadEmails[threadEmails.length - 1].id : '';
  const emailTags = useTagStore((s) => s.tagsByEmail[latestEmailId] || EMPTY_TAGS);
  const latestEmailForEffect = threadEmails.length > 0 ? threadEmails[threadEmails.length - 1] : null;
  const isThread = threadEmails.length > 1;

  // When a search is active, find the oldest email in the thread whose subject,
  // snippet, or (already-loaded) body contains the query. We highlight that email
  // and scroll its first in-body match into view.
  const searchHighlightEmailId = useMemo(() => {
    const q = searchQuery?.trim().toLowerCase();
    if (!q) return null;
    const match = threadEmails.find(
      (e) =>
        e.subject.toLowerCase().includes(q) ||
        e.snippet.toLowerCase().includes(q) ||
        (e.body ?? '').toLowerCase().includes(q),
    );
    return match?.id ?? null;
  }, [threadEmails, searchQuery]);

  useEffect(() => {
    if (!latestEmailForEffect) {
      setIsReplyOpen(false);
      setReplyBody('');
      setDraftSources([]);
      setIsGeneratingDraft(false);
      draftRequestIdRef.current = null;
      return;
    }

    setIsReplyOpen(false);
    setReplyBody(buildReplyTemplate(latestEmailForEffect));
    setThreadExpanded(false);
    setDraftSources([]);
    setIsGeneratingDraft(false);
    draftRequestIdRef.current = null;
  }, [latestEmailForEffect]);

  // Chat-generated reply draft: once the matching thread is loaded, open
  // the inline ReplyCompose with the AI body prepended on top of the
  // quoted template — same shape the AI Draft button produces. Runs after
  // the reset effect above (declaration order = execution order), so the
  // AI body lands on top of a freshly-rebuilt template instead of fighting
  // the reset. Consume clears the slot so re-rendering the same thread
  // (e.g. via account switch and back) does not re-open a stale draft.
  useEffect(() => {
    if (
      !shouldConsumePendingChatDraft(
        pendingChatDraft,
        threadEmails.map((e) => e.id),
      )
    )
      return;
    setReplyMode('reply');
    const draft = pendingChatDraft!;
    setReplyBody((existing) => `${draft.body}\n\n${existing}`);
    setIsReplyOpen(true);
    setDraftSources([]);
    setIsGeneratingDraft(false);
    draftRequestIdRef.current = null;
    consumePendingChatDraft();
  }, [pendingChatDraft, threadEmails, consumePendingChatDraft]);

  if (threadEmails.length === 0 && !isLoading) {
    return (
      <div className="flex-1 bg-white flex items-center justify-center">
        <div className="text-center p-8">
          <svg className="mx-auto h-12 w-12 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1}
              d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"
            />
          </svg>
          <h3 className="mt-2 text-sm font-medium text-gray-900">{t('inbox:noEmailSelected')}</h3>
          <p className="mt-1 text-sm text-gray-500">{t('inbox:selectEmailHint')}</p>
        </div>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="flex-1 bg-white flex items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600 mx-auto"></div>
          <p className="mt-2 text-sm text-gray-500">{t('inbox:loadingThread')}</p>
        </div>
      </div>
    );
  }

  const latestEmail = latestEmailForEffect!;

  const toggleEmailExpanded = (emailId: string) => {
    setExpandedEmails((prev) => {
      const next = new Set(prev);
      if (next.has(emailId)) {
        next.delete(emailId);
      } else {
        next.add(emailId);
      }
      return next;
    });
  };

  return (
    <div className="flex-1 bg-white flex flex-col overflow-hidden">
      {lightboxMeta && <AttachmentLightbox meta={lightboxMeta} onClose={() => setLightboxMeta(null)} />}
      <header className="px-4 py-2 border-b border-gray-200 flex-shrink-0">
        {/* Row 1: subject + inline tags on the left, window controls on the right */}
        <div className="flex items-center gap-3 min-w-0">
          <h1 className="text-lg font-semibold text-gray-900 truncate">{latestEmail.subject || '(No subject)'}</h1>
          {emailTags.length > 0 && (
            <div className="flex-shrink-0">
              <TagChips tags={emailTags} />
            </div>
          )}
          {isThread && <span className="flex-shrink-0 text-xs text-gray-400">{threadEmails.length} msgs</span>}
          <div className="ml-auto flex items-center gap-1 flex-shrink-0">
            <button
              onClick={() => {
                setReplyMode('reply');
                setReplyBody(buildReplyTemplate(latestEmail));
                setIsReplyOpen((value) => !value);
              }}
              className="px-3 py-1 bg-primary-600 text-white text-sm font-medium rounded hover:bg-primary-700 transition-colors"
            >
              Reply
            </button>
            <button
              onClick={() => {
                setReplyMode('reply-all');
                setReplyBody(buildReplyTemplate(latestEmail));
                setIsReplyOpen((value) => !value);
              }}
              className="px-3 py-1 bg-primary-500 text-white text-sm font-medium rounded hover:bg-primary-600 transition-colors"
            >
              {t('inbox:emailView.replyAll')}
            </button>
            {aiDraftsEnabled && (
              <button
                onClick={async () => {
                  // AI Draft always opens in reply-all so the suggested body
                  // lands in a compose with every thread participant prefilled.
                  setReplyMode('reply-all');
                  setReplyBody(buildReplyTemplate(latestEmail));
                  setDraftSources([]);
                  setIsReplyOpen(true);
                  setIsGeneratingDraft(true);
                  addLog('info', 'ai', 'Requesting AI draft…');
                  try {
                    const requestId = await api.generateDraft(latestEmail.id);
                    draftRequestIdRef.current = requestId;
                  } catch (err) {
                    setIsGeneratingDraft(false);
                    draftRequestIdRef.current = null;
                    addLog('error', 'ai', `Failed to start AI draft: ${err}`);
                  }
                }}
                disabled={isGeneratingDraft}
                className="flex items-center gap-1.5 px-3 py-1 bg-purple-600 text-white text-sm font-medium rounded hover:bg-purple-700 transition-colors disabled:opacity-60 disabled:cursor-not-allowed"
                title={t('inbox:emailView.aiDraftTitle')}
              >
                {isGeneratingDraft ? (
                  <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-white" />
                ) : (
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 17.657l.707.707M12 21v-1m-3-7a3 3 0 116 0c0 1.657-1.5 2.5-1.5 4h-3c0-1.5-1.5-2.343-1.5-4z"
                    />
                  </svg>
                )}
                AI Draft
              </button>
            )}
            {onOpenInTab && (
              <button
                onClick={onOpenInTab}
                className="p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailView.openInNewTab')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                  />
                </svg>
              </button>
            )}
            <button
              onClick={async () => {
                if (!latestEmail) return;
                setIsDeleting(true);
                addLog('info', 'sync', `Deleting thread "${latestEmail.subject.slice(0, 50)}"...`);
                try {
                  for (const email of threadEmails) {
                    await deleteEmailFromStore(email.id);
                  }
                  addLog('success', 'sync', 'Thread deleted');
                  onClose();
                } catch (err) {
                  addLog('error', 'sync', `Delete failed: ${err}`);
                } finally {
                  setIsDeleting(false);
                }
              }}
              disabled={isDeleting}
              className="p-1.5 text-gray-400 hover:text-red-500 hover:bg-red-50 rounded transition-colors disabled:opacity-50"
              title={t('inbox:emailView.deleteThread')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                />
              </svg>
            </button>
            {fullWidth ? (
              <button
                onClick={onClose}
                className="flex items-center gap-1 px-2 py-1 text-sm text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailView.back')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                </svg>
                Back
              </button>
            ) : (
              <button
                onClick={onClose}
                className="p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailView.close')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            )}
          </div>
        </div>
        {isReplyOpen && (
          <ReplyCompose
            email={latestEmail}
            threadEmails={threadEmails}
            accounts={accounts}
            defaultAccountId={activeAccountId || latestEmail.accountId}
            mode={replyMode}
            initialBody={replyBody}
            isLoadingDraft={isGeneratingDraft}
            draftSources={draftSources}
            onCancel={() => {
              setIsReplyOpen(false);
              setReplyBody(buildReplyTemplate(latestEmail));
              setDraftSources([]);
              setIsGeneratingDraft(false);
              draftRequestIdRef.current = null;
            }}
            onSend={async ({
              fromAccountId,
              toEmails,
              ccEmails,
              body: replyText,
              bodyHtml,
              inlineImages,
              attachments,
            }) => {
              addLog('info', 'sync', `Sending reply to ${toEmails.join(', ')}...`);
              await api.sendReply(
                latestEmail.id,
                replyText,
                fromAccountId,
                toEmails,
                ccEmails,
                bodyHtml,
                inlineImages,
                attachments,
              );
              await api.syncAccount(latestEmail.accountId);
              addLog('success', 'sync', `Reply sent to ${toEmails.join(', ')}`);
              setIsReplyOpen(false);
            }}
          />
        )}
      </header>

      <div className="flex-1 overflow-y-auto">
        {getThreadViewItems(threadEmails, threadExpanded).map((item) => {
          if (item.type === 'collapsed') {
            return (
              <button
                key="collapsed"
                type="button"
                onClick={() => setThreadExpanded(true)}
                className="w-full px-6 py-3 text-sm text-primary-600 hover:bg-primary-50 border-b border-gray-100 transition-colors text-left"
              >
                Show {item.count} more message{item.count !== 1 ? 's' : ''}
              </button>
            );
          }

          const { email, index } = item;
          const isLast = index === threadEmails.length - 1;
          const isFocused = focusEmailId === email.id;
          const isSearchMatch = searchHighlightEmailId === email.id;
          const isExpanded = isLast || isFocused || isSearchMatch || expandedEmails.has(email.id);

          return (
            <ThreadEmailItem
              key={email.id}
              email={email}
              isExpanded={isExpanded}
              isLast={isLast}
              isFocused={isFocused}
              isSearchMatch={isSearchMatch}
              highlightQuery={searchQuery}
              onToggle={() => toggleEmailExpanded(email.id)}
              onOpenAttachment={handleOpenAttachment}
            />
          );
        })}
      </div>
    </div>
  );
}
