// The overlay registry and the rule that keeps it complete.

import { describe, expect, it } from 'vitest';
import { anyOverlayOpen, useOverlayStore } from './overlayStore';

const SOURCES = import.meta.glob<string>(['/src/**/*.tsx', '!/src/**/*.test.*'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

describe('overlay registry', () => {
  it('counts pushes and pops and never goes below zero', () => {
    useOverlayStore.setState({ count: 0 });
    const { push, pop } = useOverlayStore.getState();
    push();
    push();
    expect(anyOverlayOpen(useOverlayStore.getState())).toBe(true);
    pop();
    pop();
    pop();
    expect(useOverlayStore.getState().count).toBe(0);
    expect(anyOverlayOpen(useOverlayStore.getState())).toBe(false);
  });

  // A full-screen layer the registry does not know about would let a
  // conversation shortcut act behind it. Every file drawing one registers.
  it('every full-screen overlay in the app registers itself', () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(50);
    const missing = Object.entries(SOURCES)
      .filter(([, text]) => /className=[^>]*\bfixed inset-0\b/.test(text) && !text.includes('useOverlay('))
      .map(([path]) => path);
    expect(missing).toEqual([]);
  });
});
