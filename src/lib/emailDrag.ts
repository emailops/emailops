// Drag-and-drop contract between email rows (drag sources) and the sidebar's
// Inbox / folder entries (drop targets). A custom MIME type keeps foreign
// drags (files, text selections) from ever looking like an email move.

export const EMAIL_DRAG_MIME = 'application/x-emailops-email';

export interface EmailDragPayload {
  /** The emails that move: the dragged row alone, or every checked row when
   *  the dragged row is part of the multi-selection. Never empty. */
  emailIds: string[];
  accountId: string;
  /** The emails' current mailbox — drops onto the same mailbox are no-ops. */
  mailbox: string;
}

export function writeEmailDragPayload(dataTransfer: DataTransfer, payload: EmailDragPayload): void {
  dataTransfer.setData(EMAIL_DRAG_MIME, JSON.stringify(payload));
  dataTransfer.effectAllowed = 'move';
}

/**
 * Which emails a drag starting on `draggedId` carries. When the dragged row is
 * checked, the whole selection moves (the dragged row first); an unchecked
 * row moves alone, whatever else is checked — as Gmail and Thunderbird do.
 */
export function emailIdsToDrag(draggedId: string, selectedIds: ReadonlySet<string>): string[] {
  if (!selectedIds.has(draggedId)) return [draggedId];
  return [draggedId, ...[...selectedIds].filter((id) => id !== draggedId)];
}

/** What dropping a payload on a folder does. */
export type EmailDropPlan =
  | { kind: 'ignore' }
  | { kind: 'single'; emailId: string }
  | { kind: 'bulk'; emailIds: string[] };

/**
 * Decide what a drop on `targetMailbox` of `accountId` does. Pure.
 * Foreign/malformed drags, another account's emails and drops on the emails'
 * own mailbox are ignored; one email takes the single move, several take the
 * bulk move (`moveEmailsToMailbox`).
 */
export function planEmailDrop(
  payload: EmailDragPayload | null,
  accountId: string,
  targetMailbox: string,
): EmailDropPlan {
  if (!payload || payload.accountId !== accountId || payload.mailbox === targetMailbox) return { kind: 'ignore' };
  const [first, ...others] = payload.emailIds;
  return others.length === 0 ? { kind: 'single', emailId: first } : { kind: 'bulk', emailIds: payload.emailIds };
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
    const ids = candidate.emailIds;
    if (
      !Array.isArray(ids) ||
      ids.length === 0 ||
      !ids.every((id) => typeof id === 'string' && id !== '') ||
      typeof candidate.accountId !== 'string' ||
      candidate.accountId === '' ||
      typeof candidate.mailbox !== 'string'
    ) {
      return null;
    }
    return { emailIds: [...new Set(ids as string[])], accountId: candidate.accountId, mailbox: candidate.mailbox };
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

/**
 * The card is plain DOM built during `dragstart`, so Tailwind classes cannot
 * style it (they are not generated for runtime-only markup); it uses the same
 * palette values instead: gray-900 text, gray-500 sender, primary-600 accent
 * (`primary` is the custom sky scale in tailwind.config.js).
 */
const PALETTE = {
  gray900: '#111827',
  gray500: '#6b7280',
  primary600: '#0284c7',
  /** primary-600 at 50% for the border. */
  primary600Border: 'rgba(2, 132, 199, 0.5)',
} as const;

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
  // Compact, single line: sender in bold first (it tells emails apart at a
  // glance), subject after it in grey. Small enough not to hide the sidebar
  // folder the user is aiming at.
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
    border: `1px solid ${PALETTE.primary600Border}`,
    boxShadow: '0 2px 8px rgba(0, 0, 0, 0.15)',
    font: '10px/15px system-ui, sans-serif',
    color: PALETTE.gray900,
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    pointerEvents: 'none',
    zIndex: '2147483647',
  } satisfies Partial<CSSStyleDeclaration>);

  // The sender is short and is shown in full whenever possible; a long
  // subject gives way first.
  const sender = doc.createElement('span');
  sender.style.fontWeight = '600';
  sender.style.overflow = 'hidden';
  sender.style.textOverflow = 'ellipsis';
  sender.style.flexShrink = '0';
  sender.style.maxWidth = '60%';
  sender.textContent = clip(preview.sender);

  const subject = doc.createElement('span');
  subject.style.color = PALETTE.gray500;
  subject.style.overflow = 'hidden';
  subject.style.textOverflow = 'ellipsis';
  subject.style.flexShrink = '1';
  subject.textContent = clip(preview.subject) || '—';

  if (preview.count && preview.count > 1) {
    const badge = doc.createElement('span');
    badge.dataset.role = 'count';
    Object.assign(badge.style, {
      flexShrink: '0',
      minWidth: '16px',
      height: '16px',
      padding: '0 4px',
      borderRadius: '8px',
      background: PALETTE.primary600,
      color: '#fff',
      fontWeight: '700',
      fontSize: '10px',
      lineHeight: '16px',
      textAlign: 'center',
    } satisfies Partial<CSSStyleDeclaration>);
    badge.textContent = String(preview.count);
    card.append(badge);
  }

  card.append(sender, subject);
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
 * across the list (on its way to a sidebar folder), WebKitGTK shows the
 * "no drop" cursor over every row — read by users as "this can't be moved".
 * Calling `preventDefault()` marks the list as a drop zone and
 * `dropEffect = 'move'` keeps the normal drag cursor. `'none'` is not an
 * option: on WebKitGTK it brings the "no drop" cursor back over the whole
 * list. Dropping on the list still does nothing: {@link ignoreEmailDrop} is
 * its `drop` handler and swallows the drop. Foreign drags (files, text) are
 * left alone.
 */
export function allowEmailDragOver(e: { dataTransfer: DataTransfer | null; preventDefault: () => void }): void {
  if (!e.dataTransfer || !isEmailDrag(e.dataTransfer)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = 'move';
}

/** `drop` handler for the email list, wired next to {@link allowEmailDragOver}:
 *  an email dropped back on the list is a cancelled move. It is swallowed
 *  (`preventDefault`) so nothing else interprets the data; no email moves. */
export function ignoreEmailDrop(e: { dataTransfer: DataTransfer | null; preventDefault: () => void }): void {
  if (!e.dataTransfer || !isEmailDrag(e.dataTransfer)) return;
  e.preventDefault();
}

/** True when a dragover event carries an email payload (contents are not
 *  readable during dragover — only the type list is). */
export function isEmailDrag(dataTransfer: DataTransfer): boolean {
  return Array.from(dataTransfer.types).includes(EMAIL_DRAG_MIME);
}
