// Combined Lens configuration modal with two tabs: Scope and Prompt.
// Replaces the separate LensScopeEditor + LensPromptEditor modals.
// - Scope tab: filters that determine which emails are analyzed (no version bump on save).
// - Prompt tab: the extraction prompt (bumps prompt_version on save, marking existing rows stale).

import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import { Select } from '@/components/shared/Select';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLensStore } from '@/stores/lensStore';
import type { Lens, LensDirection, LensScope } from '@/types';

import { validateSenderDomains, validateSenderEmails } from './scopeValidation';

interface LensConfigModalProps {
  lens: Lens | null;
  open: boolean;
  onClose: () => void;
}

const MAILBOXES = ['inbox', 'sent', 'archive', 'spam', 'trash'] as const;
const CATEGORIES = ['Primary', 'Promotions', 'Social', 'Updates', 'Forums'] as const;

export function LensConfigModal({ lens, open, onClose }: LensConfigModalProps) {
  const { t } = useTranslation(['common', 'lenses']);
  const accounts = useAccountStore((s) => s.accounts);
  const updateLens = useLensStore((s) => s.updateLens);

  const [activeTab, setActiveTab] = useState<'scope' | 'prompt'>('scope');
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // ── Scope state ──────────────────────────────────────────────────────────
  const [accountId, setAccountId] = useState('');
  const [mailboxes, setMailboxes] = useState<string[]>([]);
  const [categories, setCategories] = useState<string[]>([]);
  const [direction, setDirection] = useState<LensDirection>('either');
  const [lastDays, setLastDays] = useState('');
  const [senderDomains, setSenderDomains] = useState('');
  const [senderEmails, setSenderEmails] = useState('');
  const [query, setQuery] = useState('');
  const [querySearchBody, setQuerySearchBody] = useState(true);

  // ── Prompt state ─────────────────────────────────────────────────────────
  const [promptText, setPromptText] = useState('');

  // Re-seed form whenever the modal opens or the active Lens changes.
  useEffect(() => {
    if (!open || !lens) return;
    const s = lens.scope ?? {};
    setAccountId(s.accountIds && s.accountIds.length === 1 ? s.accountIds[0] : '');
    setMailboxes(s.mailboxes ?? []);
    setCategories(s.categories ?? []);
    setDirection(s.direction ?? 'either');
    setLastDays(s.dateRange?.lastDays != null ? String(s.dateRange.lastDays) : '');
    setSenderDomains((s.senderDomains ?? []).join(', '));
    setSenderEmails((s.senderEmails ?? []).join(', '));
    setQuery(s.query ?? '');
    setQuerySearchBody(s.querySearchBody ?? false);
    setPromptText(lens.promptText);
    setActiveTab('scope');
    setError(null);
  }, [open, lens]);

  const domainCheck = useMemo(() => validateSenderDomains(senderDomains), [senderDomains]);
  const emailCheck = useMemo(() => validateSenderEmails(senderEmails), [senderEmails]);

  const buildScope = (): LensScope => {
    const parsedDays = lastDays.trim() ? Number.parseInt(lastDays.trim(), 10) : NaN;
    const scope: LensScope = {
      accountIds: accountId ? [accountId] : null,
      mailboxes: mailboxes.length ? mailboxes : null,
      categories: categories.length ? categories : null,
      direction: direction === 'either' ? null : direction,
      query: query.trim() || null,
      senderDomains: domainCheck.values.length ? domainCheck.values : null,
      senderEmails: emailCheck.values.length ? emailCheck.values : null,
      dateRange: Number.isFinite(parsedDays) && parsedDays > 0 ? { lastDays: parsedDays } : null,
    };
    // Only include querySearchBody when true — backend defaults to false (subject only).
    if (querySearchBody) scope.querySearchBody = true;
    return scope;
  };

  const promptDirty = promptText.trim() !== (lens?.promptText ?? '').trim();

  const saveDisabled =
    isSaving ||
    (activeTab === 'scope' && (!!domainCheck.error || !!emailCheck.error)) ||
    (activeTab === 'prompt' && !promptDirty);

  const handleSave = async () => {
    if (!lens) return;
    setIsSaving(true);
    setError(null);
    try {
      if (activeTab === 'scope') {
        await updateLens(lens.id, { scope: buildScope() });
      } else {
        await updateLens(lens.id, { promptText });
      }
      onClose();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsSaving(false);
    }
  };

  if (!lens) return null;

  const toggleIn = (list: string[], v: string, setter: (next: string[]) => void) => {
    setter(list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Config — ${lens.name}`}
      size="lg"
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={isSaving}
            className="rounded border border-gray-600 px-3 py-1 text-xs text-gray-200 hover:bg-gray-700 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void handleSave()}
            disabled={saveDisabled}
            className="rounded bg-blue-600 px-3 py-1 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-50"
          >
            {isSaving ? 'Saving…' : 'Save'}
          </button>
        </div>
      }
    >
      {/* Tab bar */}
      <div className="mb-4 flex border-b border-gray-700">
        {(['scope', 'prompt'] as const).map((tab) => (
          <button
            key={tab}
            type="button"
            onClick={() => setActiveTab(tab)}
            className={`px-4 py-1.5 text-xs font-medium capitalize transition-colors ${
              activeTab === tab ? 'border-b-2 border-blue-500 text-blue-300' : 'text-gray-400 hover:text-gray-200'
            }`}
          >
            {tab}
          </button>
        ))}
      </div>

      {/* Scope tab */}
      {activeTab === 'scope' && (
        <div className="space-y-4 text-xs text-gray-300">
          <p className="text-[11px] text-gray-500">
            Choose which emails this Lens analyzes. Scope changes apply to future runs; existing extracted rows stay
            until you re-run backfill.
          </p>

          <div className="grid grid-cols-2 gap-3">
            <label className="block">
              <span className="mb-1 block text-gray-400">{t('lenses:scope.account')}</span>
              <Select
                value={accountId}
                options={[
                  { value: '', label: t('lenses:scope.allAccounts') },
                  ...accounts.map((a) => ({ value: a.id, label: a.email })),
                ]}
                onChange={(value) => setAccountId(value)}
                ariaLabel={t('lenses:scope.account')}
                fullWidth
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-gray-400">{t('lenses:scope.direction')}</span>
              <Select
                value={direction}
                options={[
                  { value: 'either', label: t('lenses:scope.either') },
                  { value: 'inbound', label: t('lenses:scope.inboundOnly') },
                  { value: 'outbound', label: t('lenses:scope.outboundOnly') },
                ]}
                onChange={(value) => setDirection(value as LensDirection)}
                ariaLabel={t('lenses:scope.direction')}
                fullWidth
              />
            </label>
          </div>

          <div className="space-y-1">
            <span className="block text-gray-400">{t('lenses:scope.mailboxes')}</span>
            <div className="flex flex-wrap gap-1.5">
              {MAILBOXES.map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => toggleIn(mailboxes, m, setMailboxes)}
                  className={`rounded border px-2 py-0.5 text-[11px] ${
                    mailboxes.includes(m)
                      ? 'border-blue-500 bg-blue-600/30 text-blue-200'
                      : 'border-gray-600 text-gray-300 hover:bg-gray-700'
                  }`}
                >
                  {m}
                </button>
              ))}
            </div>
            <p className="text-[10px] text-gray-500">{t('lenses:scope.mailboxesEmptyHelp')}</p>
          </div>

          <div className="space-y-1">
            <span className="block text-gray-400">{t('lenses:scope.categories')}</span>
            <div className="flex flex-wrap gap-1.5">
              {CATEGORIES.map((c) => (
                <button
                  key={c}
                  type="button"
                  onClick={() => toggleIn(categories, c, setCategories)}
                  className={`rounded border px-2 py-0.5 text-[11px] ${
                    categories.includes(c)
                      ? 'border-blue-500 bg-blue-600/30 text-blue-200'
                      : 'border-gray-600 text-gray-300 hover:bg-gray-700'
                  }`}
                >
                  {c}
                </button>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <label className="block">
              <span className="mb-1 block text-gray-400">{t('lenses:scope.lastNDays')}</span>
              <input
                type="number"
                min={1}
                value={lastDays}
                onChange={(e) => setLastDays(e.target.value)}
                placeholder={t('lenses:scope.lastNDaysPlaceholder')}
                className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-gray-400">{t('lenses:scope.senderDomains')}</span>
              <input
                type="text"
                value={senderDomains}
                onChange={(e) => setSenderDomains(e.target.value)}
                placeholder="stripe.com, wise.com" // i18n-ignore: example sender domains
                className={`w-full rounded border bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:outline-none ${
                  domainCheck.error ? 'border-red-500 focus:border-red-400' : 'border-gray-600 focus:border-blue-500'
                }`}
              />
              {domainCheck.error && <p className="mt-1 text-[10px] text-red-400">{domainCheck.error}</p>}
            </label>
          </div>

          <label className="block">
            <span className="mb-1 block text-gray-400">{t('lenses:scope.senderEmails')}</span>
            <input
              type="text"
              value={senderEmails}
              onChange={(e) => setSenderEmails(e.target.value)}
              placeholder="billing@stripe.com, invoices@vendor.com" // i18n-ignore: example sender emails
              className={`w-full rounded border bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:outline-none ${
                emailCheck.error ? 'border-red-500 focus:border-red-400' : 'border-gray-600 focus:border-blue-500'
              }`}
            />
            {emailCheck.error && <p className="mt-1 text-[10px] text-red-400">{emailCheck.error}</p>}
          </label>

          <div className="space-y-2">
            <label className="block">
              <span className="mb-1 block text-gray-400">{t('lenses:scope.keywordQuery')}</span>
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder='e.g. "invoice" OR "receipt"' // i18n-ignore: FTS5 query syntax sample
                className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
              />
            </label>
            <label className="flex items-center gap-2 text-[11px] text-gray-300">
              <input
                type="checkbox"
                checked={querySearchBody}
                onChange={(e) => setQuerySearchBody(e.target.checked)}
                className="h-3 w-3 accent-blue-500"
              />
              {t('lenses:scope.searchBody')}
              <span className="text-gray-500">{t('lenses:scope.keywordsBodyHint')}</span>
            </label>
          </div>

          {error && <div className="text-xs text-red-400">{error}</div>}
        </div>
      )}

      {/* Prompt tab */}
      {activeTab === 'prompt' && (
        <div className="space-y-3">
          <p className="text-[11px] text-gray-500">
            This prompt is sent to the model alongside each email's content. Saving will mark all existing rows as stale
            (prompt_version bump) so they can be re-extracted.
          </p>
          <div className="text-[11px] text-gray-500">
            Prompt version: <span className="text-gray-300">{lens.promptVersion}</span>
          </div>
          <textarea
            value={promptText}
            onChange={(e) => setPromptText(e.currentTarget.value)}
            spellCheck={false}
            rows={14}
            className="w-full rounded border border-gray-600 bg-[#1e1e1e] p-3 font-mono text-xs leading-relaxed text-gray-100 focus:border-blue-500 focus:outline-none"
            placeholder={t('lenses:prompt.placeholder')}
          />
          {error && <div className="text-xs text-red-400">{error}</div>}
        </div>
      )}
    </Modal>
  );
}
