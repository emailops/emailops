import { type KeyboardEvent, useEffect, useRef, useState } from 'react';
import { useAutoGrow } from '@/hooks/useAutoGrow';
import { CategoryFilterDropdown } from './CategoryFilterDropdown';

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
}

export function ChatInput({ onSend, disabled, placeholder, prefillText, prefillNonce }: ChatInputProps) {
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
    <div className="border-t border-gray-200 bg-white px-6 py-4">
      <div className="flex gap-3 items-end">
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={onKeyDown}
          rows={2}
          placeholder={placeholder ?? 'Ask about your emails… (Enter to send, Shift+Enter for newline)'}
          disabled={disabled}
          className="flex-1 resize-none rounded-lg border border-gray-300 px-3 py-2 text-sm focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-100 disabled:bg-gray-50"
        />
        <button
          type="button"
          onClick={submit}
          disabled={disabled || value.trim().length === 0}
          className="px-4 py-2 bg-primary-600 text-white text-sm font-medium rounded-lg hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          Send
        </button>
      </div>
      <div className="mt-2 flex items-center">
        <CategoryFilterDropdown />
      </div>
    </div>
  );
}
