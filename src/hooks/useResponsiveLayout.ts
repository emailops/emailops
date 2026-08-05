import { useEffect, useState } from 'react';
import { currentPlatform } from '@/lib/api';
import { isMobilePlatform, shouldUseStackedLayout } from '@/lib/platform';

export interface ResponsiveLayout {
  /**
   * Render panes as a single-column navigation stack (list → thread) instead of
   * side by side. True on any touch platform, and on a desktop window narrower
   * than the split-layout breakpoint.
   */
  isStacked: boolean;
  /** True on a touch-first mobile OS, regardless of viewport size. */
  isMobile: boolean;
}

/**
 * Live layout mode for the current platform and window size.
 *
 * The decision itself is the pure `shouldUseStackedLayout` in `lib/platform.ts`
 * and is table-tested there; this hook only supplies live inputs and
 * re-renders when the viewport changes. Keeping the policy out of the hook is
 * what makes the breakpoint behaviour testable without a DOM.
 *
 * `currentPlatform()` is read on every render rather than cached: it is a cheap
 * synchronous OS-plugin read, and caching it in state would add a render cycle
 * where the platform is unknown — during which a phone would briefly lay itself
 * out as a desktop.
 */
export function useResponsiveLayout(): ResponsiveLayout {
  const [viewportWidth, setViewportWidth] = useState<number>(() =>
    typeof window === 'undefined' ? Number.POSITIVE_INFINITY : window.innerWidth,
  );

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const handleResize = () => setViewportWidth(window.innerWidth);
    window.addEventListener('resize', handleResize);
    // Re-read once on mount: between the useState initializer and this effect
    // the window may already have been resized (or, on iOS, rotated).
    handleResize();
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  const platform = currentPlatform();

  return {
    isStacked: shouldUseStackedLayout(platform, viewportWidth),
    isMobile: isMobilePlatform(platform),
  };
}
