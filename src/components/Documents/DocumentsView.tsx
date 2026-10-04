import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';
import { selectDocuments, selectInvitations, selectSelectedDoc, useSharedDocsStore } from '@/stores/sharedDocsStore';
import type { DocKind, SharedDoc } from '@/types';
import { DocumentPane } from './DocumentPane';

interface DocumentsViewProps {
  accountId: string | null;
}

interface DocRowProps {
  doc: SharedDoc;
  selected: boolean;
  onSelect: () => void;
}

function DocRow({ doc, selected, onSelect }: DocRowProps) {
  const { t } = useTranslation(['documents']);
  const others = doc.participants.length - 1;
  const status =
    doc.status === 'invited'
      ? t('documents:invitedBy', { address: doc.participants[0] ?? '' })
      : doc.status === 'left'
        ? t('documents:status.left')
        : doc.consentedAt !== null && doc.dirtySince !== null
          ? t('documents:status.pending')
          : others > 0
            ? t('documents:status.shared', { count: others })
            : t('documents:status.private');
  return (
    <li className={selected ? 'bg-gray-700/60' : 'hover:bg-gray-800'}>
      <button
        type="button"
        data-testid={`shared-doc-row-${doc.id}`}
        onClick={onSelect}
        className="w-full text-left px-4 py-2"
      >
        <span className="block text-sm text-gray-200 truncate">{doc.title}</span>
        <span className="block text-xs text-gray-500 truncate">
          {t(`documents:kind.${doc.kind}` as const)} · {status}
        </span>
      </button>
    </li>
  );
}

/**
 * The Documents view: the account's shared documents and invitations on the
 * left, the selected one open on the right. Experimental (Settings → Shared
 * documents).
 */
export function DocumentsView({ accountId }: DocumentsViewProps) {
  const { t } = useTranslation(['documents', 'settings']);
  const addLog = useLogStore((s) => s.addLog);
  const accountEmail = useAccountStore((s) => s.accounts.find((a) => a.id === accountId)?.email ?? '');
  const { setAccount, select, create, isLoading, error } = useSharedDocsStore();
  const state = useSharedDocsStore();
  const invitations = selectInvitations(state);
  const documents = selectDocuments(state);
  const selected = selectSelectedDoc(state);
  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState('');
  const [kind, setKind] = useState<DocKind>('doc');
  const [createError, setCreateError] = useState<string | null>(null);

  useEffect(() => {
    void setAccount(accountId);
  }, [accountId, setAccount]);

  const handleCreate = async () => {
    setCreateError(null);
    try {
      const doc = await create(kind, title);
      addLog('success', 'sync', `Created "${doc.title}"`);
      setCreating(false);
      setTitle('');
      select(doc.id);
    } catch (err) {
      const msg = errorText(err);
      setCreateError(msg);
      addLog('error', 'sync', `The document could not be created: ${msg}`);
    }
  };

  const shownError = createError ?? error;

  return (
    <div className="flex flex-1 min-h-0 min-w-0 overflow-hidden bg-[#1e1e1e]" data-testid="documents-view">
      <aside className="w-72 flex-shrink-0 border-r border-gray-700 flex flex-col min-h-0">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-700">
          <h2 className="text-sm font-semibold text-gray-200 flex-1">{t('documents:title')}</h2>
          <span className="px-1.5 py-0.5 rounded bg-amber-900/40 text-amber-300 text-[10px] font-semibold uppercase">
            {t('settings:dialog.experimental')}
          </span>
          <button
            type="button"
            data-testid="shared-doc-new"
            onClick={() => setCreating((c) => !c)}
            className="px-2 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white"
          >
            {t('documents:new')}
          </button>
        </div>
        {shownError && (
          <div className="mx-3 mt-3 p-2 bg-red-900/30 border border-red-800 rounded text-red-300 text-xs">
            {shownError}
          </div>
        )}
        {creating && (
          <form
            className="flex flex-col gap-2 px-4 py-3 border-b border-gray-700"
            onSubmit={(e) => {
              e.preventDefault();
              void handleCreate();
            }}
          >
            <input
              data-testid="shared-doc-new-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t('documents:newTitlePlaceholder')}
              className="px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
            />
            <div className="flex gap-2">
              <select
                data-testid="shared-doc-new-kind"
                value={kind}
                onChange={(e) => setKind(e.target.value === 'sheet' ? 'sheet' : 'doc')}
                className="flex-1 px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
              >
                <option value="doc">{t('documents:kind.doc')}</option>
                <option value="sheet">{t('documents:kind.sheet')}</option>
              </select>
              <button
                type="submit"
                data-testid="shared-doc-create"
                disabled={!title.trim()}
                className="px-2 py-1 text-xs rounded bg-gray-700 hover:bg-gray-600 text-gray-200 disabled:opacity-40"
              >
                {t('documents:create')}
              </button>
            </div>
          </form>
        )}
        <div className="flex-1 overflow-y-auto">
          {invitations.length > 0 && (
            <>
              <h3 className="px-4 pt-3 pb-1 text-[11px] font-semibold uppercase text-gray-500">
                {t('documents:invitations')}
              </h3>
              <ul>
                {invitations.map((d) => (
                  <DocRow key={d.id} doc={d} selected={d.id === selected?.id} onSelect={() => select(d.id)} />
                ))}
              </ul>
            </>
          )}
          <ul className="py-1">
            {!isLoading && documents.length === 0 && invitations.length === 0 && (
              <li className="px-4 py-3 text-xs text-gray-500">{t('documents:empty')}</li>
            )}
            {documents.map((d) => (
              <DocRow key={d.id} doc={d} selected={d.id === selected?.id} onSelect={() => select(d.id)} />
            ))}
          </ul>
        </div>
        <p className="px-4 py-2 text-[11px] text-gray-500 border-t border-gray-700">{t('documents:intro')}</p>
      </aside>
      {selected && accountEmail ? (
        <DocumentPane key={selected.id} doc={selected} accountEmail={accountEmail} />
      ) : (
        <div className="flex-1 flex items-center justify-center text-sm text-gray-500">{t('documents:selectHint')}</div>
      )}
    </div>
  );
}
