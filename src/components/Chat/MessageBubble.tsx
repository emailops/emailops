import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { useChatStore } from '@/stores/chatStore';
import { useLogStore } from '@/stores/logStore';
import type { ChatMessage, ChatPhase } from '@/types';
import { MarkdownContent } from './MarkdownContent';
import { ReasoningSection, StatsFooter } from './ReasoningTrace';
import { buildIdSearchQuery, collectReferencedEmailIds } from './referencedEmails';
import { SourcesList } from './SourcesList';

interface MessageBubbleProps {
  message: ChatMessage;
  isStreaming: boolean;
  /** Coarse processing stage of the in-flight turn — only set for the message
   *  currently streaming. Drives the LM Studio-style status shown before any
   *  answer tokens arrive. */
  phase?: ChatPhase | null;
  accountId: string;
  onOpenEmail?: () => void;
  /** Show the emails this answer references in the email list, via this search query. */
  onShowEmailsInList?: (query: string) => void;
  /** The user marked this answer wrong and said why — run a corrective turn.
   *  Omitted where retrying makes no sense (an already-rejected answer, or a
   *  surface with no store wired). */
  onReject?: (reason: string) => void;
  /** Already marked wrong: the control is replaced by a note, so the user
   *  cannot stack corrections on the same dead answer. */
  isRejected?: boolean;
  /** A turn is already in flight — the retry button stays visible but inert
   *  rather than queueing a second turn the store would drop anyway. */
  isSending?: boolean;
}

/** LM Studio-style "Processing…" status: a spinner plus a localized label for
 *  the stage the backend just entered (routing → retrieving → tools →
 *  generating). Shown in place of the bare typing dots once the backend tells
 *  us what it's doing, so a slow prompt-processing pass reads as progress
 *  rather than a hang. */
function ProcessingStatus({ phase }: { phase: ChatPhase }) {
  const { t } = useTranslation(['chat']);
  const research = useChatStore((s) => s.researchProgress);
  const label =
    phase === 'researching' && research
      ? t(`chat:processing.research.${research.stage}` as const, {
          read: research.emailsRead,
          total: research.emailsTotal,
          batch: research.batch,
          batches: research.batches,
        })
      : t(`chat:processing.${phase}` as const);
  return (
    <span className="inline-flex items-center gap-2 text-gray-500">
      <svg className="w-3.5 h-3.5 animate-spin text-gray-400" viewBox="0 0 24 24" fill="none">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
      <span>{label}</span>
    </span>
  );
}

/** Split assistant content into "thinking" (scratchpad inside <think>...</think>
 *  tags that reasoning models like Qwen3-thinking, DeepSeek-R1, QwQ, gpt-oss
 *  emit) and the remaining answer text.
 *
 *  Handles streaming: if the opening tag is present but the closing tag hasn't
 *  arrived yet, treat everything after <think> as in-progress thinking and
 *  return `thinkingComplete: false` so the UI can keep the section expanded
 *  while tokens are still arriving. Handles multiple consecutive blocks too,
 *  in case the model wraps several reasoning passes.
 */
function splitThinking(content: string): {
  thinking: string;
  answer: string;
  thinkingComplete: boolean;
} {
  const OPEN = '<think>';
  const CLOSE = '</think>';
  let answer = '';
  let thinking = '';
  let inProgress = false;
  let i = 0;
  while (i < content.length) {
    const open = content.indexOf(OPEN, i);
    if (open === -1) {
      answer += content.slice(i);
      break;
    }
    answer += content.slice(i, open);
    const close = content.indexOf(CLOSE, open + OPEN.length);
    if (close === -1) {
      // Streaming: closing tag hasn't arrived yet — treat the rest as live thinking.
      thinking += (thinking ? '\n' : '') + content.slice(open + OPEN.length);
      inProgress = true;
      break;
    }
    thinking += (thinking ? '\n' : '') + content.slice(open + OPEN.length, close);
    i = close + CLOSE.length;
  }
  return { thinking, answer, thinkingComplete: !inProgress };
}

/** Collapsible section that shows a reasoning model's <think>...</think> scratchpad.
 *  Auto-expanded while the closing tag hasn't streamed yet; collapsed by default
 *  once thinking finishes so it doesn't dominate the bubble.
 */
