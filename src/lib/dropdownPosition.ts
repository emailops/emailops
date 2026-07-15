export interface DropdownTopInput {
  /** Anchor (trigger button) rect edges in viewport coordinates. */
  anchorTop: number;
  anchorBottom: number;
  /** Measured height of the dropdown menu. */
  menuHeight: number;
  viewportHeight: number;
  /** Gap between anchor and menu, also used as the minimum viewport inset. */
  margin?: number;
}

/**
 * Vertical position for a fixed-position dropdown so it never renders off
 * screen: below the anchor when it fits, flipped above when it doesn't,
 * clamped inside the viewport as a last resort.
 */
export function computeDropdownTop({
  anchorTop,
  anchorBottom,
  menuHeight,
  viewportHeight,
  margin = 4,
}: DropdownTopInput): number {
  const below = anchorBottom + margin;
  if (below + menuHeight <= viewportHeight - margin) {
    return below;
  }
  const above = anchorTop - margin - menuHeight;
  if (above >= margin) {
    return above;
  }
  return Math.max(margin, viewportHeight - margin - menuHeight);
}
