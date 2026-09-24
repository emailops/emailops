import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAutoGrow } from '@/hooks/useAutoGrow';
import { formatDuration } from '@/lib/researchTime';
import { useChatStore } from '@/stores/chatStore';
import { CategoryFilterDropdown } from './CategoryFilterDropdown';

/** Arms research mode for the next message: the backend reads many more
 *  emails in batches and writes a detailed report, at the cost of minutes.
 *  Per message on purpose — the send disarms it (see `chatStore.dispatchTurn`),
 *  so the chat never stays slow by accident. */
function ResearchToggle() {
  const { t } = useTranslation(['chat']);
  const researchMode = useChatStore((s) => s.researchMode);
  const setResearchMode = useChatStore((s) => s.setResearchMode);
  return (
    <button
      type="button"
      data-testid="chat-research-toggle"
      aria-pressed={researchMode}
      onClick={() => setResearchMode(!researchMode)}
      title={t('chat:research.title')}
      className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-md border text-xs transition-colors ${
        researchMode
          ? 'border-primary-300 bg-primary-50 text-primary-700'
          : 'border-gray-200 bg-white text-gray-600 hover:border-gray-300 hover:bg-gray-50'
      }`}
    >
      <svg
        className="w-3.5 h-3.5"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-hidden="true"
      >
        <circle cx="11" cy="11" r="7" />
        <path d="M21 21l-4.35-4.35M8 11h6M11 8v6" strokeLinecap="round" />
      </svg>
      <span className="font-medium">{t('chat:research.toggle')}</span>
    </button>
  );
}

interface ChatInputProps {
  onSend: (content: string) => void;
  disabled: boolean;
  placeholder?: string;
  /** When `prefillNonce` changes, the textarea's value is replaced with
   *  `prefillText` and focus moves to the caret position at the end of the
   *  text. Used by the "Write a draft" shortcut chip so the user can
   *  finish the sentence rather than have the model auto-send. The nonce
   *  lets the parent re-apply the same text (e.g. click the chip twice). */
  prefillText?: string;
  prefillNonce?: number;
  /** Tighter padding + single-row floor for the narrow right-hand chat panel. */
  compact?: boolean;
  /** Rendered directly above the textarea — the panel's context chip slot. */
  contextSlot?: ReactNode;
}

/** The estimate of a research question, for the user to start or drop before
 *  anything is sent: how many emails, in how many batches, about how long. */
function ResearchConfirm() {
  const { t } = useTranslation(['chat']);
  const pending = useChatStore((s) => s.pendingResearch);
  const confirmResearch = useChatStore((s) => s.confirmResearch);
  const cancelResearch = useChatStore((s) => s.cancelResearch);
  if (!pending) return null;
  const estimate = pending.estimate;
  const canStart = pending.status === 'ready' && estimate != null && estimate.emails > 0;
  let body: ReactNode;
  if (pending.status === 'estimating') {
    body = <span className="text-gray-600">{t('chat:research.estimating')}</span>;
  } else if (pending.status === 'error') {
    body = <span className="text-red-700">{t('chat:research.estimateFailed', { error: pending.error ?? '' })}</span>;
  } else if (estimate && estimate.emails === 0) {
    body = <span className="text-gray-700">{t('chat:research.estimateNone')}</span>;
  } else if (estimate) {
    const filter = estimate.filter
      ? Object.entries(estimate.filter)
          .map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v)}`)
          .join(', ')
      : null;
    body = (
      <>
        <div className="text-gray-900">
          {t('chat:research.estimate', {
            emails: estimate.emails.toLocaleString(),
            batches: estimate.batches.toLocaleString(),
            time: formatDuration(estimate.seconds),
          })}
        </div>
        <div className="text-xs text-gray-600">
          {filter ? `${t('chat:research.filter')}: ${filter}` : t('chat:research.byMeaning')}
        </div>
      </>
    );
  }
  return (
    <div
      data-testid="research-confirm"
      className="mb-2 rounded-lg border border-primary-200 bg-primary-50 px-3 py-2 text-sm"
    >
      <div className="mb-1 truncate text-xs text-gray-500">“{pending.content}”</div>
      {body}
      <div className="mt-2 flex gap-2">
        {canStart && (
          <button
            type="button"
            data-testid="research-start"
            onClick={() => void confirmResearch()}
            className="rounded-md bg-primary-600 px-3 py-1 text-xs font-medium text-white hover:bg-primary-700"
          >
            {t('chat:research.start')}
          </button>
        )}
        <button
          type="button"
          data-testid="research-cancel"
          onClick={cancelResearch}
          className="rounded-md border border-gray-300 bg-white px-3 py-1 text-xs text-gray-700 hover:bg-gray-50"
        >
          {t('chat:research.cancel')}
        </button>
      </div>
    </div>
  );
}

function ResearchHint() {
  const { t } = useTranslation(['chat']);
  const researchMode = useChatStore((s) => s.researchMode);
  if (!researchMode) return null;
  return <span className="text-xs text-gray-500">{t('chat:research.hint')}</span>;
}

export function ChatInput({
  onSend,
  disabled,
  placeholder,
  prefillText,
  prefillNonce,
  compact = false,
  contextSlot,
}: ChatInputProps) {
  const [value, setValue] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  // A research question waits for its estimate to be confirmed: no new send
  // until the user starts or cancels it.
  const researchPending = useChatStore((s) => s.pendingResearch !== null);
  const inputPrefill = useChatStore((s) => s.inputPrefill);
  const isDisabled = disabled || researchPending;

  // A cancelled research question comes back to the input for editing.
  useEffect(() => {
    if (inputPrefill) setValue(inputPrefill.text);
  }, [inputPrefill]);

  // Grow with the prompt (rows={2} sets the floor) and shrink back on send;
  // past the cap the textarea scrolls internally instead of pushing the
  // conversation off-screen.
  useAutoGrow(textareaRef, value);

  // Sync external prefills into local state. Only fires when the nonce
  // changes so typing locally doesn't clash with stale text.
  useEffect(() => {
    if (prefillNonce === undefined || prefillText === undefined) return;
    setValue(prefillText);
    // Focus + place caret at the end on the next tick so the user can keep
    // typing where the sentence trails off.
    const el = textareaRef.current;
    if (el) {
      requestAnimationFrame(() => {
        el.focus();
        const end = prefillText.length;
        el.setSelectionRange(end, end);
      });
    }
  }, [prefillNonce, prefillText]);

  const submit = () => {
    const trimmed = value.trim();
    if (!trimmed || isDisabled) return;
    onSend(trimmed);
    setValue('');
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className={`border-t border-gray-200 bg-white ${compact ? 'px-3 py-3' : 'px-6 py-4'}`}>
      {contextSlot}
      <ResearchConfirm />
      <div className={`flex items-end ${compact ? 'gap-2' : 'gap-3'}`}>
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={onKeyDown}
          rows={compact ? 1 : 2}
          placeholder={placeholder ?? 'Ask about your emails… (Enter to send, Shift+Enter for newline)'}
          disabled={isDisabled}
          className="flex-1 resize-none rounded-lg border border-gray-300 px-3 py-2 text-sm focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-100 disabled:bg-gray-50"
        />
        <button
          type="button"
          onClick={submit}
          disabled={isDisabled || value.trim().length === 0}
          className={`bg-primary-600 text-white font-medium rounded-lg hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors ${
            compact ? 'px-3 py-2 text-xs' : 'px-4 py-2 text-sm'
          }`}
        >
          Send
        </button>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <CategoryFilterDropdown />
        <ResearchToggle />
        <ResearchHint />
      </div>
    </div>
  );
}
