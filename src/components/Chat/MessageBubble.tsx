import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import type { ChatMessage, ChatPhase } from '@/types';
import { MarkdownContent } from './MarkdownContent';
import { ReasoningSection, StatsFooter } from './ReasoningTrace';
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
}

/** LM Studio-style "Processing…" status: a spinner plus a localized label for
 *  the stage the backend just entered (routing → retrieving → tools →
 *  generating). Shown in place of the bare typing dots once the backend tells
 *  us what it's doing, so a slow prompt-processing pass reads as progress
 *  rather than a hang. */
function ProcessingStatus({ phase }: { phase: ChatPhase }) {
  const { t } = useTranslation(['chat']);
  return (
    <span className="inline-flex items-center gap-2 text-gray-500 dark:text-gray-400">
      <svg className="w-3.5 h-3.5 animate-spin text-gray-400 dark:text-gray-500" viewBox="0 0 24 24" fill="none">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
      <span>{t(`chat:processing.${phase}` as const)}</span>
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
      className="mb-2 rounded-lg border border-gray-200 bg-gray-50/70 text-xs dark:border-gray-700 dark:bg-surface-raised/70"
      // `key` forces a re-mount when the streaming flag flips so the `open`
      // attribute is re-applied (browsers ignore changes to `open` on existing
      // <details> elements that the user has interacted with — re-mount is the
      // simplest way to keep the default state honest).
      open={streaming || undefined}
    >
      <summary className="cursor-pointer select-none px-2.5 py-1.5 text-gray-600 font-medium flex items-center gap-1.5 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200">
        <svg
          className={`w-3.5 h-3.5 text-gray-400 dark:text-gray-500 ${streaming ? 'animate-pulse' : ''}`}
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
      <pre className="px-2.5 pb-2 pt-1 whitespace-pre-wrap font-sans text-[11px] leading-relaxed text-gray-600 dark:text-gray-400">
        {text}
      </pre>
    </details>
  );
}

export function MessageBubble({ message, isStreaming, phase, accountId, onOpenEmail }: MessageBubbleProps) {
  const isUser = message.role === 'user';
  const addLog = useLogStore((s) => s.addLog);

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
          isUser
            ? 'bg-primary-600 text-white whitespace-pre-wrap'
            : 'bg-gray-100 text-gray-900 border border-gray-200 dark:bg-surface-hover dark:text-gray-100 dark:border-gray-700'
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
                <span className="inline-flex items-center gap-1 text-gray-500 dark:text-gray-400">
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
            {!isStreaming && message.trace && <ReasoningSection trace={message.trace} />}
          </>
        )}
      </div>
    </div>
  );
}
