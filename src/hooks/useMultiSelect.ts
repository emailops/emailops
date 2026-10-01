import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import * as api from '@/lib/api';
import {
  applyClick,
  clickModifiers,
  EMPTY_SELECTION,
  isAllSelected,
  isMultiSelecting,
  type MultiSelection,
  pruneSelection,
  selectAll,
  toggleSelectAll,
} from '@/lib/multiSelect';
import type { Email } from '@/types';

function isTyping(target: EventTarget | null): boolean {
  if (!target || typeof (target as Element).closest !== 'function') return false;
  const el = target as HTMLElement;
  return (
    el.isContentEditable ||
    !!el.closest(
      'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="textbox"], [role="dialog"]',
    )
  );
}

/**
 * Multi-selection over the visible email list (see `lib/multiSelect.ts` for
 * the click rules). `onOpen` is called for a plain click — the normal "open
 * this email" path — so single-email behaviour is unchanged.
 *
 * While two or more emails are selected: Escape clears the selection, and
 * Delete/Backspace calls `onDeleteSelection` (outside text fields). Ctrl/⌘+A
 * selects every visible email when focus is in the list.
 */
export function useMultiSelect(
  visible: Email[],
  openId: string | null,
  onOpen: (email: Email) => void,
  onDeleteSelection: () => void,
  listRef: React.RefObject<HTMLElement | null>,
) {
  const [selection, setSelection] = useState<MultiSelection>(EMPTY_SELECTION);
  const order = useMemo(() => visible.map((e) => e.id), [visible]);
  // ⌘ on macOS, Ctrl elsewhere.
  const isMac = useMemo(() => api.currentPlatform() === 'macos', []);

  // Emails deleted, moved or filtered out leave the selection.
  useEffect(() => {
    setSelection((s) => pruneSelection(s, order));
  }, [order]);

  const handleRowClick = useCallback(
    (email: Email, e?: React.MouseEvent | React.KeyboardEvent) => {
      const mods = e && 'shiftKey' in e ? clickModifiers(e, isMac) : { toggle: false, range: false };
      const outcome = applyClick(selection, email.id, mods, order, openId);
      setSelection(outcome.selection);
      if (outcome.kind === 'open') onOpen(email);
    },
    [selection, order, openId, isMac, onOpen],
  );

  const clear = useCallback(() => setSelection(EMPTY_SELECTION), []);
  // ✓ in the bar: select every visible email, or deselect them when they all
  // are. The bar stays open either way; only ✕ / Escape leave the mode.
  const toggleAllVisible = useCallback(() => setSelection((s) => toggleSelectAll(s, order)), [order]);
  const allSelected = isAllSelected(selection, order);

  const active = isMultiSelecting(selection);
  const selectedIds = useMemo(() => new Set(active ? selection.ids : []), [active, selection.ids]);
  const selectedEmails = useMemo(() => {
    if (!active) return [];
    const byId = new Map(visible.map((e) => [e.id, e]));
    return selection.ids.map((id) => byId.get(id)).filter((e): e is Email => !!e);
  }, [active, selection.ids, visible]);

  const deleteRef = useRef(onDeleteSelection);
  deleteRef.current = onDeleteSelection;
  const selectionRef = useRef(selection);
  selectionRef.current = selection;

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || isTyping(e.target)) return;
      const toggle = isMac ? e.metaKey : e.ctrlKey;
      if (toggle && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'a') {
        // Only inside the list: Ctrl+A elsewhere keeps its usual meaning.
        if (listRef.current && e.target instanceof Node && listRef.current.contains(e.target)) {
          e.preventDefault();
          setSelection(selectAll(order));
        }
        return;
      }
      if (!active) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        setSelection(EMPTY_SELECTION);
        return;
      }
      if ((e.key === 'Delete' || e.key === 'Backspace') && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        if (e.repeat) return;
        // Takes precedence over the reading pane's single-thread Delete —
        // also when nothing is ticked, so Delete never removes the open email
        // by surprise while the user is in selection mode.
        e.preventDefault();
        e.stopImmediatePropagation();
        if (selectionRef.current.ids.length > 0) deleteRef.current();
      }
    };
    // Capture phase, so a multi-selection is handled before the open thread's own Delete.
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [active, isMac, order, listRef]);

  return { selectedIds, selectedEmails, isActive: active, allSelected, handleRowClick, clear, toggleAllVisible };
}
