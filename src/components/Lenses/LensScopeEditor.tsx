// Modal that lets the user edit the scope (which emails to analyze) of an
// existing Lens. Scope-only edits do NOT bump prompt_version — they only
// affect future scope evaluation. New matching emails will be picked up by
// the next backfill / incremental run.

import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLensStore } from '@/stores/lensStore';
import type { Lens, LensDirection, LensScope } from '@/types';

import { validateSenderDomains, validateSenderEmails } from './scopeValidation';

interface LensScopeEditorProps {
  lens: Lens | null;
  open: boolean;
  onClose: () => void;
}

const MAILBOXES = ['inbox', 'sent', 'archive', 'spam', 'trash'] as const;
const CATEGORIES = ['Primary', 'Promotions', 'Social', 'Updates', 'Forums'] as const;

export function LensScopeEditor({ lens, open, onClose }: LensScopeEditorProps) {
  const { t } = useTranslation(['common', 'lenses']);
  const accounts = useAccountStore((s) => s.accounts);
  const updateLens = useLensStore((s) => s.updateLens);

  const [accountId, setAccountId] = useState('');
  const [mailboxes, setMailboxes] = useState<string[]>([]);
  const [categories, setCategories] = useState<string[]>([]);
  const [direction, setDirection] = useState<LensDirection>('either');
  const [lastDays, setLastDays] = useState('');
  const [senderDomains, setSenderDomains] = useState('');
  const [senderEmails, setSenderEmails] = useState('');
  const [query, setQuery] = useState('');
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    setError(null);
  }, [open, lens]);

  // Live validation so we can disable Save and surface a hint inline rather
  // than letting the user save a scope that silently matches zero emails.
  const domainCheck = useMemo(() => validateSenderDomains(senderDomains), [senderDomains]);
  const emailCheck = useMemo(() => validateSenderEmails(senderEmails), [senderEmails]);

  const buildScope = useMemo(
    () => (): LensScope => {
      const parsedDays = lastDays.trim() ? Number.parseInt(lastDays.trim(), 10) : NaN;
      return {
        accountIds: accountId ? [accountId] : null,
        mailboxes: mailboxes.length ? mailboxes : null,
        categories: categories.length ? categories : null,
        direction: direction === 'either' ? null : direction,
        query: query.trim() || null,
        senderDomains: domainCheck.values.length ? domainCheck.values : null,
        senderEmails: emailCheck.values.length ? emailCheck.values : null,
        dateRange: Number.isFinite(parsedDays) && parsedDays > 0 ? { lastDays: parsedDays } : null,
      };
    },
    [accountId, mailboxes, categories, direction, lastDays, domainCheck, emailCheck, query],
  );

  if (!lens) return null;

  const toggleIn = (list: string[], v: string, setter: (next: string[]) => void) => {
    setter(list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  };

  const handleSave = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await updateLens(lens.id, { scope: buildScope() });
      onClose();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Edit scope — ${lens.name}`}
      subtitle="Choose which emails this Lens analyzes. Scope changes apply to future runs; existing extracted rows stay until you re-run backfill."
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
            disabled={isSaving || !!domainCheck.error || !!emailCheck.error}
            className="rounded bg-blue-600 px-3 py-1 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-50"
          >
            {isSaving ? 'Saving…' : 'Save'}
          </button>
        </div>
      }
    >
      <div className="space-y-4 text-xs text-gray-300">
        <div className="grid grid-cols-2 gap-3">
          <label className="block">
            <span className="mb-1 block text-gray-400">{t('lenses:scope.account')}</span>
            <select
              value={accountId}
              onChange={(e) => setAccountId(e.target.value)}
              className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
            >
              <option value="">{t('lenses:scope.allAccounts')}</option>
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.email}
                </option>
              ))}
            </select>
          </label>
          <label className="block">
            <span className="mb-1 block text-gray-400">{t('lenses:scope.direction')}</span>
            <select
              value={direction}
              onChange={(e) => setDirection(e.target.value as LensDirection)}
              className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
            >
              <option value="either">{t('lenses:scope.either')}</option>
              <option value="inbound">{t('lenses:scope.inboundOnly')}</option>
              <option value="outbound">{t('lenses:scope.outboundOnly')}</option>
            </select>
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

        {error && <div className="text-xs text-red-400">{error}</div>}
      </div>
    </Modal>
  );
}
