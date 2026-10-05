import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { childFolders, folderPath } from '@/lib/docFolders';
import { errorText } from '@/lib/errors';
import { fileToBase64 } from '@/lib/fileBase64';
import { importOfficeFile } from '@/lib/officeImport';
import { useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';
import { selectFolderDocs, selectInvitations, selectSelectedDoc, useSharedDocsStore } from '@/stores/sharedDocsStore';
import type { DocFolder, DocKind, SharedDoc } from '@/types';
import { DocumentPane } from './DocumentPane';

interface DocumentsViewProps {
  accountId: string | null;
}

interface DocRowProps {
  doc: SharedDoc;
  selected: boolean;
  /** Where the document sits, shown in search results. */
  location?: string;
  onSelect: () => void;
}

function DocRow({ doc, selected, location, onSelect }: DocRowProps) {
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
          {t(`documents:kind.${doc.kind}` as const)} · {location ?? status}
        </span>
      </button>
    </li>
  );
}

interface FolderRowProps {
  folder: DocFolder;
  onOpen: () => void;
  onRename: (name: string) => Promise<void>;
  onDelete: () => Promise<void>;
}

function FolderRow({ folder, onOpen, onRename, onDelete }: FolderRowProps) {
  const { t } = useTranslation(['documents']);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(folder.name);
  const [confirming, setConfirming] = useState(false);

  if (editing) {
    return (
      <li className="px-4 py-1">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void onRename(name).then(() => setEditing(false));
          }}
        >
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label={t('documents:folders.name')}
            className="w-full px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
          />
        </form>
      </li>
    );
  }
  return (
    <li className="group flex items-center hover:bg-gray-800">
      <button
        type="button"
        data-testid={`doc-folder-${folder.id}`}
        onClick={onOpen}
        className="flex-1 min-w-0 flex items-center gap-2 text-left px-4 py-2"
      >
        <svg className="w-4 h-4 flex-shrink-0 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V7z"
          />
        </svg>
        <span className="text-sm text-gray-200 truncate">{folder.name}</span>
      </button>
      {confirming ? (
        <span className="flex items-center gap-1 pr-2">
          <button
            type="button"
            title={t('documents:folders.deleteConfirm')}
            onClick={() => void onDelete()}
            className="px-1.5 py-0.5 text-[11px] rounded bg-red-700 hover:bg-red-600 text-white"
          >
            {t('documents:folders.delete')}
          </button>
          <button
            type="button"
            onClick={() => setConfirming(false)}
            className="px-1.5 py-0.5 text-[11px] rounded bg-gray-700 text-gray-200"
          >
            {t('documents:cancel')}
          </button>
        </span>
      ) : (
        <span className="hidden group-hover:flex items-center gap-1 pr-2">
          <button
            type="button"
            onClick={() => setEditing(true)}
            className="px-1.5 py-0.5 text-[11px] rounded text-gray-400 hover:text-white hover:bg-gray-700"
          >
            {t('documents:folders.rename')}
          </button>
          <button
            type="button"
            onClick={() => setConfirming(true)}
            className="px-1.5 py-0.5 text-[11px] rounded text-gray-400 hover:text-red-300 hover:bg-gray-700"
          >
            {t('documents:folders.delete')}
          </button>
        </span>
      )}
    </li>
  );
}

/**
 * EO Docs: the account's documents, sheets and personal folders on the left
 * (with search across all of them), the selected document on the right.
 * Experimental (Settings → EO Docs).
 */
