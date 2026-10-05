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

import { useEffect } from 'react';
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

/** Register the calling component as an overlay while `open` is true (and it
 *  is mounted). */
export function useOverlay(open = true): void {
  useEffect(() => {
    if (!open) return;
    useOverlayStore.getState().push();
    return () => useOverlayStore.getState().pop();
  }, [open]);
}
