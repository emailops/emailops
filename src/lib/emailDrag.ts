// Drag-and-drop contract between email rows (drag sources) and the sidebar's
// Inbox / folder entries (drop targets). A custom MIME type keeps foreign
// drags (files, text selections) from ever looking like an email move.

export const EMAIL_DRAG_MIME = 'application/x-emailops-email';

export interface EmailDragPayload {
  emailId: string;
  accountId: string;
  /** The email's current mailbox — drops onto the same mailbox are no-ops. */
  mailbox: string;
}

export function writeEmailDragPayload(dataTransfer: DataTransfer, payload: EmailDragPayload): void {
  dataTransfer.setData(EMAIL_DRAG_MIME, JSON.stringify(payload));
  dataTransfer.effectAllowed = 'move';
}

/** Parse and validate a drop's payload; null for foreign or malformed drags
 *  (drop handlers must treat that as "not ours" and do nothing). */
export function readEmailDragPayload(dataTransfer: DataTransfer): EmailDragPayload | null {
  const raw = dataTransfer.getData(EMAIL_DRAG_MIME);
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return null;
    const candidate = parsed as Record<string, unknown>;
    if (
      typeof candidate.emailId !== 'string' ||
      candidate.emailId === '' ||
      typeof candidate.accountId !== 'string' ||
      candidate.accountId === '' ||
      typeof candidate.mailbox !== 'string'
    ) {
      return null;
    }
    return { emailId: candidate.emailId, accountId: candidate.accountId, mailbox: candidate.mailbox };
  } catch {
    return null;
  }
}

/** True when a dragover event carries an email payload (contents are not
 *  readable during dragover — only the type list is). */
export function isEmailDrag(dataTransfer: DataTransfer): boolean {
  return Array.from(dataTransfer.types).includes(EMAIL_DRAG_MIME);
}
