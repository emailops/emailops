import { type RefObject, useLayoutEffect, useState } from 'react';

/**
 * The width the folder sidebar, the email list and the reading area share:
 * from the start of the main area's row (the sidebar's left edge) to the end
 * of the main area, so whatever sits after it (the right dock) is left out.
 * Kept current through a ResizeObserver on the main area, which resizes when
 * the dock opens or closes and when the window does. Null until measured.
 *
 * `fitColumns` turns it into the widths the columns are drawn at.
 */
export function useColumnSpace(mainRef: RefObject<HTMLElement | null>): number | null {
  const [space, setSpace] = useState<number | null>(null);
  useLayoutEffect(() => {
    const main = mainRef.current;
    const row = main?.parentElement;
    if (!main || !row || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      setSpace(main.getBoundingClientRect().right - row.getBoundingClientRect().left);
    });
    observer.observe(main);
    return () => observer.disconnect();
  }, [mainRef]);
  return space;
}
