import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { SharedDoc } from '@/types';

const PRIMARY =
  'rounded bg-primary-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-primary-700 disabled:opacity-50';
const SECONDARY = 'rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700';

interface EoDocsAttachDialogProps {
  /** The account the email is sent from: only its documents can be attached. */
  accountId: string;
  onAttach: (docs: SharedDoc[]) => void;
  onClose: () => void;
}

/**
 * Pick EO Docs to attach to an email. Attaching shares the documents with
 * the email's recipients when it is sent, so the dialog says that EO Docs
 * only works between EmailOps users and asks for the same consent to
 * automatic mail that sharing does.
 */
export function EoDocsAttachDialog({ accountId, onAttach, onClose }: EoDocsAttachDialogProps) {
  const { t } = useTranslation(['documents']);
  const [docs, setDocs] = useState<SharedDoc[] | null>(null);
  const [query, setQuery] = useState('');
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [consent, setConsent] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listSharedDocs(accountId)
      .then((all) => {
        if (!cancelled) setDocs(all.filter((d) => d.status === 'active'));
      })
      .catch((err) => {
        if (!cancelled) setError(errorText(err));
      });
    return () => {
      cancelled = true;
    };
  }, [accountId]);

  const needle = query.trim().toLowerCase();
  const shown = (docs ?? []).filter((d) => !needle || d.title.toLowerCase().includes(needle));
  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <Modal
      open
      onClose={onClose}
      size="md"
      title={t('documents:attachDialog.title')}
      subtitle={error ? <span className="text-red-300">{error}</span> : undefined}
      footer={
        <>
          <button type="button" onClick={onClose} className={SECONDARY}>
            {t('documents:cancel')}
          </button>
          <button
            type="button"
            data-testid="eodocs-attach-submit"
            disabled={!consent || picked.size === 0}
            onClick={() => onAttach((docs ?? []).filter((d) => picked.has(d.id)))}
            className={PRIMARY}
          >
            {t('documents:attachDialog.submit')}
          </button>
        </>
      }
    >
      <p
        data-testid="eodocs-attach-warning"
        className="mb-3 rounded border border-amber-700 bg-amber-900/30 p-2 text-xs text-amber-200"
      >
        {t('documents:attachDialog.warning')}
      </p>
      <input
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t('documents:attachDialog.search')}
        aria-label={t('documents:attachDialog.search')}
        className="w-full rounded border border-gray-600 bg-gray-800 px-2 py-1 text-sm text-gray-200"
      />
      <ul className="mt-2 max-h-64 overflow-y-auto">
        {docs !== null && shown.length === 0 && (
          <li className="px-1 py-2 text-xs text-gray-500">{t('documents:attachDialog.empty')}</li>
        )}
        {shown.map((d) => (
          <li key={d.id}>
            <label className="flex items-center gap-2 rounded px-1 py-1.5 text-sm text-gray-200 hover:bg-gray-700">
              <input
                type="checkbox"
                data-testid={`eodocs-attach-${d.id}`}
                checked={picked.has(d.id)}
                onChange={() => toggle(d.id)}
              />
              <span className="truncate">{d.title}</span>
              <span className="ml-auto text-xs text-gray-500">{t(`documents:kind.${d.kind}` as const)}</span>
            </label>
          </li>
        ))}
      </ul>
      <label className="mt-3 flex items-start gap-2 text-sm text-gray-300">
        <input
          type="checkbox"
          data-testid="eodocs-attach-consent"
          checked={consent}
          onChange={(e) => setConsent(e.target.checked)}
          className="mt-0.5"
        />
        <span>{t('documents:attachDialog.consent')}</span>
      </label>
    </Modal>
  );
}
