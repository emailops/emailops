import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAutoGrow } from '@/hooks/useAutoGrow';
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
    if (!trimmed || disabled) return;
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
      <div className={`flex items-end ${compact ? 'gap-2' : 'gap-3'}`}>
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={onKeyDown}
          rows={compact ? 1 : 2}
          placeholder={placeholder ?? 'Ask about your emails… (Enter to send, Shift+Enter for newline)'}
          disabled={disabled}
          className="flex-1 resize-none rounded-lg border border-gray-300 px-3 py-2 text-sm focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-100 disabled:bg-gray-50"
        />
        <button
          type="button"
          onClick={submit}
          disabled={disabled || value.trim().length === 0}
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
