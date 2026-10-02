import { type RefObject, useLayoutEffect } from 'react';

/**
 * Publish an element's rendered height as a CSS variable on the document
 * root, kept current through a ResizeObserver, and removed on unmount.
 *
 * For fixed-position overlays that must keep clear of an in-flow bar whose
 * height changes (the Output panel collapses and expands): a CSS variable
 * rather than a prop, as `ChatPanelDock` does with `--chat-dock-width`, so the
 * overlay does not have to know the bar exists.
 */
export function usePublishedHeight(ref: RefObject<HTMLElement | null>, variable: `--${string}`): void {
  useLayoutEffect(() => {
    const el = ref.current;
    const style = document.documentElement.style;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.borderBoxSize?.[0];
      const height = box ? box.blockSize : el.getBoundingClientRect().height;
      style.setProperty(variable, `${Math.round(height)}px`);
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
      style.removeProperty(variable);
    };
  }, [ref, variable]);
}
