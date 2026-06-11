import { format } from 'date-fns';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RichTextEditor } from '@/components/shared/RichTextEditor';
import type { DraftSource, EmailAttachment, RecipientSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import { plainTextToHtml, prepareOutgoingHtml } from '@/lib/composeHtml';
import type { Account, Email } from '@/types';

interface ReplyComposeProps {
  email: Email;
  threadEmails: Email[];
  accounts: Account[];
  defaultAccountId: string;
  onSend: (params: {
    fromAccountId: string;
    toEmails: string[];
    ccEmails: string[];
    body: string;
    bodyHtml?: string;
    inlineImages?: EmailAttachment[];
    attachments?: EmailAttachment[];
  }) => Promise<void>;
  onCancel: () => void;
  initialBody: string;
  mode: 'reply' | 'reply-all';
  /** True while an AI draft is being generated for this reply. Disables the
   *  textarea and shows a spinner inline so the user knows the draft is
   *  being produced on the AI queue. */
  isLoadingDraft?: boolean;
  /** Past threads the AI used as precedent for the current draft. Rendered as
   *  collapsed cards at the bottom of the reply panel for transparency. */
  draftSources?: DraftSource[];
}

function getDomain(email: string): string {
  return extractEmail(email).split('@')[1]?.toLowerCase() || '';
}

/** Extract bare email from "Name <email>" or plain email string */
function extractEmail(raw: string): string {
  const match = raw.match(/<([^>]+)>/);
  return (match ? match[1] : raw).trim().toLowerCase();
}

function detectUnusualRecipients(recipients: string[], selfEmails: string[]): string[] {
  const nonSelf = recipients.filter((r) => !selfEmails.includes(r.toLowerCase()));
  if (nonSelf.length < 2) return [];

  const domains = nonSelf.map(getDomain);
  const domainCounts: Record<string, number> = {};
  for (const d of domains) {
    domainCounts[d] = (domainCounts[d] || 0) + 1;
  }

  const maxCount = Math.max(...Object.values(domainCounts));
  const majorityDomains = new Set(
    Object.entries(domainCounts)
      .filter(([, c]) => c === maxCount)
      .map(([d]) => d),
  );

  return nonSelf.filter((r) => !majorityDomains.has(getDomain(r)));
}

export function ReplyCompose({
  email,
  threadEmails,
  accounts,
  defaultAccountId,
  onSend,
  onCancel,
  initialBody,
  mode,
  isLoadingDraft = false,
  draftSources = [],
}: ReplyComposeProps) {
  const { t } = useTranslation(['compose']);
  const selfEmails = accounts.map((a) => a.email.toLowerCase());
  const [fromAccountId, setFromAccountId] = useState(defaultAccountId);
  // Editor holds HTML. AI drafts and the initial empty state are plain text;
  // we wrap them in <p>...</p> so Tiptap renders them as one paragraph each.
  const [bodyHtml, setBodyHtml] = useState<string>(() => plainTextToHtml(initialBody));
  const [isSending, setIsSending] = useState(false);

  // Sync body when the parent updates initialBody (e.g. AI draft generation
  // replaces the "Generating draft..." placeholder with the actual draft).
  useEffect(() => {
    setBodyHtml(plainTextToHtml(initialBody));
  }, [initialBody]);

  // Compute initial recipients
  const initialTo = (() => {
    if (mode === 'reply') {
      return [email.senderEmail.toLowerCase()];
    }
    // Reply All: collect senders + recipients from ALL thread messages, minus self.
    // Using only the latest email misses participants from earlier in the thread.
    const all = new Set<string>();
    for (const msg of threadEmails) {
      all.add(extractEmail(msg.senderEmail));
      for (const r of [...msg.recipients, ...msg.cc]) {
        const clean = extractEmail(r);
        if (clean.includes('@')) all.add(clean);
      }
    }
    // Also include latest email in case threadEmails is empty
    all.add(extractEmail(email.senderEmail));
    for (const r of [...email.recipients, ...email.cc]) {
      const clean = extractEmail(r);
      if (clean.includes('@')) all.add(clean);
    }
    // Remove self
    for (const self of selfEmails) {
      all.delete(self);
    }
    return [...all];
  })();

  const [toRecipients, setToRecipients] = useState<string[]>(initialTo);
  const [ccRecipients, setCcRecipients] = useState<string[]>([]);
  const [showCc, setShowCc] = useState(false);
  const [attachments, setAttachments] = useState<EmailAttachment[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Autocomplete state
  const [toInput, setToInput] = useState('');
  const [ccInput, setCcInput] = useState('');
  const [suggestions, setSuggestions] = useState<RecipientSuggestion[]>([]);
  const [activeField, setActiveField] = useState<'to' | 'cc' | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const suggestionsRef = useRef(0);

  // Unusual recipient detection
  const allRecipients = [...toRecipients, ...ccRecipients];
  const unusualRecipients = detectUnusualRecipients(allRecipients, selfEmails);

  // Context domain for autocomplete boosting
  const contextDomain = (() => {
    const domains = allRecipients.filter((r) => !selfEmails.includes(r.toLowerCase())).map(getDomain);
    if (domains.length === 0) return undefined;
    const counts: Record<string, number> = {};
    for (const d of domains) counts[d] = (counts[d] || 0) + 1;
    return Object.entries(counts).sort((a, b) => b[1] - a[1])[0]?.[0];
  })();

  const fetchSuggestions = useCallback(
    async (prefix: string) => {
      if (prefix.length < 2) {
        setSuggestions([]);
        return;
      }
      const reqId = ++suggestionsRef.current;
      const account = accounts.find((a) => a.id === fromAccountId);
      if (!account) return;
      try {
        const results = await api.autocompleteRecipients(account.id, prefix, contextDomain, 8);
        if (suggestionsRef.current !== reqId) return;
        // Filter out already-added recipients
        const existing = new Set([...toRecipients, ...ccRecipients].map((r) => r.toLowerCase()));
        setSuggestions(results.filter((r) => !existing.has(r.email.toLowerCase())));
        setSelectedIdx(0);
      } catch {
        setSuggestions([]);
      }
    },
    [fromAccountId, accounts, toRecipients, ccRecipients, contextDomain],
  );

  const addRecipient = (field: 'to' | 'cc', email: string) => {
    const trimmed = email.trim().toLowerCase();
    if (!trimmed.includes('@')) return;
    if (field === 'to') {
      if (!toRecipients.includes(trimmed)) setToRecipients([...toRecipients, trimmed]);
      setToInput('');
    } else {
      if (!ccRecipients.includes(trimmed)) setCcRecipients([...ccRecipients, trimmed]);
      setCcInput('');
    }
    setSuggestions([]);
  };

  const removeRecipient = (field: 'to' | 'cc', email: string) => {
    if (field === 'to') setToRecipients(toRecipients.filter((r) => r !== email));
    else setCcRecipients(ccRecipients.filter((r) => r !== email));
  };

  const handleFiles = useCallback((files: FileList | null) => {
    if (!files) return;
    Array.from(files).forEach((file) => {
      const reader = new FileReader();
      reader.onload = (e) => {
        const result = e.target?.result as string;
        const data = result.split(',')[1] ?? '';
        setAttachments((prev) => [
          ...prev,
          { filename: file.name, mimeType: file.type || 'application/octet-stream', data },
        ]);
      };
      reader.readAsDataURL(file);
    });
    if (fileInputRef.current) fileInputRef.current.value = '';
  }, []);

  const handleKeyDown = (field: 'to' | 'cc', e: React.KeyboardEvent<HTMLInputElement>, input: string) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIdx((i) => Math.min(i + 1, suggestions.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIdx((i) => Math.max(i - 1, 0));
    } else if ((e.key === 'Enter' || e.key === 'Tab') && suggestions.length > 0) {
      e.preventDefault();
      addRecipient(field, suggestions[selectedIdx].email);
    } else if (e.key === 'Enter' && input.includes('@')) {
      e.preventDefault();
      addRecipient(field, input);
    } else if (e.key === 'Backspace' && input === '') {
      e.preventDefault();
      const list = field === 'to' ? toRecipients : ccRecipients;
      if (list.length > 0) removeRecipient(field, list[list.length - 1]);
    }
  };

  const handleSend = async () => {
    const prepared = prepareOutgoingHtml(bodyHtml);
    const plain = prepared.plainText.trim();
    if (toRecipients.length === 0 || !plain) return;
    setIsSending(true);
    try {
      await onSend({
        fromAccountId,
        toEmails: toRecipients,
        ccEmails: ccRecipients,
        body: plain,
        bodyHtml: prepared.bodyHtml,
        inlineImages: prepared.inlineImages,
        attachments,
      });
    } finally {
      setIsSending(false);
    }
  };

  const renderTokenInput = (
    field: 'to' | 'cc',
    label: string,
    recipients: string[],
    input: string,
    setInput: (v: string) => void,
  ) => (
    <div className="flex items-start gap-2 mb-2">
      <span className="text-sm text-gray-500 mt-1.5 w-8 flex-shrink-0">{label}</span>
      <div
        className="flex-1 flex flex-wrap gap-1 items-center border border-gray-300 rounded-lg px-2 py-1.5 bg-white min-h-[36px] focus-within:border-primary-500 focus-within:ring-2 focus-within:ring-primary-100"
        onClick={() => document.getElementById(`${field}-input`)?.focus()}
      >
        {recipients.map((r) => {
          const isUnusual = unusualRecipients.includes(r);
          return (
            <span
              key={r}
              className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium ${
                isUnusual ? 'bg-amber-100 text-amber-800 border border-amber-300' : 'bg-gray-100 text-gray-700'
              }`}
              title={isUnusual ? 'Different domain than other recipients' : r}
            >
              {r}
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  removeRecipient(field, r);
                }}
                className="hover:text-red-500"
              >
                x
              </button>
            </span>
          );
        })}
        <div className="relative flex-1 min-w-[120px]">
          <input
            id={`${field}-input`}
            type="text"
            value={input}
            onChange={(e) => {
              setInput(e.target.value);
              setActiveField(field);
              void fetchSuggestions(e.target.value);
            }}
            onFocus={() => setActiveField(field)}
            onBlur={() => setTimeout(() => setActiveField(null), 200)}
            onKeyDown={(e) => handleKeyDown(field, e, input)}
            className="w-full text-sm outline-none bg-transparent py-0.5"
            placeholder={recipients.length === 0 ? 'Add recipients...' : ''}
          />
          {activeField === field && suggestions.length > 0 && (
            <div className="absolute top-full left-0 mt-1 w-72 bg-white border border-gray-200 rounded-lg shadow-lg z-50 max-h-48 overflow-y-auto">
              {suggestions.map((s, i) => (
                <button
                  key={s.email}
                  type="button"
                  className={`w-full text-left px-3 py-2 text-sm hover:bg-gray-50 flex items-center gap-2 ${
                    i === selectedIdx ? 'bg-primary-50' : ''
                  }`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    addRecipient(field, s.email);
                  }}
                >
                  <div className="flex-1 min-w-0">
                    <div className="truncate text-gray-900">{s.email}</div>
                    {s.name && <div className="truncate text-xs text-gray-500">{s.name}</div>}
                  </div>
                  {s.domainMatch && (
                    <span className="text-[10px] px-1.5 py-0.5 bg-green-100 text-green-700 rounded flex-shrink-0">
                      {t('compose:sameDomain')}
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );

  return (
    <div className="mt-4 rounded-lg border border-gray-200 bg-gray-50 p-4">
      {/* From selector */}
      <div className="flex items-center gap-2 mb-2">
        <span className="text-sm text-gray-500 w-8 flex-shrink-0">{t('compose:from')}</span>
        <select
          value={fromAccountId}
          onChange={(e) => setFromAccountId(e.target.value)}
          className="flex-1 text-sm border border-gray-300 rounded-lg px-3 py-1.5 bg-white focus:border-primary-500 outline-none"
        >
          {accounts
            .filter((a) => a.enabled)
            .map((a) => (
              <option key={a.id} value={a.id}>
                {a.email}
              </option>
            ))}
        </select>
      </div>

      {/* To field */}
      {renderTokenInput('to', 'To', toRecipients, toInput, setToInput)}

      {/* Cc toggle + field */}
      {!showCc && (
        <button
          type="button"
          onClick={() => setShowCc(true)}
          className="text-xs text-primary-500 hover:text-primary-600 mb-2 ml-10"
        >
          + Cc
        </button>
      )}
      {showCc && renderTokenInput('cc', 'Cc', ccRecipients, ccInput, setCcInput)}

      {/* Unusual recipient warning */}
      {unusualRecipients.length > 0 && (
        <div className="mb-3 ml-10 p-2 bg-amber-50 border border-amber-200 rounded-lg flex items-start gap-2">
          <svg className="w-4 h-4 text-amber-500 flex-shrink-0 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
            <path
              fillRule="evenodd"
              d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z"
              clipRule="evenodd"
            />
          </svg>
          <div className="text-xs text-amber-700">
            <strong>{t('compose:unusualRecipients')}</strong> {unusualRecipients.join(', ')} — different domain than the
            other recipients. Double-check before sending.
          </div>
        </div>
      )}

      {/* Body */}
      <div className="relative">
        <RichTextEditor
          value={bodyHtml}
          onChange={setBodyHtml}
          disabled={isLoadingDraft}
          placeholder={isLoadingDraft ? 'Generating draft…' : 'Write your reply...'}
          contentClassName="min-h-[180px]"
        />
        {isLoadingDraft && (
          <div className="pointer-events-none absolute inset-0 flex items-start justify-center pt-4">
            <div className="flex items-center gap-2 rounded-full bg-white/90 border border-gray-200 px-3 py-1.5 text-xs text-gray-600 shadow-sm">
              <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-primary-600" />
              <span>{t('compose:aiDraft.generatingLong')}</span>
            </div>
          </div>
        )}
      </div>

      {/* Attachments */}
      {attachments.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {attachments.map((att, i) => (
            <div
              key={att.filename}
              className="flex items-center gap-1.5 px-2.5 py-1 bg-gray-100 border border-gray-200 rounded-lg text-xs text-gray-700"
            >
              <span className="max-w-[180px] truncate">{att.filename}</span>
              <button
                type="button"
                onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))}
                className="ml-0.5 text-gray-400 hover:text-red-500"
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Actions */}
      <div className="mt-3 flex items-center gap-2">
        <button
          type="button"
          onClick={() => fileInputRef.current?.click()}
          disabled={isSending}
          className="p-2 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded-lg transition-colors disabled:opacity-50"
          title={t('compose:attachFiles')}
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
            />
          </svg>
        </button>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => handleFiles(e.target.files)}
        />
        <div className="flex-1" />
        <button
          type="button"
          onClick={onCancel}
          disabled={isSending}
          className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-lg hover:bg-gray-100 disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={handleSend}
          disabled={isSending || isLoadingDraft || toRecipients.length === 0 || !bodyHtml.trim()}
          className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-lg hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {isSending ? 'Sending...' : mode === 'reply-all' ? 'Reply All' : 'Send Reply'}
        </button>
      </div>

      {/* RAG sources — past threads the AI used as precedent. Collapsed by
          default so they don't push the action buttons off-screen; the user
          opens individual cards to audit retrieval quality. */}
      {draftSources.length > 0 && <DraftSourcesPanel sources={draftSources} />}
    </div>
  );
}

function DraftSourcesPanel({ sources }: { sources: DraftSource[] }) {
  return (
    <div className="mt-4 pt-3 border-t border-gray-200">
      <div className="text-xs font-medium text-gray-500 mb-2">
        Similar past threads used for context ({sources.length})
      </div>
      <div className="flex flex-col gap-1.5">
        {sources.map((src) => (
          <DraftSourceCard key={src.emailId} source={src} />
        ))}
      </div>
    </div>
  );
}

function DraftSourceCard({ source }: { source: DraftSource }) {
  const { t } = useTranslation(['compose']);
  const [open, setOpen] = useState(false);
  const dateStr = format(new Date(source.timestamp * 1000), 'PP');
  return (
    <div className="rounded border border-gray-200 bg-white">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center gap-2 px-3 py-2 text-left text-xs hover:bg-gray-50"
      >
        <svg
          className={`w-3 h-3 text-gray-400 flex-shrink-0 transition-transform ${open ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        <span className="flex-1 min-w-0 truncate font-medium text-gray-800">{source.subject || '(No subject)'}</span>
        {source.sentByUser && (
          <span className="flex-shrink-0 px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-primary-50 text-primary-700 border border-primary-100">
            {t('compose:yourReply')}
          </span>
        )}
        <span className="flex-shrink-0 text-gray-400">{dateStr}</span>
      </button>
      {open && (
        <div className="px-3 pb-3 pt-1 text-xs text-gray-600 border-t border-gray-100">
          <div className="text-gray-500 mb-1">
            {source.sender} &lt;{source.senderEmail}&gt;
          </div>
          <div className="whitespace-pre-wrap break-words text-gray-700">{source.snippet}</div>
        </div>
      )}
    </div>
  );
}
