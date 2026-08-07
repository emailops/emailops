import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import type { SenderSuggestion } from '@/lib/api';
import * as api from '@/lib/api';

/** Detect a from:/to: autocomplete trigger at the cursor position. */
function detectAutocompleteTrigger(
  text: string,
  cursor: number,
): { field: 'from' | 'to'; prefix: string; tokenStart: number } | null {
  const textUpToCursor = text.slice(0, cursor);
  const match = /(?:^|\s)(from|to):\s*(\S*)$/i.exec(textUpToCursor);
  if (!match) return null;
  const leadingWs = match[0].length - match[0].trimStart().length;
  return {
    field: match[1].toLowerCase() as 'from' | 'to',
    prefix: match[2],
    tokenStart: match.index + leadingWs,
  };
}

interface InboxSearchBoxProps {
  /** Account ID used for sender autocomplete. */
  accountId?: string | null;
  /** Externally-controlled query value (used to sync local input when search is cleared). */
  externalQuery: string;
  /** Submit handler (non-empty trimmed query). */
  onSubmit: (query: string) => void;
  /** Clear handler (empty submission). */
  onClear: () => void;
}

/**
 * Inline search form with from:/to: sender autocomplete, used in the inbox
 * header for the full-width layout. Encapsulates the autocomplete dropdown
 * portal and keyboard navigation so the parent only deals with submit/clear.
 */
