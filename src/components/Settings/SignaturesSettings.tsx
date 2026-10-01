import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RichTextEditor } from '@/components/shared/RichTextEditor';
import { Select } from '@/components/shared/Select';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { useSignatureStore } from '@/stores/signatureStore';
import type { Account, AccountSignature, SignatureInput } from '@/types';

type Status = { kind: 'idle' } | { kind: 'busy'; label: string } | { kind: 'notice'; text: string };

function inputOf(signature: AccountSignature): SignatureInput {
  return { html: signature.html, useForNew: signature.useForNew, useForReplies: signature.useForReplies };
}

/**
 * Settings → Signatures: one signature per account, edited with the compose
 * editor (so it carries the same formatting and pasted images), plus where
 * the composers insert it. The backend sanitizes on save; the editor then
 * shows what was stored.
 */
export function SignaturesSettings({ accounts }: { accounts: Account[] }) {
  const { t } = useTranslation(['settings']);
  const addLog = useLogStore((s) => s.addLog);
  const load = useSignatureStore((s) => s.load);
  const save = useSignatureStore((s) => s.save);
  const [accountId, setAccountId] = useState(accounts[0]?.id ?? '');
  const [savedInput, setSavedInput] = useState<SignatureInput | null>(null);
  const [draft, setDraft] = useState<SignatureInput | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'idle' });
  const [error, setError] = useState<string | null>(null);

  const account = accounts.find((a) => a.id === accountId);

  useEffect(() => {
    if (!accountId) return;
    let stale = false;
    setDraft(null);
    setError(null);
    setStatus({ kind: 'idle' });
    void load(accountId).then((signature) => {
      if (stale) return;
      if (!signature) {
        setError(t('settings:signatures.loadFailed'));
        return;
      }
      setSavedInput(inputOf(signature));
      setDraft(inputOf(signature));
    });
    return () => {
      stale = true;
    };
  }, [accountId, load, t]);

  if (accounts.length === 0) {
    return (
      <div className="flex-1 overflow-y-auto px-6 py-5">
        <p className="text-sm text-gray-400">{t('settings:signatures.noAccounts')}</p>
      </div>
    );
  }

  const dirty =
    draft !== null &&
    savedInput !== null &&
    (draft.html !== savedInput.html ||
      draft.useForNew !== savedInput.useForNew ||
      draft.useForReplies !== savedInput.useForReplies);
  const busy = status.kind === 'busy';

  const update = (patch: Partial<SignatureInput>) => {
    setDraft((current) => (current ? { ...current, ...patch } : current));
    setStatus({ kind: 'idle' });
  };

  const handleSave = async () => {
    if (!draft) return;
    setError(null);
    setStatus({ kind: 'busy', label: t('settings:signatures.saving') });
    try {
      const stored = await save(accountId, draft);
      setSavedInput(inputOf(stored));
      setDraft(inputOf(stored));
      setStatus({ kind: 'notice', text: t('settings:signatures.saved') });
    } catch (err) {
      setStatus({ kind: 'idle' });
      setError(errorText(err));
      addLog('error', 'account', `Could not save the signature: ${errorText(err)}`);
    }
  };

  const handleImport = async () => {
    setError(null);
    setStatus({ kind: 'busy', label: t('settings:signatures.importing') });
    addLog('info', 'account', 'Importing the Gmail signature…');
    try {
      const html = await api.importProviderSignature(accountId);
      if (html) {
        update({ html });
        setStatus({ kind: 'notice', text: t('settings:signatures.imported') });
        addLog('success', 'account', 'Gmail signature imported into the editor');
      } else {
        setStatus({ kind: 'notice', text: t('settings:signatures.importNone') });
      }
    } catch (err) {
      setStatus({ kind: 'idle' });
      setError(errorText(err));
      addLog('error', 'account', `Could not import the Gmail signature: ${errorText(err)}`);
    }
  };

  return (
    <div className="flex-1 overflow-y-auto px-6 py-5 space-y-4" data-testid="signatures-settings">
      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:signatures.title')}</h3>
        <p className="text-xs text-gray-500">{t('settings:signatures.help')}</p>
      </section>

      {accounts.length > 1 && (
        <Select
          ariaLabel={t('settings:signatures.account')}
          value={accountId}
          options={accounts.map((a) => ({ value: a.id, label: a.email }))}
          onChange={setAccountId}
          size="sm"
        />
      )}

      {draft && (
        <>
          <div className="bg-white rounded border border-gray-600 text-gray-900">
            <RichTextEditor
              key={accountId}
              value={draft.html}
              onChange={(html) => update({ html })}
              placeholder={t('settings:signatures.editorPlaceholder')}
              disabled={busy}
            />
          </div>
          <p className="text-xs text-gray-500">{t('settings:signatures.imagesHelp')}</p>

          <div className="space-y-2">
            <label className="flex items-center gap-2 text-sm text-gray-300">
              <input
                type="checkbox"
                checked={draft.useForNew}
                onChange={(e) => update({ useForNew: e.target.checked })}
                disabled={busy}
              />
              {t('settings:signatures.useForNew')}
            </label>
            <label className="flex items-center gap-2 text-sm text-gray-300">
              <input
                type="checkbox"
                checked={draft.useForReplies}
                onChange={(e) => update({ useForReplies: e.target.checked })}
                disabled={busy}
              />
              {t('settings:signatures.useForReplies')}
            </label>
            <p className="text-xs text-gray-500">{t('settings:signatures.placementHelp')}</p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => void handleSave()}
              disabled={busy || !dirty}
              className="px-3 py-1.5 text-sm bg-primary-600 text-white rounded hover:bg-primary-500 disabled:opacity-50"
            >
              {t('settings:signatures.save')}
            </button>
            {dirty && (
              <button
                type="button"
                onClick={() => {
                  setDraft(savedInput);
                  setStatus({ kind: 'idle' });
                }}
                disabled={busy}
                className="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded"
              >
                {t('settings:signatures.discard')}
              </button>
            )}
            {account?.provider === 'gmail' && (
              <button
                type="button"
                onClick={() => void handleImport()}
                disabled={busy}
                className="px-3 py-1.5 text-sm text-gray-300 border border-gray-600 hover:text-white hover:bg-gray-700 rounded"
              >
                {t('settings:signatures.importGmail')}
              </button>
            )}
            {status.kind === 'busy' && <span className="text-xs text-gray-400">{status.label}</span>}
            {status.kind === 'notice' && <span className="text-xs text-green-400">{status.text}</span>}
          </div>
        </>
      )}
      {error && <p className="text-xs text-red-400">{error}</p>}
    </div>
  );
}
