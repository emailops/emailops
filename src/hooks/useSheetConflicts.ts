import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type * as Y from 'yjs';
import { REMOTE_ORIGIN } from '@/lib/sharedDocOrigin';
import { type CellConflict, findConflicts } from '@/lib/sheetConflicts';
import { columnLabel } from '@/lib/sheetModel';
import { useLogStore } from '@/stores/logStore';
import { useToastStore } from '@/stores/toastStore';

/**
 * The concurrent cell edits of a sheet (`sheetConflicts.ts`), kept current as
 * the document changes. With `notify`, a change from someone else that drops a
 * value raises a toast and a log line, once per dropped value.
 */
export function useSheetConflicts(doc: Y.Doc | null, notify = false): CellConflict[] {
  const { t } = useTranslation(['documents']);
  const addLog = useLogStore((s) => s.addLog);
  const addToast = useToastStore((s) => s.addToast);
  const [conflicts, setConflicts] = useState<CellConflict[]>([]);
  const tRef = useRef(t);
  tRef.current = t;

  useEffect(() => {
    if (!doc) {
      setConflicts([]);
      return;
    }
    let known = new Set(findConflicts(doc).map((c) => c.id));
    const refresh = (_update: Uint8Array, origin: unknown) => {
      const found = findConflicts(doc);
      setConflicts(found);
      const fresh = found.filter((c) => !c.resolved && !known.has(c.id));
      known = new Set(found.map((c) => c.id));
      if (!notify || origin !== REMOTE_ORIGIN) return;
      for (const c of fresh) {
        const message = tRef.current('documents:conflicts.arrived', { cell: `${columnLabel(c.col)}${c.row + 1}` });
        addToast({ message });
        addLog('warn', 'sync', message);
      }
    };
    setConflicts(findConflicts(doc));
    doc.on('update', refresh);
    return () => doc.off('update', refresh);
  }, [doc, notify, addLog, addToast]);

  return conflicts;
}