function ThinkingSection({ text, streaming }: { text: string; streaming: boolean }) {
  return (
    <details
      className="mb-2 rounded-lg border border-gray-200 bg-gray-50/70 text-xs"
      // `key` forces a re-mount when the streaming flag flips so the `open`
      // attribute is re-applied (browsers ignore changes to `open` on existing
      // <details> elements that the user has interacted with — re-mount is the
      // simplest way to keep the default state honest).
      open={streaming || undefined}
    >
      <summary className="cursor-pointer select-none px-2.5 py-1.5 text-gray-600 font-medium flex items-center gap-1.5 hover:text-gray-800">
        <svg
          className={`w-3.5 h-3.5 text-gray-400 ${streaming ? 'animate-pulse' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          strokeWidth={2}
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
          />
        </svg>
        <span>{streaming ? 'Thinking…' : 'Reasoning'}</span>
      </summary>
      <pre className="px-2.5 pb-2 pt-1 whitespace-pre-wrap font-sans text-[11px] leading-relaxed text-gray-600">
        {text}
      </pre>
    </details>
  );
}

/**
 * "This answer isn't right" → say why → retry.
 *
 * Two steps on purpose: a one-click thumbs-down tells the model nothing it can
 * act on, and the whole point of this control is to feed the retry something
 * specific ("esos correos son de septiembre"). Module scope, not nested in
 * `MessageBubble`, so typing in the textarea survives the parent re-rendering
 * on every stream token (see `src/CLAUDE.md` → Component Identity & Remounts).
 */
function WrongAnswerControl({ onReject, disabled }: { onReject: (reason: string) => void; disabled: boolean }) {
  const { t } = useTranslation(['chat', 'common']);
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Focus on open rather than `autoFocus`: the attribute is an a11y hazard on a
  // control that can mount at any time (a screen reader loses its place), while
  // focusing the box the user just chose to open is exactly what they asked for.
  useEffect(() => {
    if (open) textareaRef.current?.focus();
  }, [open]);

  if (!open) {
    return (
      <button
        type="button"
        data-testid="chat-mark-wrong"
        onClick={() => setOpen(true)}
        className="mt-2 inline-flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs text-gray-500 transition-colors hover:bg-gray-200 hover:text-gray-700"
      >
        <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M10 14H5.236a2 2 0 01-1.789-2.894l3.5-7A2 2 0 018.736 3h4.018a2 2 0 01.485.06l3.76.94m-7 10v5a2 2 0 002 2h.096c.5 0 .905-.405.905-.904 0-.715.211-1.413.608-2.008L17 13V4m-7 10h2m5-10h2a2 2 0 012 2v6a2 2 0 01-2 2h-2.5"
          />
        </svg>
        {t('chat:message.wrong')}
      </button>
    );
  }

  const submit = () => {
    const trimmed = reason.trim();
    if (!trimmed || disabled) return;
    onReject(trimmed);
    setOpen(false);
    setReason('');
  };

  return (
    <div className="mt-2 rounded-lg border border-gray-300 bg-white p-2">
      <label className="mb-1 block text-xs text-gray-600" htmlFor="chat-wrong-reason">
        {t('chat:message.wrongPrompt')}
      </label>
      <textarea
        id="chat-wrong-reason"
        data-testid="chat-wrong-reason"
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) submit();
        }}
        rows={2}
        ref={textareaRef}
        placeholder={t('chat:message.wrongPlaceholder')}
        className="w-full resize-y rounded border border-gray-300 px-2 py-1 text-xs text-gray-900 focus:border-primary-500 focus:outline-none"
      />
      <div className="mt-1.5 flex justify-end gap-2">
        <button
          type="button"
          onClick={() => {
            setOpen(false);
            setReason('');
          }}
          className="rounded px-2 py-1 text-xs text-gray-600 hover:bg-gray-100"
        >
          {t('common:actions.cancel')}
        </button>
        <button
          type="button"
          data-testid="chat-wrong-submit"
          onClick={submit}
          disabled={disabled || reason.trim().length === 0}
          className="rounded bg-primary-600 px-2.5 py-1 text-xs font-medium text-white transition-colors hover:bg-primary-700 disabled:opacity-50"
        >
          {t('chat:message.wrongSubmit')}
        </button>
      </div>
    </div>
  );
}

export function MessageBubble({
  message,
  isStreaming,
  phase,
  accountId,
  onOpenEmail,
  onShowEmailsInList,
  onReject,
  isRejected,
  isSending,
}: MessageBubbleProps) {
  const { t } = useTranslation(['chat']);
  const isUser = message.role === 'user';
  const addLog = useLogStore((s) => s.addLog);
  const referencedEmailIds = isUser ? [] : collectReferencedEmailIds(message);

  const handleOpenAttachment = async (ns: 'meta' | 'attach', id: string) => {
    try {
      if (ns === 'meta') {
        await api.openEmailAttachmentMeta(accountId, id);
      } else {
        await api.openAttachmentExternally(accountId, id);
      }
    } catch (err) {
      // Try the other namespace as a fallback — the tool's preference for
      // `meta` over `attach` can miss when the meta row was never materialized
      // for a rule-matched download, and vice versa.
      try {
        if (ns === 'meta') {
          await api.openAttachmentExternally(accountId, id);
        } else {
          await api.openEmailAttachmentMeta(accountId, id);
        }
      } catch (err2) {
        addLog('error', 'chat', `Failed to open attachment: ${err2 ?? err}`);
      }
    }
  };

  // Reasoning models emit their scratchpad inline as <think>...</think>. Split
  // it out so we can render the actual answer as markdown and the reasoning as
  // a collapsible section — but only if there's something inside the tags.
  const { thinking, answer, thinkingComplete } = isUser
    ? { thinking: '', answer: message.content, thinkingComplete: true }
    : splitThinking(message.content);
  const trimmedThinking = thinking.trim();
  const hasThinking = trimmedThinking.length > 0;
  const hasAnswer = answer.trim().length > 0;
  // Show typing dots while nothing visible has arrived yet (covers the case
  // where the model has only emitted "<think>" so far with no body).
  const showTypingDots = !isUser && isStreaming && !hasAnswer && !hasThinking;

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'} my-2`}>
      <div
        className={`max-w-[80%] rounded-2xl px-4 py-3 text-sm leading-relaxed break-words ${
          isUser ? 'bg-primary-600 text-white whitespace-pre-wrap' : 'bg-gray-100 text-gray-900 border border-gray-200'
        }`}
      >
        {isUser ? (
          message.content
        ) : (
          <>
            {showTypingDots &&
              (phase ? (
                <ProcessingStatus phase={phase} />
              ) : (
                <span className="inline-flex items-center gap-1 text-gray-500">
                  <span className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce" />
                  <span className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce [animation-delay:150ms]" />
                  <span className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce [animation-delay:300ms]" />
                </span>
              ))}
            {hasThinking && <ThinkingSection text={trimmedThinking} streaming={!thinkingComplete && isStreaming} />}
            {hasAnswer && (
              <MarkdownContent
                content={answer}
                sources={message.sources}
                accountId={accountId}
                onOpenEmail={onOpenEmail}
                onOpenAttachment={handleOpenAttachment}
                emailRefAllowlist={message.referencedEmailIds}
                draftRefAllowlist={message.referencedDraftIds}
              />
            )}
            {isStreaming && hasAnswer && (
              <span className="inline-block w-1.5 h-4 ml-0.5 bg-gray-500 align-middle animate-pulse" />
            )}
            {!isStreaming && <StatsFooter message={message} />}
            {!isStreaming && <SourcesList sources={message.sources} accountId={accountId} onOpenEmail={onOpenEmail} />}
            {!isStreaming && onShowEmailsInList && referencedEmailIds.length > 0 && (
              <button
                type="button"
                data-testid="chat-show-in-list"
                onClick={() => onShowEmailsInList(buildIdSearchQuery(referencedEmailIds))}
                className="mt-2 inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-primary-600 text-xs font-medium text-white hover:bg-primary-700 transition-colors"
              >
                <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h10" />
                </svg>
                {t('chat:sources.showInList', { count: referencedEmailIds.length })}
              </button>
            )}
            {!isStreaming && message.trace && <ReasoningSection trace={message.trace} />}
            {!isStreaming && hasAnswer && onReject && !isRejected && (
              <WrongAnswerControl onReject={onReject} disabled={Boolean(isSending)} />
            )}
            {isRejected && (
              <div data-testid="chat-rejected-note" className="mt-2 text-xs italic text-amber-700">
                {t('chat:message.rejected')}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
