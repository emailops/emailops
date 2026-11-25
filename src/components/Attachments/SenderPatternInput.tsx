import { useId, useRef, useState } from 'react';
import type { SenderSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { replaceSenderEntry, type SenderEntry, senderEntryAt } from '@/lib/senderPattern';
import { useLogStore } from '@/stores/logStore';

/** Shorter entries match too many senders to be worth a lookup. */
const MIN_CHARS = 2;
const MAX_SUGGESTIONS = 8;

interface SenderPatternInputProps {
  value: string;
  onChange: (value: string) => void;
  /** The rule's account: its senders are the ones suggested. */
  accountId: string;
  inputId?: string;
  placeholder?: string;
}

/**
 * The sender field of an attachment rule. As you type, it suggests the
 * senders of the rule's account (your own address included, for mail you
 * sent) for the comma-separated entry under the cursor; picking one replaces
 * that entry only. Typing a wildcard pattern stays possible: nothing is picked
 * unless a suggestion is highlighted or clicked.
 */
export function SenderPatternInput({ value, onChange, accountId, inputId, placeholder }: SenderPatternInputProps) {
  const addLog = useLogStore((s) => s.addLog);
  const [suggestions, setSuggestions] = useState<SenderSuggestion[]>([]);
  const [entry, setEntry] = useState<SenderEntry | null>(null);
  // Keyboard-highlighted suggestion (-1 = none).
  const [active, setActive] = useState(-1);
  // Id of the latest lookup: an answer for earlier text is dropped.
  const lookupRef = useRef(0);
  const listId = useId();
  const showList = suggestions.length > 0;
  const optionId = (index: number) => `${listId}-option-${index}`;

  const close = () => {
    lookupRef.current++;
    setSuggestions([]);
    setActive(-1);
  };

  const lookUp = async (next: string, cursor: number) => {
    const found = senderEntryAt(next, cursor);
    const lookup = ++lookupRef.current;
    if (found.text.length < MIN_CHARS) {
      setSuggestions([]);
      setActive(-1);
      return;
    }
    try {
      const senders = await api.autocompleteSenders(accountId, found.text, MAX_SUGGESTIONS);
      if (lookup !== lookupRef.current) return;
      setEntry(found);
      setSuggestions(senders);
      setActive(-1);
    } catch (err) {
      if (lookup !== lookupRef.current) return;
      setSuggestions([]);
      addLog('error', 'attachments', `Failed to suggest senders: ${errorText(err)}`);
    }
  };

  const pick = (suggestion: SenderSuggestion) => {
    if (entry) onChange(replaceSenderEntry(value, entry, suggestion.email));
    close();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (!showList) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const step = e.key === 'ArrowDown' ? 1 : -1;
      setActive((prev) => (prev + step + suggestions.length) % suggestions.length);
    } else if (e.key === 'Enter' && active >= 0) {
      e.preventDefault();
      pick(suggestions[active]);
    } else if (e.key === 'Escape') {
      e.stopPropagation(); // close the list, not the dialog
      close();
    }
  };

  return (
    <div className="relative">
      <input
        id={inputId}
        type="text"
        role="combobox"
        aria-expanded={showList}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={showList && active >= 0 ? optionId(active) : undefined}
        autoComplete="off"
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          void lookUp(e.target.value, e.target.selectionStart ?? e.target.value.length);
        }}
        onBlur={close}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
      />
      {showList && (
        <div
          id={listId}
          role="listbox"
          className="absolute z-10 mt-1 w-full max-h-48 overflow-y-auto bg-white border border-gray-200 rounded-lg shadow-lg py-1"
        >
          {suggestions.map((s, index) => (
            <div
              key={s.email}
              id={optionId(index)}
              role="option"
              aria-selected={active === index}
              tabIndex={-1}
              // mousedown, not click: it fires before the input's blur closes the list.
              onMouseDown={(e) => {
                e.preventDefault();
                pick(s);
              }}
              className={`px-3 py-1.5 text-sm cursor-pointer hover:bg-gray-100 ${active === index ? 'bg-gray-100' : ''}`}
            >
              <span className="text-gray-900">{s.email}</span>
              {s.name && s.name !== s.email && <span className="ml-2 text-xs text-gray-500">{s.name}</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