export function InboxSearchBox({ accountId, externalQuery, onSubmit, onClear }: InboxSearchBoxProps) {
  const { t } = useTranslation(['inbox']);
  const [localQuery, setLocalQuery] = useState(externalQuery);
  // Keep local input in sync when search is cleared externally
  useEffect(() => {
    setLocalQuery(externalQuery);
  }, [externalQuery]);

  const searchInputRef = useRef<HTMLInputElement>(null);
  const searchContainerRef = useRef<HTMLDivElement>(null);
  const autocompleteReqRef = useRef(0);
  const [autocomplete, setAutocomplete] = useState<{
    field: 'from' | 'to';
    prefix: string;
    tokenStart: number;
    suggestions: SenderSuggestion[];
    selectedIndex: number;
    pos: { top: number; left: number; width: number };
  } | null>(null);

  const fetchSuggestions = useCallback(
    async (field: 'from' | 'to', prefix: string, tokenStart: number) => {
      if (!accountId) return;
      const rect = searchContainerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const pos = { top: rect.bottom + 4, left: rect.left, width: rect.width };
      const reqId = autocompleteReqRef.current + 1;
      autocompleteReqRef.current = reqId;
      try {
        const suggestions = await api.autocompleteSenders(accountId, prefix, 8);
        if (autocompleteReqRef.current !== reqId) return;
        if (suggestions.length === 0) {
          setAutocomplete(null);
          return;
        }
        setAutocomplete({ field, prefix, tokenStart, suggestions, selectedIndex: 0, pos });
      } catch {
        if (autocompleteReqRef.current === reqId) setAutocomplete(null);
      }
    },
    [accountId],
  );

  const applySuggestion = useCallback(
    (suggestion: SenderSuggestion) => {
      if (!autocomplete) return;
      const before = localQuery.slice(0, autocomplete.tokenStart);
      const after = localQuery.slice(searchInputRef.current?.selectionStart ?? localQuery.length);
      const inserted = `${autocomplete.field}:${suggestion.email} `;
      const newValue = `${before}${inserted}${after}`;
      setLocalQuery(newValue);
      setAutocomplete(null);
      const q = newValue.trim();
      if (q) onSubmit(q);
    },
    [autocomplete, localQuery, onSubmit],
  );

  const handleSearchChange = useCallback(
    (value: string) => {
      setLocalQuery(value);
      const cursor = searchInputRef.current?.selectionStart ?? value.length;
      const trigger = detectAutocompleteTrigger(value, cursor);
      if (trigger) {
        fetchSuggestions(trigger.field, trigger.prefix, trigger.tokenStart);
      } else {
        setAutocomplete(null);
      }
    },
    [fetchSuggestions],
  );

  const handleSearchKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (autocomplete) {
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          setAutocomplete((prev) =>
            prev ? { ...prev, selectedIndex: (prev.selectedIndex + 1) % prev.suggestions.length } : null,
          );
          return;
        }
        if (e.key === 'ArrowUp') {
          e.preventDefault();
          setAutocomplete((prev) =>
            prev
              ? { ...prev, selectedIndex: (prev.selectedIndex - 1 + prev.suggestions.length) % prev.suggestions.length }
              : null,
          );
          return;
        }
        if (e.key === 'Tab' && autocomplete.suggestions.length > 0) {
          e.preventDefault();
          applySuggestion(autocomplete.suggestions[autocomplete.selectedIndex]);
          return;
        }
        if (e.key === 'Enter' && autocomplete.suggestions.length > 0) {
          if (autocomplete.selectedIndex > 0 || autocomplete.prefix.length < 5) {
            e.preventDefault();
            applySuggestion(autocomplete.suggestions[autocomplete.selectedIndex]);
            return;
          }
          setAutocomplete(null);
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          setAutocomplete(null);
          return;
        }
      }
      if (e.key === 'Escape' && !autocomplete) {
        setLocalQuery('');
        onClear();
      }
    },
    [autocomplete, applySuggestion, onClear],
  );

  return (
    <>
      <form
        className="w-[50ch] max-w-full"
        onSubmit={(e) => {
          e.preventDefault();
          const q = localQuery.trim();
          setAutocomplete(null);
          if (q) onSubmit(q);
          else onClear();
        }}
      >
        <div
          ref={searchContainerRef}
          className="flex items-center bg-gray-100 hover:bg-gray-200 focus-within:bg-white focus-within:ring-1 focus-within:ring-primary-400 rounded-lg px-2 py-1.5 gap-1.5 transition-colors dark:bg-surface-hover dark:hover:bg-gray-700 dark:focus-within:bg-surface"
        >
          <svg
            className="w-3.5 h-3.5 text-gray-400 flex-shrink-0 dark:text-gray-500"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
            />
          </svg>
          <input
            ref={searchInputRef}
            value={localQuery}
            onChange={(e) => handleSearchChange(e.target.value)}
            onKeyDown={handleSearchKeyDown}
            placeholder={t('inbox:searchBox.inlinePlaceholder', { operators: 'from:, to:, subject:' })}
            className="flex-1 bg-transparent text-sm outline-none min-w-0 placeholder-gray-400 dark:placeholder-gray-500"
          />
          {localQuery && (
            <button
              type="button"
              onClick={() => {
                setLocalQuery('');
                setAutocomplete(null);
                onClear();
              }}
              className="text-gray-400 hover:text-gray-600 flex-shrink-0 dark:text-gray-500 dark:hover:text-gray-400"
            >
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}
        </div>
      </form>

      {autocomplete &&
        autocomplete.suggestions.length > 0 &&
        createPortal(
          <div
            className="fixed z-[200] bg-white rounded-lg shadow-lg border border-gray-200 overflow-hidden dark:bg-surface dark:border-gray-700"
            style={{ top: autocomplete.pos.top, left: autocomplete.pos.left, width: autocomplete.pos.width }}
          >
            {autocomplete.suggestions.map((s, idx) => (
              <button
                key={s.email}
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  applySuggestion(s);
                }}
                onMouseEnter={() => setAutocomplete((prev) => (prev ? { ...prev, selectedIndex: idx } : null))}
                className={`w-full text-left px-3 py-2 flex items-center gap-2 text-sm ${
                  idx === autocomplete.selectedIndex
                    ? 'bg-primary-50 dark:bg-primary-900/20'
                    : 'hover:bg-gray-50 dark:hover:bg-surface-raised'
                }`}
              >
                <svg
                  className="w-3.5 h-3.5 text-gray-400 flex-shrink-0 dark:text-gray-500"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M16 12a4 4 0 10-8 0 4 4 0 008 0zm0 0v1.5a2.5 2.5 0 005 0V12a9 9 0 10-9 9m4.5-1.206a8.959 8.959 0 01-4.5 1.207"
                  />
                </svg>
                <div className="flex-1 min-w-0">
                  {s.name && s.name !== s.email && (
                    <div className="font-medium text-gray-900 truncate dark:text-gray-100">{s.name}</div>
                  )}
                  <div
                    className={`${s.name && s.name !== s.email ? 'text-xs text-gray-500 dark:text-gray-400' : 'text-gray-900 dark:text-gray-100'} truncate`}
                  >
                    {s.email}
                  </div>
                </div>
              </button>
            ))}
            <div className="px-3 py-1 text-[10px] text-gray-400 border-t border-gray-100 bg-gray-50 dark:text-gray-500 dark:border-gray-800 dark:bg-surface-raised">
              ↑↓ navigate · ↵ / Tab select · Esc dismiss
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