export function DocumentsView({ accountId }: DocumentsViewProps) {
  const { t } = useTranslation(['documents', 'settings']);
  const addLog = useLogStore((s) => s.addLog);
  const accountEmail = useAccountStore((s) => s.accounts.find((a) => a.id === accountId)?.email ?? '');
  const state = useSharedDocsStore();
  const {
    setAccount,
    select,
    create,
    isLoading,
    error,
    folders,
    folderId,
    openFolder,
    createFolder,
    renameFolder,
    deleteFolder,
    search,
    searchQuery,
    searchResults,
  } = state;
  const invitations = selectInvitations(state);
  const documents = selectFolderDocs(state);
  const selected = selectSelectedDoc(state);
  const [creating, setCreating] = useState<'doc' | 'folder' | null>(null);
  const [title, setTitle] = useState('');
  const [kind, setKind] = useState<DocKind>('doc');
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const importInputRef = useRef<HTMLInputElement>(null);
  const searching = searchQuery.trim() !== '';
  const path = folderPath(folders, folderId);

  useEffect(() => {
    void setAccount(accountId);
  }, [accountId, setAccount]);

  const run = async (what: string, fn: () => Promise<unknown>) => {
    setActionError(null);
    try {
      await fn();
      return true;
    } catch (err) {
      const msg = errorText(err);
      setActionError(msg);
      addLog('error', 'sync', `${what}: ${msg}`);
      return false;
    }
  };

  const handleCreate = async () => {
    const ok = await run(
      creating === 'folder' ? 'The folder could not be created' : 'The document could not be created',
      async () => {
        if (creating === 'folder') {
          await createFolder(title);
          return;
        }
        const doc = await create(kind, title);
        addLog('success', 'sync', `Created "${doc.title}"`);
        // A new document goes into the folder on screen.
        const placed = folderId ? await state.moveDoc(doc.id, folderId) : doc;
        select(placed.id);
      },
    );
    if (ok) {
      setCreating(null);
      setTitle('');
    }
  };

  const handleImport = async (files: FileList | null) => {
    const file = files?.[0];
    if (importInputRef.current) importInputRef.current.value = '';
    if (!file || !accountId) return;
    setNotice(null);
    await run(`${file.name} could not be imported`, async () => {
      const result = await importOfficeFile(accountId, file.name, await fileToBase64(file), folderId);
      await state.reload();
      if (result.docs[0]) select(result.docs[0].id);
      addLog('success', 'sync', `Imported ${file.name} into EO Docs`);
      const parts = [t('documents:import.done', { count: result.docs.length })];
      if (result.skippedImages > 0) parts.push(t('documents:import.skippedImages', { count: result.skippedImages }));
      setNotice(parts.join(' · '));
    });
  };

  const locationOf = (doc: SharedDoc) =>
    [t('documents:root'), ...folderPath(folders, doc.folderId).map((f) => f.name)].join(' › ');

  const shownError = actionError ?? error;

  return (
    <div className="flex flex-1 min-h-0 min-w-0 overflow-hidden bg-[#1e1e1e]" data-testid="documents-view">
      <aside className="w-72 flex-shrink-0 border-r border-gray-700 flex flex-col min-h-0">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-700">
          <h2 className="text-sm font-semibold text-gray-200 flex-1 whitespace-nowrap">{t('documents:title')}</h2>
          <span className="px-1.5 py-0.5 rounded bg-amber-900/40 text-amber-300 text-[10px] font-semibold uppercase">
            {t('settings:dialog.experimental')}
          </span>
          <button
            type="button"
            data-testid="doc-folder-new"
            title={t('documents:folders.new')}
            aria-label={t('documents:folders.new')}
            onClick={() => setCreating((c) => (c === 'folder' ? null : 'folder'))}
            className="p-1 rounded text-gray-300 hover:text-white hover:bg-gray-700"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V7zm9 3v6m-3-3h6"
              />
            </svg>
          </button>
          <button
            type="button"
            data-testid="shared-doc-import"
            title={t('documents:import.hint')}
            aria-label={t('documents:import.button')}
            onClick={() => importInputRef.current?.click()}
            className="p-1 rounded text-gray-300 hover:text-white hover:bg-gray-700"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"
              />
            </svg>
          </button>
          <input
            ref={importInputRef}
            type="file"
            accept=".docx,.xlsx,.xlsm,.xls,.ods"
            className="hidden"
            data-testid="shared-doc-import-input"
            onChange={(e) => void handleImport(e.target.files)}
          />
          <button
            type="button"
            data-testid="shared-doc-new"
            onClick={() => setCreating((c) => (c === 'doc' ? null : 'doc'))}
            className="px-2 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white"
          >
            {t('documents:new')}
          </button>
        </div>
        <div className="px-3 py-2 border-b border-gray-700">
          <input
            type="search"
            data-testid="shared-doc-search"
            value={searchQuery}
            onChange={(e) => void search(e.target.value)}
            placeholder={t('documents:searchPlaceholder')}
            aria-label={t('documents:searchPlaceholder')}
            className="w-full px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
          />
        </div>
        {shownError && (
          <div className="mx-3 mt-3 p-2 bg-red-900/30 border border-red-800 rounded text-red-300 text-xs">
            {shownError}
          </div>
        )}
        {notice && !shownError && (
          <div
            data-testid="shared-doc-notice"
            className="mx-3 mt-3 p-2 bg-gray-800 border border-gray-700 rounded text-gray-300 text-xs"
          >
            {notice}
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
              placeholder={creating === 'folder' ? t('documents:folders.name') : t('documents:newTitlePlaceholder')}
              className="px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
            />
            <div className="flex gap-2">
              {creating === 'doc' && (
                <select
                  data-testid="shared-doc-new-kind"
                  value={kind}
                  onChange={(e) => setKind(e.target.value === 'sheet' ? 'sheet' : 'doc')}
                  className="flex-1 px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
                >
                  <option value="doc">{t('documents:kind.doc')}</option>
                  <option value="sheet">{t('documents:kind.sheet')}</option>
                </select>
              )}
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
          {searching ? (
            <ul className="py-1" data-testid="shared-doc-results">
              {searchResults?.length === 0 && (
                <li className="px-4 py-3 text-xs text-gray-500">
                  {t('documents:noResults', { query: searchQuery.trim() })}
                </li>
              )}
              {searchResults?.map((d) => (
                <DocRow
                  key={d.id}
                  doc={d}
                  selected={d.id === selected?.id}
                  location={locationOf(d)}
                  onSelect={() => select(d.id)}
                />
              ))}
            </ul>
          ) : (
            <>
              {folderId === null && invitations.length > 0 && (
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
              <nav
                className="flex flex-wrap items-center gap-1 px-4 pt-3 pb-1 text-xs text-gray-400"
                data-testid="doc-breadcrumb"
              >
                <button type="button" onClick={() => openFolder(null)} className="hover:text-white">
                  {t('documents:root')}
                </button>
                {path.map((f) => (
                  <span key={f.id} className="flex items-center gap-1">
                    <span aria-hidden="true">›</span>
                    <button type="button" onClick={() => openFolder(f.id)} className="hover:text-white">
                      {f.name}
                    </button>
                  </span>
                ))}
              </nav>
              <ul className="py-1">
                {childFolders(folders, folderId).map((f) => (
                  <FolderRow
                    key={f.id}
                    folder={f}
                    onOpen={() => openFolder(f.id)}
                    onRename={async (name) => {
                      await run('The folder could not be renamed', () => renameFolder(f.id, name));
                    }}
                    onDelete={async () => {
                      await run('The folder could not be deleted', () => deleteFolder(f.id));
                    }}
                  />
                ))}
                {!isLoading &&
                  documents.length === 0 &&
                  childFolders(folders, folderId).length === 0 &&
                  invitations.length === 0 && (
                    <li className="px-4 py-3 text-xs text-gray-500">{t('documents:empty')}</li>
                  )}
                {documents.map((d) => (
                  <DocRow key={d.id} doc={d} selected={d.id === selected?.id} onSelect={() => select(d.id)} />
                ))}
              </ul>
            </>
          )}
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
