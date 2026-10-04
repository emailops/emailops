import { useEffect, useRef, useState } from 'react';
import * as Y from 'yjs';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { bytesFromBase64, bytesToBase64 } from '@/lib/yjsBytes';
import { useLogStore } from '@/stores/logStore';
import { useSharedDocsStore } from '@/stores/sharedDocsStore';

/** Transaction origin of changes that came from the backend: never saved back. */
export const REMOTE_ORIGIN = 'shared-doc-remote';

/** Edits are batched this long before they are saved to the backend. */
const SAVE_DELAY_MS = 400;

interface UseSharedYDocArgs {
  accountId: string;
  docId: string;
  /** Mail the pending changes when the editor closes, instead of waiting for
   *  the pause. Only for a document the user shared or accepted. */
  mailOnClose: boolean;
}

/**
 * The `Y.Doc` an editor binds to, kept in sync with the backend's copy:
 * loads the stored state, saves local edits (batched), and pulls whatever the
 * backend merged from other people when the store reports a change.
 * `doc` is `null` until the state has loaded.
 */
export function useSharedYDoc({ accountId, docId, mailOnClose }: UseSharedYDocArgs) {
  const addLog = useLogStore((s) => s.addLog);
  const change = useSharedDocsStore((s) => s.changes[docId] ?? 0);
  const [doc, setDoc] = useState<Y.Doc | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mailOnCloseRef = useRef(mailOnClose);
  mailOnCloseRef.current = mailOnClose;

  useEffect(() => {
    let cancelled = false;
    const ydoc = new Y.Doc();
    let pending: Uint8Array[] = [];
    let timer: ReturnType<typeof setTimeout> | null = null;

    const report = (what: string, err: unknown) => {
      const msg = errorText(err);
      if (!cancelled) setError(msg);
      addLog('error', 'sync', `${what}: ${msg}`);
    };
    const save = async () => {
      timer = null;
      if (pending.length === 0) return;
      const update = Y.mergeUpdates(pending);
      pending = [];
      try {
        await api.applySharedDocUpdate(accountId, docId, bytesToBase64(update));
      } catch (err) {
        report('The document could not be saved', err);
      }
    };
    const onUpdate = (update: Uint8Array, origin: unknown) => {
      if (origin === REMOTE_ORIGIN) return;
      pending.push(update);
      if (!timer) timer = setTimeout(() => void save(), SAVE_DELAY_MS);
    };
    ydoc.on('update', onUpdate);

    setDoc(null);
    setError(null);
    api
      .getSharedDocState(accountId, docId)
      .then((state) => {
        if (cancelled) return;
        Y.applyUpdate(ydoc, bytesFromBase64(state), REMOTE_ORIGIN);
        setDoc(ydoc);
      })
      .catch((err) => report('The document could not be opened', err));

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      // `save` takes the pending edits synchronously, before the doc goes away.
      const saved = save();
      ydoc.off('update', onUpdate);
      ydoc.destroy();
      if (mailOnCloseRef.current) {
        void saved
          .then(() => api.flushSharedDoc(accountId, docId))
          .catch((err) => report('The changes could not be mailed, they will be retried', err));
      }
    };
  }, [accountId, docId, addLog]);

  // Another person's changes arrived: pull exactly what this editor lacks.
  useEffect(() => {
    if (!doc || change === 0) return;
    let cancelled = false;
    const stateVector = bytesToBase64(Y.encodeStateVector(doc));
    api
      .getSharedDocDiff(accountId, docId, stateVector)
      .then((diff) => {
        if (!cancelled) Y.applyUpdate(doc, bytesFromBase64(diff), REMOTE_ORIGIN);
      })
      .catch((err) => {
        const msg = errorText(err);
        if (!cancelled) setError(msg);
        addLog('error', 'sync', `New changes to the document could not be loaded: ${msg}`);
      });
    return () => {
      cancelled = true;
    };
  }, [doc, change, accountId, docId, addLog]);

  return { doc, error };
}
