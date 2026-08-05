// Tests for the live layout-mode hook backing the mobile shell.
//
// The stacking *policy* is the pure `shouldUseStackedLayout` in
// `lib/platform.ts` and is table-tested there. What is worth covering here is
// the glue the pure function cannot express: that the hook re-renders when the
// viewport changes, and that it detaches its listener on unmount.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useResponsiveLayout } from './useResponsiveLayout';

const { currentPlatform } = vi.hoisted(() => ({ currentPlatform: vi.fn(() => '') }));

vi.mock('@/lib/api', () => ({ currentPlatform }));

let container: HTMLDivElement;
let root: Root;

function Harness() {
  const { isStacked, isMobile } = useResponsiveLayout();
  return (
    <div>
      <span data-testid="stacked">{String(isStacked)}</span>
      <span data-testid="mobile">{String(isMobile)}</span>
    </div>
  );
}

function readFlag(name: 'stacked' | 'mobile'): string {
  return container.querySelector(`[data-testid="${name}"]`)?.textContent ?? '';
}

/** Resize the jsdom window and fire the event the hook subscribes to. */
function resizeTo(width: number) {
  act(() => {
    window.innerWidth = width;
    window.dispatchEvent(new Event('resize'));
  });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  window.innerWidth = 1440;
  currentPlatform.mockReturnValue('macos');
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe('useResponsiveLayout', () => {
  it('uses the split layout on a wide desktop window', () => {
    act(() => root.render(<Harness />));
    expect(readFlag('stacked')).toBe('false');
    expect(readFlag('mobile')).toBe('false');
  });

  it('stacks on iOS even when the viewport is wide', () => {
    currentPlatform.mockReturnValue('ios');
    window.innerWidth = 1366;
    act(() => root.render(<Harness />));
    expect(readFlag('stacked')).toBe('true');
    expect(readFlag('mobile')).toBe('true');
  });

  it('reacts to a desktop window being dragged narrow and back', () => {
    // The regression this hook exists to prevent: a layout decided once at
    // mount would leave a resized desktop window in the wrong mode.
    act(() => root.render(<Harness />));
    expect(readFlag('stacked')).toBe('false');

    resizeTo(500);
    expect(readFlag('stacked')).toBe('true');

    resizeTo(1200);
    expect(readFlag('stacked')).toBe('false');
  });

  it('detaches its resize listener on unmount', () => {
    const removeSpy = vi.spyOn(window, 'removeEventListener');
    act(() => root.render(<Harness />));
    act(() => root.unmount());

    expect(removeSpy).toHaveBeenCalledWith('resize', expect.any(Function));
    removeSpy.mockRestore();

    // Re-create so the shared afterEach unmount stays valid.
    root = createRoot(container);
  });
});
