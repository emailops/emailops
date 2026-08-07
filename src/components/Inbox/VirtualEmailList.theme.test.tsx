// The virtual list positions each row with an inline style, and that style used
// to carry `backgroundColor: 'white'`. An inline colour beats every Tailwind
// class, including `dark:`, so in dark mode the whole message list stayed white
// under light text — every subject and sender unreadable. No class-level audit
// can catch this: there is no class to find.

import { describe, expect, it } from 'vitest';
import { ROW_WRAPPER_CLASS, rowWrapperStyle } from './VirtualEmailList';

describe('the virtual list row wrapper', () => {
  it('carries no inline background colour', () => {
    // Whatever else the inline style does (absolute positioning, transform),
    // the colour has to come from a class so the theme can reach it.
    const style = rowWrapperStyle(0);
    expect(style).not.toHaveProperty('backgroundColor');
    expect(JSON.stringify(style)).not.toMatch(/white|#fff/i);
  });

  it('still positions the row', () => {
    // The rest of the inline style is load-bearing for virtualisation.
    const style = rowWrapperStyle(240);
    expect(style.position).toBe('absolute');
    expect(style.transform).toBe('translateY(240px)');
  });

  it('is opaque in both themes', () => {
    // Opacity is the actual requirement: rows are absolutely positioned and a
    // row whose content outgrows its measured height would otherwise bleed
    // through the alpha-tinted unread row below it as doubled text.
    expect(ROW_WRAPPER_CLASS).toMatch(/(^|\s)bg-white(\s|$)/);
    expect(ROW_WRAPPER_CLASS).toMatch(/dark:bg-surface(\s|$)/);
    // A translucent utility here would re-open the bleed.
    expect(ROW_WRAPPER_CLASS).not.toMatch(/bg-\S+\/\d/);
  });
});
