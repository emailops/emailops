import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { DraftAttachmentInput, EmailAttachment } from '@/lib/api';
import * as api from '@/lib/api';
import { planIntake } from '@/lib/attachmentIntake';
import { errorText } from '@/lib/errors';

/** The bytes of `file` as base64, without the data-URL prefix. */
function readBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
    reader.onerror = () => reject(reader.error ?? new Error(`Could not read ${file.name}`));
    reader.readAsDataURL(file);
  });
}

/**
 * The files a composer attaches, each stored for its draft as it arrives
 * (#274): the draft names them, so closing the composer or the app does not
 * lose them, and sending reads them from the draft.
 */
export function useDraftFiles(accountId: string, initial: DraftAttachmentInput[] = []) {
  const { t } = useTranslation(['compose']);
  const [files, setFiles] = useState<DraftAttachmentInput[]>(initial);
  // Whether a save names the files. A composer that opened with none and was
  // given none leaves them out, so a text-only save never clears the files of
  // a draft it did not load; once it holds some, removing the last one clears
  // them.
  const [managed, setManaged] = useState(initial.length > 0);
  const [pending, setPending] = useState<{ name: string; size: number }[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Read when a store finishes, so a file lands in the account the composer
  // has by then rather than the one it had when the file was picked.
  const accountRef = useRef(accountId);
  accountRef.current = accountId;

  const store = useCallback(
    async (name: string, size: number, read: () => Promise<EmailAttachment>) => {
      const entry = { name, size };
      setPending((prev) => [...prev, entry]);
      try {
        const stored = await api.storeDraftFile(accountRef.current, await read());
        setFiles((prev) => [...prev, stored]);
        setManaged(true);
      } catch (err) {
        setError(t('compose:intake.failed', { name, error: errorText(err) }));
      } finally {
        setPending((prev) => prev.filter((p) => p !== entry));
      }
    },
    [t],
  );

  /** Check, read and store files the user picked, dropped or pasted. */
  const addFiles = (list: FileList | File[] | null) => {
    if (!list) return;
    const offered = Array.from(list);
    const pendingBytes = pending.reduce((sum, p) => sum + p.size, 0);
    const plan = planIntake(files.length + pending.length, pendingBytes, offered);
    setError(
      plan.refused.length > 0
        ? plan.refused.map((r) => t(`compose:intake.${r.reason}`, { name: r.name })).join(' ')
        : null,
    );
    for (const index of plan.accepted) {
      const file = offered[index];
      void store(file.name, file.size, async () => ({
        filename: file.name,
        mimeType: file.type || 'application/octet-stream',
        data: await readBase64(file),
      }));
    }
  };

  /** Store attachments already read, such as a forward's original files. */
  const addAttachments = (attachments: EmailAttachment[]) => {
    for (const attachment of attachments) {
      void store(attachment.filename, attachment.data.length, async () => attachment);
    }
  };

  const remove = (index: number) => setFiles((prev) => prev.filter((_, i) => i !== index));

  return {
    files,
    /** What a draft save takes as its attachments: `undefined` leaves the
     *  draft's files as they are. */
    forSave: managed ? files : undefined,
    isStoring: pending.length > 0,
    storingNames: pending.map((p) => p.name),
    error,
    addFiles,
    addAttachments,
    remove,
  };
}
