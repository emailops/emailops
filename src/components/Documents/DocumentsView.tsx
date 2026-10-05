import { type DragEvent, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { isDocDrag, readDocDrag, writeDocDrag } from '@/lib/docDrag';
import { childFolders, folderPath } from '@/lib/docFolders';
import { errorText } from '@/lib/errors';
import { importOfficeFile } from '@/lib/officeImport';
import { useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';
import { selectFolderDocs, selectInvitations, selectSelectedDoc, useSharedDocsStore } from '@/stores/sharedDocsStore';
import type { DocFolder, DocKind, SharedDoc } from '@/types';
import { DeleteDocDialog } from './DeleteDocDialog';
import { DocumentPane } from './DocumentPane';

interface DocumentsViewProps {
  accountId: string | null;
}

/** Props that make an element a drop target for a dragged document; `over`
 *  says whether one is held over it right now. */
function docDropTarget(onDrop: (docId: string) => void, setOver: (over: boolean) => void) {
  return {
    onDragOver: (e: DragEvent) => {
      if (!isDocDrag(e.dataTransfer)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      setOver(true);
    },
    onDragLeave: () => setOver(false),
    onDrop: (e: DragEvent) => {
      setOver(false);
      const docId = readDocDrag(e.dataTransfer);
      if (!docId) return;
      e.preventDefault();
      onDrop(docId);
    },
  };
}

const DROP_OVER = 'bg-primary-900/40 ring-1 ring-primary-500';

interface DocRowProps {
  doc: SharedDoc;
  selected: boolean;
  /** Where the document sits, shown in search results. */
  location?: string;
  onSelect: () => void;
  onDelete: () => void;
}

function DocRow({ doc, selected, location, onSelect, onDelete }: DocRowProps) {
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
    <li className={`group flex items-center ${selected ? 'bg-gray-700/60' : 'hover:bg-gray-800'}`}>
      <button
        type="button"
        data-testid={`shared-doc-row-${doc.id}`}
        draggable
        onDragStart={(e) => writeDocDrag(e.dataTransfer, doc.id)}
        onClick={onSelect}
        className="flex-1 min-w-0 text-left px-4 py-2"
      >
        <span className="block text-sm text-gray-200 truncate">{doc.title}</span>
        <span className="block text-xs text-gray-500 truncate">
          {t(`documents:kind.${doc.kind}` as const)} · {location ?? status}
        </span>
      </button>
      <button
        type="button"
        data-testid={`shared-doc-delete-${doc.id}`}
        title={t('documents:delete')}
        aria-label={t('documents:delete')}
        onClick={onDelete}
        className="hidden group-hover:block mr-2 p-1 rounded text-gray-500 hover:text-red-300 hover:bg-gray-700"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
          />
        </svg>
      </button>
    </li>
  );
}

interface FolderRowProps {
  folder: DocFolder;
  onOpen: () => void;
  /** A document dragged onto the folder. */
  onDropDoc: (docId: string) => void;
  onRename: (name: string) => Promise<void>;
  onDelete: () => Promise<void>;
}

function FolderRow({ folder, onOpen, onDropDoc, onRename, onDelete }: FolderRowProps) {
  const { t } = useTranslation(['documents']);
  const [over, setOver] = useState(false);
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
    <li
      className={`group flex items-center ${over ? DROP_OVER : 'hover:bg-gray-800'}`}
      {...docDropTarget(onDropDoc, setOver)}
    >
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

interface CrumbProps {
  label: string;
  onOpen: () => void;
  onDropDoc: (docId: string) => void;
}

/** A breadcrumb entry: opens that folder, and takes documents dropped on it. */
function Crumb({ label, onOpen, onDropDoc }: CrumbProps) {
  const [over, setOver] = useState(false);
  return (
    <button
      type="button"
      onClick={onOpen}
      {...docDropTarget(onDropDoc, setOver)}
      className={`rounded px-1 hover:text-white ${over ? DROP_OVER : ''}`}
    >
      {label}
    </button>
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
    moveDoc,
    deleteDoc,
  } = state;
  const invitations = selectInvitations(state);
  const documents = selectFolderDocs(state);
  const selected = selectSelectedDoc(state);
  const [creating, setCreating] = useState<'doc' | 'folder' | null>(null);
  const [title, setTitle] = useState('');
  const [kind, setKind] = useState<DocKind>('doc');
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<SharedDoc | null>(null);
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

  const handleImport = async () => {
    if (!accountId) return;
    setNotice(null);
    // The native dialog, opened by the backend: it filters to importable
    // files, and the webview never names a path.
    let file: api.PickedImportFile | null;
    try {
      file = await api.pickImportFile();
    } catch (err) {
      const msg = errorText(err);
      setActionError(msg);
      addLog('error', 'sync', `The file could not be opened: ${msg}`);
      return;
    }
    if (!file) return;
    const picked = file;
    await run(`${picked.filename} could not be imported`, async () => {
      const result = await importOfficeFile(accountId, picked.filename, picked.data, folderId);
      await state.reload();
      if (result.docs[0]) select(result.docs[0].id);
      addLog('success', 'sync', `Imported ${picked.filename} into EO Docs`);
      const parts = [t('documents:import.done', { count: result.docs.length })];
      if (result.skippedImages > 0) parts.push(t('documents:import.skippedImages', { count: result.skippedImages }));
      setNotice(parts.join(' · '));
    });
  };

  const dropInto = (folder: string | null) => (docId: string) =>
    void run('The document could not be moved', () => moveDoc(docId, folder));

  const handleDelete = async (doc: SharedDoc) => {
    await deleteDoc(doc.id);
    addLog('success', 'sync', `Deleted "${doc.title}" from EO Docs`);
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
            onClick={() => void handleImport()}
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
                  onDelete={() => setDeleting(d)}
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
                      <DocRow
                        key={d.id}
                        doc={d}
                        selected={d.id === selected?.id}
                        onSelect={() => select(d.id)}
                        onDelete={() => setDeleting(d)}
                      />
                    ))}
                  </ul>
                </>
              )}
              <nav
                className="flex flex-wrap items-center gap-1 px-4 pt-3 pb-1 text-xs text-gray-400"
                data-testid="doc-breadcrumb"
              >
                <Crumb label={t('documents:root')} onOpen={() => openFolder(null)} onDropDoc={dropInto(null)} />
                {path.map((f) => (
                  <span key={f.id} className="flex items-center gap-1">
                    <span aria-hidden="true">›</span>
                    <Crumb label={f.name} onOpen={() => openFolder(f.id)} onDropDoc={dropInto(f.id)} />
                  </span>
                ))}
              </nav>
              <ul className="py-1">
                {childFolders(folders, folderId).map((f) => (
                  <FolderRow
                    key={f.id}
                    folder={f}
                    onOpen={() => openFolder(f.id)}
                    onDropDoc={dropInto(f.id)}
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
                  <DocRow
                    key={d.id}
                    doc={d}
                    selected={d.id === selected?.id}
                    onSelect={() => select(d.id)}
                    onDelete={() => setDeleting(d)}
                  />
                ))}
              </ul>
            </>
          )}
        </div>
        <p className="px-4 py-2 text-[11px] text-gray-500 border-t border-gray-700">{t('documents:intro')}</p>
      </aside>
      {selected && accountEmail ? (
        <DocumentPane
          key={selected.id}
          doc={selected}
          accountEmail={accountEmail}
          onDelete={() => setDeleting(selected)}
        />
      ) : (
        <div className="flex-1 flex items-center justify-center text-sm text-gray-500">{t('documents:selectHint')}</div>
      )}
      {deleting && (
        <DeleteDocDialog
          title={deleting.title}
          sharedWith={deleting.participants.filter((p) => p !== accountEmail.toLowerCase()).length}
          onDelete={() => handleDelete(deleting)}
          onClose={() => setDeleting(null)}
        />
      )}
    </div>
  );
}
