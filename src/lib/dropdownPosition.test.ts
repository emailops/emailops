import { describe, expect, it } from 'vitest';
import { computeDropdownTop } from './dropdownPosition';

describe('computeDropdownTop', () => {
  it('positions the menu below the anchor when it fits in the viewport', () => {
    const top = computeDropdownTop({
      anchorTop: 100,
      anchorBottom: 130,
      menuHeight: 300,
      viewportHeight: 800,
    });
    expect(top).toBe(134); // anchorBottom + 4px gap
  });

  it('flips the menu above the anchor when it would overflow the bottom edge', () => {
    const top = computeDropdownTop({
      anchorTop: 700,
      anchorBottom: 730,
      menuHeight: 300,
      viewportHeight: 800,
    });
    expect(top).toBe(396); // anchorTop - 4px gap - menuHeight
  });

  it('clamps to the bottom edge when the menu fits neither below nor above', () => {
    const top = computeDropdownTop({
      anchorTop: 200,
      anchorBottom: 230,
      menuHeight: 380,
      viewportHeight: 400,
    });
    expect(top).toBe(16); // viewportHeight - 4px inset - menuHeight
  });

  it('never returns a top above the viewport, even for menus taller than it', () => {
    const top = computeDropdownTop({
      anchorTop: 150,
      anchorBottom: 180,
      menuHeight: 380,
      viewportHeight: 300,
    });
    expect(top).toBe(4); // pinned to the top inset
  });

  it('honors a custom margin', () => {
    const top = computeDropdownTop({
      anchorTop: 100,
      anchorBottom: 130,
      menuHeight: 200,
      viewportHeight: 800,
      margin: 8,
    });
    expect(top).toBe(138);
  });
});
