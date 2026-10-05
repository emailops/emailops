import type { Editor } from '@tiptap/react';
import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import { useSharedYDoc } from '@/hooks/useSharedYDoc';
import { useVersionDoc } from '@/hooks/useVersionDoc';
import * as api from '@/lib/api';
import { folderOptions } from '@/lib/docFolders';
import { errorText } from '@/lib/errors';
import { gridToHtml, readGrid } from '@/lib/sheetModel';
import { useLogStore } from '@/stores/logStore';
import { useSharedDocsStore } from '@/stores/sharedDocsStore';
import type { DocVersion, SharedDoc } from '@/types';
import { DocEditor } from './DocEditor';
import { HistoryPanel } from './HistoryPanel';
import { ShareDialog } from './ShareDialog';
import { SheetEditor } from './SheetEditor';

const BUTTON = 'px-2 py-1 text-xs rounded bg-gray-700 hover:bg-gray-600 text-gray-200 disabled:opacity-40';
const NOOP = () => {};

interface DocumentPaneProps {
  doc: SharedDoc;
  /** The address of the account the document belongs to. */
  accountEmail: string;
}

/** One open document: its header (sharing, status) and its editor. */
export function DocumentPane({ doc, accountEmail }: DocumentPaneProps) {
  const { t, i18n } = useTranslation(['documents']);
  const addLog = useLogStore((s) => s.addLog);
  const share = useSharedDocsStore((s) => s.share);
  const accept = useSharedDocsStore((s) => s.accept);
  const leave = useSharedDocsStore((s) => s.leave);
  const reload = useSharedDocsStore((s) => s.reload);
  const folders = useSharedDocsStore((s) => s.folders);
  const moveDoc = useSharedDocsStore((s) => s.moveDoc);
  const fmt = useFormatters();
  const [historyOpen, setHistoryOpen] = useState(false);
  const [version, setVersion] = useState<DocVersion | null>(null);
  const { doc: versionDoc, error: versionError } = useVersionDoc(doc.accountId, doc.id, version?.id ?? null);
  const consented = doc.consentedAt !== null;
  const editable = doc.status === 'active';
  const { doc: ydoc, error: syncError } = useSharedYDoc({
    accountId: doc.accountId,
    docId: doc.id,
    mailOnClose: consented && editable,
  });
  const editorRef = useRef<Editor | null>(null);
  const onEditor = useCallback((editor: Editor | null) => {
    editorRef.current = editor;
  }, []);
  const [sharing, setSharing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const others = doc.participants.filter((p) => p !== accountEmail.toLowerCase());

  const act = async (what: string, fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
    } catch (err) {
      const msg = errorText(err);
      setError(msg);
      addLog('error', 'sync', `${what}: ${msg}`);
    }
  };

  const handleShare = async (recipients: string[]) => {
    const snapshot =
      doc.kind === 'sheet'
        ? ydoc
          ? gridToHtml(readGrid(ydoc), i18n.language)
          : null
        : (editorRef.current?.getHTML() ?? null);
    await share(doc.id, recipients, snapshot);
    addLog('success', 'sync', `Shared "${doc.title}" with ${recipients.join(', ')}`);
  };

  const handleSendNow = () =>
    act('The changes could not be sent', async () => {
      const sent = await api.flushSharedDoc(doc.accountId, doc.id);
      if (sent) addLog('success', 'sync', `Changes to "${doc.title}" sent`);
      await reload();
    });

  const shownError = error ?? syncError ?? versionError;

  return (
    <section className="flex-1 min-w-0 flex flex-col min-h-0">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-700">
        <div className="flex-1 min-w-0">
          <h2 className="text-sm font-semibold text-gray-100 truncate">{doc.title}</h2>
          <p className="text-xs text-gray-500 truncate" data-testid="shared-doc-participants">
            {others.length === 0
              ? t('documents:status.private')
              : `${t('documents:participants')}: ${others.join(', ')}`}
          </p>
        </div>
        {doc.status === 'invited' && (
          <>
            <button
              type="button"
              data-testid="shared-doc-accept"
              onClick={() => void act('The invitation could not be accepted', () => accept(doc.id))}
              className="px-2 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white"
            >
              {t('documents:accept')}
            </button>
            <button
              type="button"
              data-testid="shared-doc-decline"
              onClick={() => void act('The invitation could not be declined', () => leave(doc.id))}
              className={BUTTON}
            >
              {t('documents:decline')}
            </button>
          </>
        )}
        {/* Always offered once shared: the list's pending mark only refreshes on reload. */}
        {editable && consented && (
          <button
            type="button"
            data-testid="shared-doc-send-now"
            onClick={() => void handleSendNow()}
            className={BUTTON}
          >
            {t('documents:sendNow')}
          </button>
        )}
        {editable && (
          <button
            type="button"
            data-testid="shared-doc-share"
            onClick={() => setSharing(true)}
            disabled={!ydoc}
            className="px-2 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white disabled:opacity-40"
          >
            {t('documents:share')}
          </button>
        )}
        <select
          data-testid="shared-doc-move"
          aria-label={t('documents:folders.moveTo')}
          title={t('documents:folders.moveTo')}
          value={doc.folderId ?? ''}
          onChange={(e) => void act('The document could not be moved', () => moveDoc(doc.id, e.target.value || null))}
          className="max-w-40 px-1 py-1 text-xs rounded bg-gray-700 text-gray-200 border border-gray-600"
        >
          <option value="">{t('documents:folders.none')}</option>
          {folderOptions(folders).map(({ folder, depth }) => (
            <option key={folder.id} value={folder.id}>
              {`${'\u2003'.repeat(depth)}${folder.name}`}
            </option>
          ))}
        </select>
        <button
          type="button"
          data-testid="shared-doc-history-toggle"
          aria-pressed={historyOpen}
          onClick={() => {
            setHistoryOpen((o) => !o);
            setVersion(null);
          }}
          className={historyOpen ? 'px-2 py-1 text-xs rounded bg-gray-500 text-white' : BUTTON}
        >
          {t('documents:history.open')}
        </button>
        {editable && others.length > 0 && (
          <button
            type="button"
            data-testid="shared-doc-leave"
            onClick={() => void act('The document could not be left', () => leave(doc.id))}
            className={BUTTON}
          >
            {t('documents:leave')}
          </button>
        )}
      </div>
      {shownError && (
        <div
          data-testid="shared-doc-error"
          className="m-3 mb-0 p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm"
        >
          {shownError}
        </div>
      )}
      {doc.status !== 'active' && (
        <p className="mx-3 mt-3 p-2 rounded bg-gray-800 text-xs text-gray-400">
          {doc.status === 'invited' ? t('documents:readOnlyInvitation') : t('documents:readOnlyLeft')}
        </p>
      )}
      <div className="flex flex-1 min-h-0">
        <div className="flex flex-col flex-1 min-w-0 min-h-0">
          {version ? (
            <>
              <div
                data-testid="shared-doc-version-banner"
                className="flex items-center gap-3 mx-3 mt-3 p-2 rounded bg-amber-900/30 border border-amber-800 text-xs text-amber-200"
              >
                <span className="flex-1">
                  {t('documents:history.viewing', { date: fmt.dateTime(version.createdAt) })}
                </span>
                <button type="button" onClick={() => setVersion(null)} className={BUTTON}>
                  {t('documents:history.backToCurrent')}
                </button>
              </div>
              {versionDoc &&
                (doc.kind === 'sheet' ? (
                  <SheetEditor key={version.id} doc={versionDoc} editable={false} />
                ) : (
                  <DocEditor key={version.id} doc={versionDoc} editable={false} onEditor={NOOP} />
                ))}
            </>
          ) : (
            ydoc &&
            (doc.kind === 'sheet' ? (
              <SheetEditor doc={ydoc} editable={editable} />
            ) : (
              <DocEditor doc={ydoc} editable={editable} onEditor={onEditor} />
            ))
          )}
        </div>
        {historyOpen && (
          <HistoryPanel
            accountId={doc.accountId}
            docId={doc.id}
            me={accountEmail.toLowerCase()}
            selectedId={version?.id ?? null}
            onSelect={setVersion}
            onClose={() => {
              setHistoryOpen(false);
              setVersion(null);
            }}
          />
        )}
      </div>
      {sharing && (
        <ShareDialog
          accountId={doc.accountId}
          title={doc.title}
          exclude={doc.participants}
          fromAddress={accountEmail}
          onShare={handleShare}
          onClose={() => setSharing(false)}
        />
      )}
    </section>
  );
}
