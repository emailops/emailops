/**
 * Keyboard shortcut to move the open email thread to the Trash: the Delete
 * key (and Backspace on its own, the Mac "delete" key).
 *
 * Pure so the "when does it fire?" rules are unit-tested without React; see
 * `deleteShortcut.test.ts`. It must never fire while the user is typing —
 * in a text field, the rich-text editor, a reply, or behind an open dialog —
 * since Delete/Backspace are ordinary editing keys there.
 */

export interface DeleteShortcutContext {
  /** Something is open in the reading pane. */
  hasThread: boolean;
  /** The inline reply/forward composer is open. */
  isReplyOpen: boolean;
  /** A delete is already running. */
  isDeleting: boolean;
}

/** The parts of a KeyboardEvent the rule needs (easy to fake in tests). */
export interface KeyLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  repeat?: boolean;
  defaultPrevented?: boolean;
  target: EventTarget | null;
}

function isEditable(target: EventTarget | null): boolean {
  if (!target || typeof (target as Element).closest !== 'function') return false;
  const el = target as HTMLElement;
  if (el.isContentEditable) return true;
  // Any field, the rich-text editor, or something inside an open dialog.
  return !!el.closest(
    'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="textbox"], [role="dialog"], [role="menu"], [role="listbox"]',
  );
}

/** Whether this key press should move the open thread to the Trash. */
export function isDeleteShortcut(e: KeyLike, ctx: DeleteShortcutContext): boolean {
  if (e.key !== 'Delete' && e.key !== 'Backspace') return false;
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
  if (e.repeat || e.defaultPrevented) return false;
  if (!ctx.hasThread || ctx.isReplyOpen || ctx.isDeleting) return false;
  return !isEditable(e.target);
}
