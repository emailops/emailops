// Drag-and-drop contract between email rows (drag sources) and the sidebar's
// Inbox / folder entries (drop targets). A custom MIME type keeps foreign
// drags (files, text selections) from ever looking like an email move.

export const EMAIL_DRAG_MIME = 'application/x-emailops-email';

export interface EmailDragItem {
  emailId: string;
  accountId: string;
  /** The email's current mailbox — drops onto the same mailbox are no-ops. */
  mailbox: string;
}

export interface EmailDragPayload extends EmailDragItem {
  /** The other emails of a multi-selection dragged together with this one
   *  (the row under the pointer). Absent for a single-email drag. */
  extra?: EmailDragItem[];
}

/** Every email a drop moves: the dragged row first, then the rest of the selection. */
export function dragPayloadItems(payload: EmailDragPayload): EmailDragItem[] {
  const first: EmailDragItem = { emailId: payload.emailId, accountId: payload.accountId, mailbox: payload.mailbox };
  return [first, ...(payload.extra ?? [])];
}

function parseItem(value: unknown): EmailDragItem | null {
  if (typeof value !== 'object' || value === null) return null;
  const c = value as Record<string, unknown>;
  if (
    typeof c.emailId !== 'string' ||
    c.emailId === '' ||
    typeof c.accountId !== 'string' ||
    c.accountId === '' ||
    typeof c.mailbox !== 'string'
  ) {
    return null;
  }
  return { emailId: c.emailId, accountId: c.accountId, mailbox: c.mailbox };
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
    const first = parseItem(parsed);
    if (!first) return null;
    const rawExtra = (parsed as Record<string, unknown>).extra;
    if (rawExtra === undefined) return first;
    // A malformed selection is rejected whole rather than half-moved.
    if (!Array.isArray(rawExtra)) return null;
    const extra = rawExtra.map(parseItem);
    if (extra.some((item) => item === null)) return null;
    return { ...first, extra: extra as EmailDragItem[] };
  } catch {
    return null;
  }
}

/** What the drag preview card shows. */
export interface EmailDragPreview {
  sender: string;
  subject: string;
  /** How many emails move together (more than 1 shows a count badge). */
  count?: number;
}

const PREVIEW_ID = 'emailops-drag-preview';
const PREVIEW_MAX_CHARS = 40;

function clip(text: string): string {
  const t = text.replace(/\s+/g, ' ').trim();
  return t.length > PREVIEW_MAX_CHARS ? `${t.slice(0, PREVIEW_MAX_CHARS - 1)}…` : t;
}

/**
 * Build the semi-transparent card that follows the pointer while an email is
 * dragged (like Thunderbird's). Without it, WebKitGTK shows nothing under the
 * cursor for a dragged list row — only the "no drop" cursor — so the user
 * cannot tell anything is being moved.
 *
 * Plain DOM, not React: `setDragImage` snapshots the element synchronously
 * during `dragstart`, and WebKit only renders it while it is attached to the
 * document. It is parked off-screen and removed on the next tick. Text is set
 * with `textContent`, so sender/subject can never inject markup.
 */
export function buildDragPreview(doc: Document, preview: EmailDragPreview): HTMLElement {
  doc.getElementById(PREVIEW_ID)?.remove();
  // Compact, single line: subject in bold, sender after it in grey. Small
  // enough not to hide the sidebar folder the user is aiming at.
  const card = doc.createElement('div');
  card.id = PREVIEW_ID;
  card.setAttribute('aria-hidden', 'true');
  Object.assign(card.style, {
    position: 'fixed',
    top: '-1000px',
    left: '-1000px',
    display: 'flex',
    alignItems: 'baseline',
    gap: '6px',
    maxWidth: '200px',
    padding: '4px 8px',
    borderRadius: '6px',
    background: 'rgba(255, 255, 255, 0.85)',
    border: '1px solid rgba(59, 130, 246, 0.5)',
    boxShadow: '0 2px 8px rgba(0, 0, 0, 0.15)',
    font: '11px/16px system-ui, sans-serif',
    color: '#111827',
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    pointerEvents: 'none',
    zIndex: '2147483647',
  } satisfies Partial<CSSStyleDeclaration>);

  const subject = doc.createElement('span');
  subject.style.fontWeight = '600';
  subject.style.overflow = 'hidden';
  subject.style.textOverflow = 'ellipsis';
  subject.style.flexShrink = '1';
  subject.textContent = clip(preview.subject) || '—';

  const sender = doc.createElement('span');
  sender.style.color = '#6b7280';
  sender.style.overflow = 'hidden';
  sender.style.textOverflow = 'ellipsis';
  sender.style.flexShrink = '2';
  sender.textContent = clip(preview.sender);

  if (preview.count && preview.count > 1) {
    const badge = doc.createElement('span');
    badge.dataset.role = 'count';
    Object.assign(badge.style, {
      flexShrink: '0',
      minWidth: '16px',
      height: '16px',
      padding: '0 4px',
      borderRadius: '8px',
      background: '#2563eb',
      color: '#fff',
      fontWeight: '700',
      fontSize: '10px',
      lineHeight: '16px',
      textAlign: 'center',
    } satisfies Partial<CSSStyleDeclaration>);
    badge.textContent = String(preview.count);
    card.append(badge);
  }

  card.append(subject, sender);
  doc.body.append(card);
  return card;
}

/**
 * Attach the preview card to a drag: call from `dragstart`. Safe where
 * `setDragImage` is missing (older engines keep their default image).
 */
export function setEmailDragImage(event: { dataTransfer: DataTransfer | null }, preview: EmailDragPreview): void {
  const dt = event.dataTransfer;
  if (!dt || typeof dt.setDragImage !== 'function' || typeof document === 'undefined') return;
  const card = buildDragPreview(document, preview);
  // Grab point just inside the card's top-left corner, so the card trails the
  // pointer instead of covering what is under it.
  dt.setDragImage(card, 8, 8);
  // The snapshot is taken during dragstart; the element is no longer needed.
  window.setTimeout(() => card.remove(), 0);
}

/**
 * `dragover` handler for the email list itself. While an email is dragged
 * across the list (on its way to a sidebar folder), the system would show the
 * "no drop" cursor over every row — read by users as "this can't be moved".
 * Marking the list as a neutral zone (`dropEffect = 'none'` is *not* used)
 * keeps the normal drag cursor; nothing happens if the email is dropped here
 * because the list has no `drop` handler. Foreign drags are left alone.
 */
export function allowEmailDragOver(e: { dataTransfer: DataTransfer | null; preventDefault: () => void }): void {
  if (!e.dataTransfer || !isEmailDrag(e.dataTransfer)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = 'move';
}

/** `drop` handler for the email list: an email dropped back on the list is a
 *  cancelled move — swallow it so nothing else interprets the data. */
export function ignoreEmailDrop(e: { dataTransfer: DataTransfer | null; preventDefault: () => void }): void {
  if (!e.dataTransfer || !isEmailDrag(e.dataTransfer)) return;
  e.preventDefault();
}

/** True when a dragover event carries an email payload (contents are not
 *  readable during dragover — only the type list is). */
export function isEmailDrag(dataTransfer: DataTransfer): boolean {
  return Array.from(dataTransfer.types).includes(EMAIL_DRAG_MIME);
}
