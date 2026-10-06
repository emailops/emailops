import { useEffect, useState } from 'react';
import * as Y from 'yjs';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { bytesFromBase64 } from '@/lib/yjsBytes';

/** A detached, read-only `Y.Doc` holding one past version of a document. */
export function useVersionDoc(accountId: string, docId: string, versionId: number | null) {
  const [doc, setDoc] = useState<Y.Doc | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDoc(null);
    setError(null);
    if (versionId === null) return;
    let cancelled = false;
    const ydoc = new Y.Doc();
    api
      .getSharedDocVersion(accountId, docId, versionId)
      .then((state) => {
        if (cancelled) return;
        Y.applyUpdate(ydoc, bytesFromBase64(state));
        setDoc(ydoc);
      })
      .catch((err) => {
        if (!cancelled) setError(errorText(err));
      });
    return () => {
      cancelled = true;
      ydoc.destroy();
    };
  }, [accountId, docId, versionId]);

  return { doc, error };
}
