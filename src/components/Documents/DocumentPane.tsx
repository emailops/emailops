import type { Editor } from '@tiptap/react';
import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSharedYDoc } from '@/hooks/useSharedYDoc';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { gridToHtml, readGrid } from '@/lib/sheetModel';
import { useLogStore } from '@/stores/logStore';
import { useSharedDocsStore } from '@/stores/sharedDocsStore';
import type { SharedDoc } from '@/types';
import { DocEditor } from './DocEditor';
import { ShareDialog } from './ShareDialog';
import { SheetEditor } from './SheetEditor';

const BUTTON = 'px-2 py-1 text-xs rounded bg-gray-700 hover:bg-gray-600 text-gray-200 disabled:opacity-40';

interface DocumentPaneProps {
  doc: SharedDoc;
  /** The address of the account the document belongs to. */
  accountEmail: string;
}

/** One open document: its header (sharing, status) and its editor. */
export function DocumentPane({ doc, accountEmail }: DocumentPaneProps) {
  const { t } = useTranslation(['documents']);
  const addLog = useLogStore((s) => s.addLog);
  const share = useSharedDocsStore((s) => s.share);
  const accept = useSharedDocsStore((s) => s.accept);
  const leave = useSharedDocsStore((s) => s.leave);
  const reload = useSharedDocsStore((s) => s.reload);
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
      doc.kind === 'sheet' ? (ydoc ? gridToHtml(readGrid(ydoc)) : null) : (editorRef.current?.getHTML() ?? null);
    await share(doc.id, recipients, snapshot);
    addLog('success', 'sync', `Shared "${doc.title}" with ${recipients.join(', ')}`);
  };

  const handleSendNow = () =>
    act('The changes could not be sent', async () => {
      const sent = await api.flushSharedDoc(doc.accountId, doc.id);
      if (sent) addLog('success', 'sync', `Changes to "${doc.title}" sent`);
      await reload();
    });

  const shownError = error ?? syncError;

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
        {editable && consented && doc.dirtySince !== null && (
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
      {ydoc &&
        (doc.kind === 'sheet' ? (
          <SheetEditor doc={ydoc} editable={editable} />
        ) : (
          <DocEditor doc={ydoc} editable={editable} onEditor={onEditor} />
        ))}
      {sharing && (
        <ShareDialog
          title={doc.title}
          fromAddress={accountEmail}
          onShare={handleShare}
          onClose={() => setSharing(false)}
        />
      )}
    </section>
  );
}
