import { describe, expect, it } from 'vitest';
import { computeMatchScrollTop, findScrollParent } from './matchScroll';

describe('computeMatchScrollTop', () => {
  it('positions the match a third of the way down the container', () => {
    const top = computeMatchScrollTop(
      { scrollTop: 100, rectTop: 50, clientHeight: 600 },
      /* frameRectTop */ 400,
      /* matchTop */ 250,
    );
    // 100 + (400 - 50) + 250 - 600/3 = 500
    expect(top).toBe(500);
  });

  it('never returns a negative scroll offset', () => {
    const top = computeMatchScrollTop({ scrollTop: 0, rectTop: 0, clientHeight: 900 }, 10, 20);
    expect(top).toBe(0);
  });
});

describe('findScrollParent', () => {
  it('returns the nearest ancestor with a scrollable overflow-y', () => {
    const outer = document.createElement('div');
    outer.style.overflowY = 'auto';
    const middle = document.createElement('div');
    const inner = document.createElement('div');
    middle.appendChild(inner);
    outer.appendChild(middle);
    document.body.appendChild(outer);
    try {
      expect(findScrollParent(inner)).toBe(outer);
    } finally {
      outer.remove();
    }
  });

  it('returns null when no scrollable ancestor exists', () => {
    const lone = document.createElement('div');
    document.body.appendChild(lone);
    try {
      expect(findScrollParent(lone)).toBeNull();
    } finally {
      lone.remove();
    }
  });
});
