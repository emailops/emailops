import { type ReactElement, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ParsedSearchQuery, SearchMethod, SearchResult, SenderSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { formatDate as formatDateIntl, formatTime as formatTimeIntl } from '@/lib/intl';
import { selectEffectiveAccountId, useAccountStore } from '@/stores/accountStore';
import type { Email, EmailCategory } from '@/types';

/** Detect an autocomplete trigger (`from:` or `to:` token) at the cursor position.
 *  Returns the field, the prefix typed so far, and the token's start index.
 *  Handles optional whitespace after the colon (e.g. `from: jo` as well as `from:jo`). */
function detectAutocompleteTrigger(
  text: string,
  cursor: number,
): { field: 'from' | 'to'; prefix: string; tokenStart: number } | null {
  const textUpToCursor = text.slice(0, cursor);
  // Match (from|to): followed by optional whitespace, then zero-or-more non-whitespace chars at end.
  // The trailing \S* must reach the end of the text-up-to-cursor, ensuring we're still typing the value.
  const match = /(?:^|\s)(from|to):\s*(\S*)$/i.exec(textUpToCursor);
  if (!match) return null;
  // Compute start of the 'from:' / 'to:' token, skipping any leading whitespace in the match.
  const leadingWs = match[0].length - match[0].trimStart().length;
  return {
    field: match[1].toLowerCase() as 'from' | 'to',
    prefix: match[2], // \S* guarantees no surrounding whitespace
    tokenStart: match.index + leadingWs,
  };
}

interface SearchBarProps {
  /** Query scope: an account id, or null for the unified ("All accounts")
   *  view — the backend then searches every enabled account. */
  accountId: string | null;
  onSelectEmail: (email: Email) => void;
  onApplySearch: (query: string) => void;
  /** Apply search with pre-fetched results to avoid a duplicate backend call */
  onApplySearchWithResults: (query: string, emails: Email[]) => void;
  selectedCategories: EmailCategory[];
  onClose: () => void;
}

export function SearchBar({
  accountId,
  onSelectEmail,
  onApplySearch,
  onApplySearchWithResults,
  selectedCategories,
  onClose,
}: SearchBarProps) {
  const { t, i18n } = useTranslation(['common', 'inbox']);
  const locale = i18n.language || 'en';
  // `accountId === null` is the unified view (search all enabled accounts) —
  // only a truly empty account list blocks searching. Sender autocomplete
  // needs one concrete account, so it falls back to the first enabled one.
  const hasAccounts = useAccountStore((s) => s.accounts.length > 0);
  const effectiveAccountId = useAccountStore((s) => selectEffectiveAccountId(s.accounts, s.activeAccountId));
  const [query, setQuery] = useState('');
  const [isSearching, setIsSearching] = useState(false);
  const [results, setResults] = useState<SearchResult | null>(null);
  /** Query string the current results correspond to (so we can reuse them on submit) */
  const [resultsQuery, setResultsQuery] = useState<string | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestRequestIdRef = useRef(0);
  /** The query and promise for the currently in-flight fetch (if any) */
  const inFlightRef = useRef<{ query: string; promise: Promise<SearchResult> } | null>(null);

  // Autocomplete state for from:/to: tokens
  const [autocomplete, setAutocomplete] = useState<{
    field: 'from' | 'to';
    prefix: string;
    tokenStart: number;
    suggestions: SenderSuggestion[];
    selectedIndex: number;
  } | null>(null);
  const autocompleteReqRef = useRef(0);

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Handle keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  useEffect(() => {
    return () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
    };
  }, []);

  const performSearch = useCallback(
    async (searchQuery: string): Promise<SearchResult | null> => {
      if (!searchQuery.trim()) {
        setResults(null);
        setResultsQuery(null);
        setSearchError(null);
        setIsSearching(false);
        return null;
      }

      if (!accountId && !hasAccounts) {
        setResults(null);
        setResultsQuery(null);
        setSearchError('Select an account before searching.');
        setIsSearching(false);
        return null;
      }

      // Reuse an in-flight fetch for the same query
      if (inFlightRef.current && inFlightRef.current.query === searchQuery) {
        return inFlightRef.current.promise;
      }

      const requestId = latestRequestIdRef.current + 1;
      latestRequestIdRef.current = requestId;
      setIsSearching(true);
      setSearchError(null);

      const promise = api.searchEmails(accountId, searchQuery, true, selectedCategories);
      inFlightRef.current = { query: searchQuery, promise };

      try {
        const result = await promise;

        if (latestRequestIdRef.current !== requestId) {
          return result;
        }

        setResults(result);
        setResultsQuery(searchQuery);
        return result;
      } catch (error) {
        if (latestRequestIdRef.current !== requestId) {
          return null;
        }

        setResults(null);
        setResultsQuery(null);
        setSearchError(errorText(error));
        return null;
      } finally {
        if (latestRequestIdRef.current === requestId) {
          setIsSearching(false);
        }
        // Clear in-flight ref if it still points to this request
        if (inFlightRef.current && inFlightRef.current.promise === promise) {
          inFlightRef.current = null;
        }
      }
    },
    [accountId, hasAccounts, selectedCategories],
  );

  /** Fetch sender suggestions for the current autocomplete trigger */
  const fetchSuggestions = useCallback(
    async (field: 'from' | 'to', prefix: string, tokenStart: number) => {
      const autocompleteAccountId = accountId ?? effectiveAccountId;
      if (!autocompleteAccountId) return;
      const reqId = autocompleteReqRef.current + 1;
      autocompleteReqRef.current = reqId;
      try {
        const suggestions = await api.autocompleteSenders(autocompleteAccountId, prefix, 8);
        if (autocompleteReqRef.current !== reqId) return;
        if (suggestions.length === 0) {
          setAutocomplete(null);
          return;
        }
        setAutocomplete({ field, prefix, tokenStart, suggestions, selectedIndex: 0 });
      } catch {
        if (autocompleteReqRef.current === reqId) setAutocomplete(null);
      }
    },
    [accountId, effectiveAccountId],
  );

  const handleInputChange = useCallback(
    (value: string) => {
      setQuery(value);

      // Check for autocomplete trigger at cursor position
      const cursor = inputRef.current?.selectionStart ?? value.length;
      const trigger = detectAutocompleteTrigger(value, cursor);
      if (trigger) {
        fetchSuggestions(trigger.field, trigger.prefix.trim(), trigger.tokenStart);
      } else {
        setAutocomplete(null);
      }

      // Debounce search — wait until user stops typing (800ms)
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }

      if (!value.trim()) {
        setResults(null);
        setSearchError(null);
        return;
      }

      debounceRef.current = setTimeout(() => {
        performSearch(value);
      }, 800);
    },
    [performSearch, fetchSuggestions],
  );

  /** Insert the selected sender into the query, replacing the autocomplete token */
  const applySuggestion = useCallback(
    (suggestion: SenderSuggestion) => {
      if (!autocomplete) return;
      const before = query.slice(0, autocomplete.tokenStart);
      const after = query.slice(inputRef.current?.selectionStart ?? query.length);
      const inserted = `${autocomplete.field}:${suggestion.email} `;
      const newValue = `${before}${inserted}${after}`;
      setQuery(newValue);
      setAutocomplete(null);
      // Restore focus and place cursor right after the inserted text
      queueMicrotask(() => {
        const input = inputRef.current;
        if (!input) return;
        const pos = before.length + inserted.length;
        input.focus();
        input.setSelectionRange(pos, pos);
      });
    },
    [autocomplete, query],
  );

  /** Keyboard navigation for autocomplete dropdown */
  const handleInputKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (!autocomplete) return;
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setAutocomplete((prev) =>
          prev ? { ...prev, selectedIndex: (prev.selectedIndex + 1) % prev.suggestions.length } : null,
        );
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setAutocomplete((prev) =>
          prev
            ? {
                ...prev,
                selectedIndex: (prev.selectedIndex - 1 + prev.suggestions.length) % prev.suggestions.length,
              }
            : null,
        );
      } else if (e.key === 'Tab' && autocomplete.suggestions.length > 0) {
        e.preventDefault();
        applySuggestion(autocomplete.suggestions[autocomplete.selectedIndex]);
      } else if (e.key === 'Enter' && autocomplete.suggestions.length > 0) {
        // Only apply suggestion if user actively navigated (not index 0) or prefix is short
        // Otherwise let Enter fall through to form submit for complete typed emails
        if (autocomplete.selectedIndex > 0 || autocomplete.prefix.length < 5) {
          e.preventDefault();
          applySuggestion(autocomplete.suggestions[autocomplete.selectedIndex]);
        } else {
          // Dismiss autocomplete and let form submit
          setAutocomplete(null);
        }
      } else if (e.key === 'Escape') {
        e.preventDefault();
        setAutocomplete(null);
      }
    },
    [autocomplete, applySuggestion],
  );

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      // If autocomplete dropdown is showing, Enter already got intercepted by onKeyDown.
      // If it wasn't (e.g., no suggestions), fall through to normal submit.
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      const trimmed = query.trim();
      if (!trimmed) return;

      // Case 1: We already have fresh results for this query — reuse immediately.
      if (results && resultsQuery === trimmed) {
        onApplySearchWithResults(trimmed, results.emails);
        onClose();
        return;
      }

      // Case 2: A fetch is in flight for this query — await it and reuse its results.
      //         This prevents triggering a second identical backend call.
      if (inFlightRef.current && inFlightRef.current.query === trimmed) {
        const pending = inFlightRef.current.promise;
        // Close modal immediately for perceived responsiveness;
        // the results will flow into the inbox when the fetch resolves.
        onClose();
        try {
          const result = await pending;
          onApplySearchWithResults(trimmed, result.emails);
        } catch {
          // Fetch failed — fall back to a fresh apply (which will retry).
          onApplySearch(trimmed);
        }
        return;
      }

      // Case 3: No in-flight fetch — trigger a fresh backend search via the store.
      onApplySearch(trimmed);
      onClose();
    },
    [query, results, resultsQuery, onApplySearch, onApplySearchWithResults, onClose],
  );

  const handleResultClick = useCallback(
    (email: Email) => {
      const appliedQuery = resultsQuery ?? query.trim();

      if (appliedQuery && results) {
        onApplySearchWithResults(appliedQuery, results.emails);
      } else if (appliedQuery) {
        onApplySearch(appliedQuery);
      }

      onSelectEmail(email);
      onClose();
    },
    [onApplySearch, onApplySearchWithResults, onClose, onSelectEmail, query, results, resultsQuery],
  );

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] bg-black/50">
      <div className="w-full max-w-2xl bg-white rounded-xl shadow-2xl overflow-hidden">
        {/* Search Input */}
        <form onSubmit={handleSubmit} className="relative">
          <div className="flex items-center px-4 border-b border-gray-200">
            <svg className="w-5 h-5 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
              />
            </svg>
            <input
              ref={inputRef}
              type="text"
              value={query}
              onChange={(e) => handleInputChange(e.target.value)}
              onKeyDown={handleInputKeyDown}
              // Semantic/RAG search is disabled server-side in `services::search`
              // (see the `let use_ai = false` block) — keep the placeholder
              // limited to operators that actually work today. If you re-enable
              // RAG, swap this back to the NL hint variant.
              placeholder={t('inbox:searchBox.fullPlaceholder', {
                operators: 'from:, to:, subject:, before:YYYY-MM-DD, after:YYYY-MM-DD',
              })}
              className="flex-1 px-4 py-4 text-lg outline-none"
            />
            {isSearching && <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-primary-600"></div>}
            <button type="button" onClick={onClose} className="ml-2 p-2 text-gray-400 hover:text-gray-600 rounded-lg">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        </form>

        {/* Autocomplete dropdown for from:/to: */}
        {autocomplete && autocomplete.suggestions.length > 0 && (
          <div className="border-b border-gray-200 bg-white max-h-64 overflow-y-auto">
            {autocomplete.suggestions.map((s, idx) => (
              <button
                key={s.email}
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault(); // keep input focused
                  applySuggestion(s);
                }}
                onMouseEnter={() => setAutocomplete((prev) => (prev ? { ...prev, selectedIndex: idx } : null))}
                className={`w-full text-left px-4 py-2 flex items-center gap-3 ${
                  idx === autocomplete.selectedIndex ? 'bg-primary-50' : 'hover:bg-gray-50'
                }`}
              >
                <svg
                  className="w-4 h-4 text-gray-400 flex-shrink-0"
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
                  {s.name && s.name !== s.email && <div className="text-sm text-gray-900 truncate">{s.name}</div>}
                  <div
                    className={`${s.name && s.name !== s.email ? 'text-xs text-gray-500' : 'text-sm text-gray-900'} truncate`}
                  >
                    {s.email}
                  </div>
                </div>
              </button>
            ))}
            <div className="px-4 py-1.5 text-[10px] text-gray-400 border-t border-gray-100 bg-gray-50">
              {t('inbox:searchBox.autocompleteHint')}
            </div>
          </div>
        )}

        {/* Search Tips */}
        {!query && (
          <div className="p-4 bg-gray-50 border-b border-gray-200">
            <h4 className="text-xs font-semibold text-gray-500 uppercase mb-2">{t('inbox:searchTips.title')}</h4>
            <div className="grid grid-cols-2 gap-2 text-sm text-gray-600">
              <div>
                <code className="text-primary-600">from:</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.senderHint')}
              </div>
              <div>
                <code className="text-primary-600">to:</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.recipientHint')}
              </div>
              <div>
                <code className="text-primary-600">subject:</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.subjectHint')}
              </div>
              <div>
                <code className="text-primary-600">is:unread</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.unreadHint')}
              </div>
              <div>
                <code className="text-primary-600">after:</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.afterHint')}
              </div>
              <div>
                <code className="text-primary-600">before:</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.beforeHint')}
              </div>
              <div>
                <code className="text-primary-600">tag:</code> {/* // i18n-ignore: literal query syntax */}{' '}
                {t('inbox:searchTips.tagHint')}
              </div>
              <div>
                {/* // i18n-ignore (next line): today / this week / this month are literal query values */}
                <code className="text-primary-600">today</code> {/* // i18n-ignore */} /{' '}
                <code className="text-primary-600">this week</code> {/* // i18n-ignore */} /{' '}
                <code className="text-primary-600">this month</code> {/* // i18n-ignore */}
              </div>
            </div>
            {/* Semantic / RAG search panel intentionally omitted: it is
                hard-disabled in `services::search`. Re-add a status pill here
                when you flip the backend flag (and gate it on
                `api.checkAiAvailable()` again). */}
          </div>
        )}

        {/* Parsed Query Info */}
        {results?.parsedQuery && <ParsedQueryDisplay query={results.parsedQuery} />}

        {/* Results */}
        <div className="max-h-96 overflow-y-auto">
          {searchError && (
            <div className="p-4 border-b border-red-100 bg-red-50 text-sm text-red-700">
              Search failed: {searchError}
            </div>
          )}
          {results && results.emails.length === 0 && (
            <div className="p-8 text-center text-gray-500">No emails found for "{query}"</div>
          )}
          {results && results.emails.length > 0 && (
            <ul>
              {results.emails.map((emailWithScore) => (
                <li key={emailWithScore.id}>
                  <button
                    onClick={() => handleResultClick(emailWithScore)}
                    className="w-full text-left px-4 py-3 hover:bg-gray-50 transition-colors border-b border-gray-100"
                  >
                    <div className="flex items-center justify-between">
                      <span className={`font-medium ${emailWithScore.isRead ? 'text-gray-700' : 'text-gray-900'}`}>
                        {emailWithScore.sender}
                      </span>
                      <div className="flex items-center gap-2">
                        {emailWithScore.relevanceScore !== null && (
                          <RelevanceIndicator score={emailWithScore.relevanceScore} />
                        )}
                        <span className="text-xs text-gray-400">{formatDate(emailWithScore.timestamp, locale)}</span>
                      </div>
                    </div>
                    <div className={`text-sm ${emailWithScore.isRead ? 'text-gray-500' : 'text-gray-700'}`}>
                      {emailWithScore.subject || '(No subject)'}
                    </div>
                    <div className="text-sm text-gray-400 truncate">{emailWithScore.snippet}</div>
                    {emailWithScore.matchReason && (
                      <div className="mt-1 text-xs text-primary-600 flex items-center gap-1">
                        <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                          />
                        </svg>
                        {emailWithScore.matchReason}
                      </div>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* Footer */}
        {results && (
          <div className="px-4 py-2 bg-gray-50 border-t border-gray-200 flex items-center justify-between text-xs text-gray-500">
            <span>
              {results.emails.length > 0
                ? `Found ${results.emails.length} result${results.emails.length !== 1 ? 's' : ''}`
                : 'No results'}
            </span>
            <SearchMethodBadge method={results.searchMethod} />
          </div>
        )}
      </div>
    </div>
  );
}

function ParsedQueryDisplay({ query }: { query: ParsedSearchQuery }) {
  const { i18n } = useTranslation();
  const locale = i18n.language || 'en';
  const filters = [];
  if (query.fromFilter) filters.push(`from: ${query.fromFilter}`);
  if (query.toFilter) filters.push(`to: ${query.toFilter}`);
  if (query.subjectFilter) filters.push(`subject: ${query.subjectFilter}`);
  if (query.isUnread) filters.push('unread only');
  const numericDate: Intl.DateTimeFormatOptions = { year: 'numeric', month: 'numeric', day: 'numeric' };
  if (query.afterTimestamp) filters.push(`after: ${formatDateIntl(query.afterTimestamp, locale, numericDate)}`);
  if (query.beforeTimestamp) filters.push(`before: ${formatDateIntl(query.beforeTimestamp, locale, numericDate)}`);
  if (query.keywords.length > 0) filters.push(`keywords: ${query.keywords.join(', ')}`);

  if (filters.length === 0) return null;

  return (
    <div className="px-4 py-2 bg-blue-50 border-b border-blue-100">
      <div className="flex items-center gap-2 text-xs text-blue-700">
        <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 20 20">
          <path d="M9 4.804A7.968 7.968 0 005.5 4c-1.255 0-2.443.29-3.5.804v10A7.969 7.969 0 015.5 14c1.669 0 3.218.51 4.5 1.385A7.962 7.962 0 0114.5 14c1.255 0 2.443.29 3.5.804v-10A7.968 7.968 0 0014.5 4c-1.255 0-2.443.29-3.5.804V12a1 1 0 11-2 0V4.804z" />
        </svg>
        <span>Searching: {filters.join(' • ')}</span>
      </div>
    </div>
  );
}

function SearchMethodBadge({ method }: { method: SearchMethod }) {
  const config: Record<SearchMethod, { label: string; icon: ReactElement; className: string }> = {
    rag: {
      label: 'Semantic Search (RAG)',
      icon: (
        <svg className="w-3 h-3" viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z" />
        </svg>
      ),
      className: 'bg-green-100 text-green-700',
    },
    ai_parsed: {
      label: 'AI Parsed',
      icon: (
        <svg className="w-3 h-3" viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z" />
        </svg>
      ),
      className: 'bg-purple-100 text-purple-700',
    },
    pattern_parsed: {
      label: 'Filter Search',
      icon: (
        <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z"
          />
        </svg>
      ),
      className: 'bg-blue-100 text-blue-700',
    },
    keyword_search: {
      label: 'Keyword Search',
      icon: (
        <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
          />
        </svg>
      ),
      className: 'bg-gray-100 text-gray-600',
    },
  };

  const { label, icon, className } = config[method] ?? config.keyword_search;

  return (
    <span className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium ${className}`}>
      {icon}
      {label}
    </span>
  );
}

function RelevanceIndicator({ score }: { score: number }) {
  // Convert 0-1 score to percentage
  const percentage = Math.round(score * 100);

  // Color based on score
  let colorClass = 'bg-gray-200 text-gray-600';
  if (score >= 0.8) {
    colorClass = 'bg-green-100 text-green-700';
  } else if (score >= 0.65) {
    colorClass = 'bg-blue-100 text-blue-700';
  } else if (score >= 0.55) {
    colorClass = 'bg-yellow-100 text-yellow-700';
  }

  return (
    <span
      className={`inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium ${colorClass}`}
      title={`${percentage}% semantic similarity`}
    >
      {percentage}%
    </span>
  );
}

function formatDate(timestamp: number, locale: string): string {
  const date = new Date(timestamp * 1000);
  const now = new Date();
  const diff = now.getTime() - date.getTime();

  if (diff < 24 * 60 * 60 * 1000) {
    return formatTimeIntl(timestamp, locale, { hour: '2-digit', minute: '2-digit' });
  } else if (diff < 7 * 24 * 60 * 60 * 1000) {
    return formatDateIntl(timestamp, locale, { weekday: 'short' });
  } else {
    return formatDateIntl(timestamp, locale, { month: 'short', day: 'numeric' });
  }
}
