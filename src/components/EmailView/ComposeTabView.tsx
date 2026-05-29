import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RichTextEditor } from '@/components/shared/RichTextEditor';
import type { EmailAttachment, RecipientSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import { prepareOutgoingHtml } from '@/lib/composeHtml';
import { errorText } from '@/lib/errors';
import type { ComposeTab } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';

interface ComposeTabViewProps {
  tab: ComposeTab;
  accounts: Account[];
  onClose: () => void;
}

function getDomain(email: string): string {
  return extractEmail(email).split('@')[1]?.toLowerCase() || '';
}

function extractEmail(raw: string): string {
  const match = raw.match(/<([^>]+)>/);
  return (match ? match[1] : raw).trim().toLowerCase();
}

export function ComposeTabView({ tab, accounts, onClose }: ComposeTabViewProps) {
  const { t } = useTranslation(['compose']);
  const addLog = useLogStore((s) => s.addLog);

  const selfEmails = accounts.map((a) => a.email.toLowerCase());
  const [fromAccountId, setFromAccountId] = useState(tab.accountId);
  const [subject, setSubject] = useState(tab.subject);
  const [bodyHtml, setBodyHtml] = useState(tab.bodyHtml);
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  const [toRecipients, setToRecipients] = useState<string[]>(tab.toAddresses);
  const [ccRecipients, setCcRecipients] = useState<string[]>([]);
  const [showCc, setShowCc] = useState(false);
  const [attachments, setAttachments] = useState<EmailAttachment[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [toInput, setToInput] = useState('');
  const [ccInput, setCcInput] = useState('');
  const [suggestions, setSuggestions] = useState<RecipientSuggestion[]>([]);
  const [activeField, setActiveField] = useState<'to' | 'cc' | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const suggestionsRef = useRef(0);

  const allRecipients = [...toRecipients, ...ccRecipients];
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
    const trimmed = extractEmail(email);
    if (!trimmed || !trimmed.includes('@')) return;
    if (field === 'to') {
      if (!toRecipients.includes(trimmed)) setToRecipients((prev) => [...prev, trimmed]);
      setToInput('');
    } else {
      if (!ccRecipients.includes(trimmed)) setCcRecipients((prev) => [...prev, trimmed]);
      setCcInput('');
    }
    setSuggestions([]);
  };

  const removeRecipient = (field: 'to' | 'cc', email: string) => {
    if (field === 'to') setToRecipients((prev) => prev.filter((r) => r !== email));
    else setCcRecipients((prev) => prev.filter((r) => r !== email));
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
    if (toRecipients.length === 0 || !subject.trim() || !plain) return;
    setSendError(null);
    setIsSending(true);
    try {
      await api.sendNewEmail(
        fromAccountId,
        toRecipients,
        ccRecipients,
        subject.trim(),
        plain,
        attachments,
        prepared.bodyHtml,
        prepared.inlineImages,
      );
      addLog('success', 'sync', `Email sent to ${toRecipients.join(', ')}`);
      setSent(true);
      setTimeout(onClose, 1200);
    } catch (err) {
      setSendError(errorText(err));
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
        onClick={() => document.getElementById(`compose-tab-${tab.id}-${field}-input`)?.focus()}
      >
        {recipients.map((r) => (
          <span
            key={r}
            className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-700"
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
              ×
            </button>
          </span>
        ))}
        <div className="relative flex-1 min-w-[120px]">
          <input
            id={`compose-tab-${tab.id}-${field}-input`}
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
                  className={`w-full text-left px-3 py-2 text-sm hover:bg-gray-50 flex items-center gap-2 ${i === selectedIdx ? 'bg-primary-50' : ''}`}
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
    <div className="flex-1 bg-white flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-5 py-3 border-b border-gray-200 flex-shrink-0">
        <h2 className="text-base font-semibold text-gray-900">{t('compose:newEmail')}</h2>
        <button
          type="button"
          onClick={onClose}
          className="p-1 text-gray-400 hover:text-gray-600 rounded hover:bg-gray-100 transition-colors"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-5">
        {/* From */}
        <div className="flex items-center gap-2 mb-2">
          <span className="text-sm text-gray-500 w-14 flex-shrink-0">{t('compose:from')}</span>
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

        {renderTokenInput('to', 'To', toRecipients, toInput, setToInput)}

        {!showCc && (
          <button
            type="button"
            onClick={() => setShowCc(true)}
            className="text-xs text-primary-500 hover:text-primary-600 mb-2 ml-16"
          >
            + Cc
          </button>
        )}
        {showCc && renderTokenInput('cc', 'Cc', ccRecipients, ccInput, setCcInput)}

        <div className="flex items-center gap-2 mb-3">
          <span className="text-sm text-gray-500 w-14 flex-shrink-0">{t('compose:subject')}</span>
          <input
            type="text"
            value={subject}
            onChange={(e) => setSubject(e.target.value)}
            className="flex-1 text-sm border border-gray-300 rounded-lg px-3 py-1.5 bg-white focus:border-primary-500 focus:ring-2 focus:ring-primary-100 outline-none"
            placeholder={t('compose:subjectPlaceholderLong')}
          />
        </div>

        <RichTextEditor
          value={bodyHtml}
          onChange={setBodyHtml}
          placeholder={t('compose:bodyPlaceholder')}
          contentClassName="min-h-[300px]"
        />

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

        {sendError && (
          <div className="mt-3 px-3 py-2 bg-red-50 border border-red-200 rounded-lg text-xs text-red-700">
            {sendError}
          </div>
        )}
        {sent && (
          <div className="mt-3 px-3 py-2 bg-green-50 border border-green-200 rounded-lg text-xs text-green-700">
            {t('compose:sentConfirm')}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="px-5 py-3 border-t border-gray-200 flex items-center gap-2 flex-shrink-0">
        <button
          type="button"
          onClick={() => fileInputRef.current?.click()}
          disabled={isSending || sent}
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
          onClick={onClose}
          disabled={isSending || sent}
          className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-lg hover:bg-gray-100 disabled:opacity-50"
        >
          Discard
        </button>
        <button
          type="button"
          onClick={handleSend}
          disabled={isSending || sent || toRecipients.length === 0 || !subject.trim() || !bodyHtml.trim()}
          className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-lg hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {isSending ? 'Sending…' : 'Send'}
        </button>
      </div>
    </div>
  );
}
