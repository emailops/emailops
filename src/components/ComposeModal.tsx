import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { RichTextEditor } from '@/components/shared/RichTextEditor';
import type { EmailAttachment, RecipientSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import { prepareOutgoingHtml } from '@/lib/composeHtml';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';

export interface ComposeMaximizeState {
  accountId: string;
  toAddresses: string[];
  subject: string;
  /** Rich-text HTML body so the maximized tab can keep formatting + inline images. */
  bodyHtml: string;
}

interface ComposeModalProps {
  accounts: Account[];
  defaultAccountId: string;
  /** Pre-fill the To field. Used by the Contacts view's "Compose" action. */
  defaultToRecipients?: string[];
  onClose: () => void;
  onMaximize?: (state: ComposeMaximizeState) => void;
}

interface LoadingFile {
  name: string;
  progress: number; // 0–100
}

function getDomain(email: string): string {
  return extractEmail(email).split('@')[1]?.toLowerCase() || '';
}

function extractEmail(raw: string): string {
  const match = raw.match(/<([^>]+)>/);
  return (match ? match[1] : raw).trim().toLowerCase();
}

function detectUnusualRecipients(recipients: string[], selfEmails: string[]): string[] {
  const nonSelf = recipients.filter((r) => !selfEmails.includes(r.toLowerCase()));
  if (nonSelf.length < 2) return [];
  const domains = nonSelf.map(getDomain);
  const domainCounts: Record<string, number> = {};
  for (const d of domains) domainCounts[d] = (domainCounts[d] || 0) + 1;
  const maxCount = Math.max(...Object.values(domainCounts));
  const majorityDomains = new Set(
    Object.entries(domainCounts)
      .filter(([, c]) => c === maxCount)
      .map(([d]) => d),
  );
  return nonSelf.filter((r) => !majorityDomains.has(getDomain(r)));
}

export function ComposeModal({
  accounts,
  defaultAccountId,
  defaultToRecipients,
  onClose,
  onMaximize,
}: ComposeModalProps) {
  const { t } = useTranslation(['compose', 'common']);
  const addLog = useLogStore((s) => s.addLog);
  const selfEmails = accounts.map((a) => a.email.toLowerCase());
  const [fromAccountId, setFromAccountId] = useState(defaultAccountId);
  const [subject, setSubject] = useState('');
  // Rich-text HTML body. Empty string → editor shows empty state.
  const [bodyHtml, setBodyHtml] = useState('');
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  const [toRecipients, setToRecipients] = useState<string[]>(() =>
    (defaultToRecipients ?? []).map((r) => r.trim().toLowerCase()).filter(Boolean),
  );
  const [ccRecipients, setCcRecipients] = useState<string[]>([]);
  const [showCc, setShowCc] = useState(false);
  const [attachments, setAttachments] = useState<EmailAttachment[]>([]);
  const [loadingFiles, setLoadingFiles] = useState<LoadingFile[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [toInput, setToInput] = useState('');
  const [ccInput, setCcInput] = useState('');
  const [suggestions, setSuggestions] = useState<RecipientSuggestion[]>([]);
  const [activeField, setActiveField] = useState<'to' | 'cc' | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const suggestionsRef = useRef(0);

  const allRecipients = [...toRecipients, ...ccRecipients];
  const unusualRecipients = detectUnusualRecipients(allRecipients, selfEmails);
  const isLoadingAttachments = loadingFiles.length > 0;

  // Overall attachment load progress (average across all loading files)
  const loadProgress =
    loadingFiles.length === 0
      ? 100
      : Math.round(loadingFiles.reduce((sum, f) => sum + f.progress, 0) / loadingFiles.length);

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
      const name = file.name;
      setLoadingFiles((prev) => [...prev, { name, progress: 0 }]);

      const reader = new FileReader();

      reader.onprogress = (e) => {
        if (e.lengthComputable) {
          const pct = Math.round((e.loaded / e.total) * 100);
          setLoadingFiles((prev) => prev.map((f) => (f.name === name ? { ...f, progress: pct } : f)));
        }
      };

      reader.onload = (e) => {
        const result = e.target?.result as string;
        const data = result.split(',')[1] ?? '';
        setAttachments((prev) => [
          ...prev,
          { filename: name, mimeType: file.type || 'application/octet-stream', data },
        ]);
        setLoadingFiles((prev) => prev.filter((f) => f.name !== name));
      };

      reader.onerror = () => {
        setLoadingFiles((prev) => prev.filter((f) => f.name !== name));
      };

      reader.readAsDataURL(file);
    });
    // Reset the input so the same file can be re-selected if removed
    if (fileInputRef.current) fileInputRef.current.value = '';
  }, []);

  const removeAttachment = (index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  };

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

  // Close on Escape (but not while sending)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isSending) onClose();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose, isSending]);

  const handleSend = async () => {
    const prepared = prepareOutgoingHtml(bodyHtml);
    const plain = prepared.plainText.trim();
    if (toRecipients.length === 0 || !subject.trim() || !plain || isLoadingAttachments) return;
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
        onClick={() => document.getElementById(`compose-${field}-input`)?.focus()}
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
                ×
              </button>
            </span>
          );
        })}
        <div className="relative flex-1 min-w-[120px]">
          <input
            id={`compose-${field}-input`}
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

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/40" onClick={!isSending ? onClose : undefined} />
      <div className="relative bg-white rounded-xl shadow-2xl w-full max-w-2xl mx-4 flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-gray-200">
          <h2 className="text-base font-semibold text-gray-900">{t('compose:newEmail')}</h2>
          <div className="flex items-center gap-1">
            {onMaximize && (
              <button
                type="button"
                onClick={() => onMaximize({ accountId: fromAccountId, toAddresses: toRecipients, subject, bodyHtml })}
                disabled={isSending}
                className="p-1 text-gray-400 hover:text-gray-600 rounded hover:bg-gray-100 transition-colors disabled:opacity-40"
                title={t('compose:openInTab')}
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M4 8V4m0 0h4M4 4l5 5m11-1V4m0 0h-4m4 0l-5 5M4 16v4m0 0h4m-4 0l5-5m11 5l-5-5m5 5v-4m0 4h-4"
                  />
                </svg>
              </button>
            )}
            <button
              type="button"
              onClick={onClose}
              disabled={isSending}
              className="p-1 text-gray-400 hover:text-gray-600 rounded hover:bg-gray-100 transition-colors disabled:opacity-40"
            >
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        </div>

        {/* Body */}
        <div className="p-5 flex-1 overflow-y-auto">
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

          {/* To */}
          {renderTokenInput('to', 'To', toRecipients, toInput, setToInput)}

          {/* Cc toggle */}
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

          {/* Unusual recipient warning */}
          {unusualRecipients.length > 0 && (
            <div className="mb-3 ml-16 p-2 bg-amber-50 border border-amber-200 rounded-lg flex items-start gap-2">
              <svg className="w-4 h-4 text-amber-500 flex-shrink-0 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z"
                  clipRule="evenodd"
                />
              </svg>
              <div className="text-xs text-amber-700">
                <strong>{t('compose:unusualRecipients')}</strong> {unusualRecipients.join(', ')} — different domain than
                the other recipients. Double-check before sending.
              </div>
            </div>
          )}

          {/* Subject */}
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

          {/* Body */}
          <RichTextEditor
            value={bodyHtml}
            onChange={setBodyHtml}
            placeholder={t('compose:bodyPlaceholder')}
            contentClassName="min-h-[240px]"
          />

          {/* Attachment loading progress */}
          {isLoadingAttachments && (
            <div className="mt-3">
              <div className="flex items-center justify-between mb-1">
                <span className="text-xs text-gray-500">
                  Reading {loadingFiles.length} file{loadingFiles.length > 1 ? 's' : ''}…
                </span>
                <span className="text-xs text-gray-400">{loadProgress}%</span>
              </div>
              <div className="w-full h-1.5 bg-gray-200 rounded-full overflow-hidden">
                <div
                  className="h-full bg-primary-500 rounded-full transition-all duration-100"
                  style={{ width: `${loadProgress}%` }}
                />
              </div>
              <div className="mt-1.5 flex flex-wrap gap-1">
                {loadingFiles.map((f) => (
                  <span key={f.name} className="text-xs text-gray-400 truncate max-w-[200px]">
                    {f.name}
                  </span>
                ))}
              </div>
            </div>
          )}

          {/* Attached files */}
          {attachments.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-2">
              {attachments.map((att, i) => (
                <div
                  key={att.filename}
                  className="flex items-center gap-1.5 px-2.5 py-1 bg-gray-100 border border-gray-200 rounded-lg text-xs text-gray-700"
                >
                  <svg
                    className="w-3.5 h-3.5 text-gray-400 flex-shrink-0"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
                    />
                  </svg>
                  <span className="max-w-[180px] truncate">{att.filename}</span>
                  <button
                    type="button"
                    onClick={() => removeAttachment(i)}
                    className="ml-0.5 text-gray-400 hover:text-red-500"
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
          )}

          {/* Hidden file input */}
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="hidden"
            onChange={(e) => handleFiles(e.target.files)}
          />
        </div>

        {/* Send error */}
        {sendError && (
          <div className="mx-5 mb-0 mt-0 px-3 py-2 bg-red-50 border border-red-200 rounded-lg flex items-start gap-2 text-xs text-red-700">
            <svg className="w-4 h-4 flex-shrink-0 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
              <path
                fillRule="evenodd"
                d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z"
                clipRule="evenodd"
              />
            </svg>
            <span>{sendError}</span>
          </div>
        )}

        {/* Sent confirmation */}
        {sent && (
          <div className="mx-5 mb-0 px-3 py-2 bg-green-50 border border-green-200 rounded-lg flex items-center gap-2 text-xs text-green-700">
            <svg className="w-4 h-4 flex-shrink-0" fill="currentColor" viewBox="0 0 20 20">
              <path
                fillRule="evenodd"
                d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z"
                clipRule="evenodd"
              />
            </svg>
            <span>{t('compose:sentConfirm')}</span>
          </div>
        )}

        {/* Footer */}
        <div className="px-5 py-4 border-t border-gray-200 flex items-center gap-2">
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
          <div className="flex-1" />
          <button
            type="button"
            onClick={onClose}
            disabled={isSending || sent}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-lg hover:bg-gray-100 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSend}
            disabled={
              isSending ||
              sent ||
              isLoadingAttachments ||
              toRecipients.length === 0 ||
              !subject.trim() ||
              !bodyHtml.trim()
            }
            className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-lg hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isSending ? 'Sending…' : isLoadingAttachments ? 'Loading files…' : 'Send'}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
