// The overlay registry: how the app-wide key handler knows something sits on
// top of the conversation.
//
// Every dialog, drawer, lightbox, menu and popover calls `useOverlay(open)`
// while it is on screen. While the count is above zero no conversation
// shortcut runs (`useGlobalShortcuts`): the overlay owns the keyboard, and a
// Delete meant for a dialog field or pressed by habit must never trash the
// thread behind it. An explicit registration replaces guessing from CSS
// classes, which silently missed portalled menus and absolutely positioned
// popovers (docs/DECISIONS.md, "Overlays register themselves").
//
// The registry also knows the order they opened in: Escape belongs to the top
// overlay alone, so closing a confirmation does not close the dialog under it.

import { useCallback, useEffect, useId } from 'react';
import { create } from 'zustand';

interface OverlayStore {
  /** Overlays currently on screen. */
  count: number;
  push: () => void;
  pop: () => void;
}

export const useOverlayStore = create<OverlayStore>((set) => ({
  count: 0,
  push: () => set((s) => ({ count: s.count + 1 })),
  pop: () => set((s) => ({ count: Math.max(0, s.count - 1) })),
}));

/** Pure selector: an overlay is on screen. */
export const anyOverlayOpen = (state: Pick<OverlayStore, 'count'>): boolean => state.count > 0;

/** The overlays on screen, oldest first. */
const stack: string[] = [];

/** Register the calling component as an overlay while `open` is true (and it
 *  is mounted). Returns whether it is the top overlay right now. */
export function useOverlay(open = true): () => boolean {
  const id = useId();
  useEffect(() => {
    if (!open) return;
    stack.push(id);
    useOverlayStore.getState().push();
    return () => {
      stack.splice(stack.indexOf(id), 1);
      useOverlayStore.getState().pop();
    };
  }, [open, id]);
  return useCallback(() => stack[stack.length - 1] === id, [id]);
}
