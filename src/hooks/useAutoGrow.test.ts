// Tests for the auto-grow textarea height planner used by ChatInput.
//
// The chat input was previously fixed at rows={2}; prompts longer than a
// couple of lines forced the user to work inside a tiny scrolling box. The
// planner computes the target height from the textarea's scrollHeight,
// clamped to a max, switching to internal scrolling only past the clamp.

import { describe, expect, it } from 'vitest';
import { planTextareaHeight } from './useAutoGrow';

describe('planTextareaHeight', () => {
  it('grows to fit content below the max (border included, no scrollbar)', () => {
    expect(planTextareaHeight({ scrollHeightPx: 96, borderPx: 2, maxPx: 220 })).toEqual({
      heightPx: 98,
      overflowY: 'hidden',
    });
  });

  it('clamps at the max and enables internal scrolling', () => {
    expect(planTextareaHeight({ scrollHeightPx: 500, borderPx: 2, maxPx: 220 })).toEqual({
      heightPx: 220,
      overflowY: 'auto',
    });
  });

  it('exactly at the max still hides the scrollbar (nothing is cut off)', () => {
    expect(planTextareaHeight({ scrollHeightPx: 218, borderPx: 2, maxPx: 220 })).toEqual({
      heightPx: 220,
      overflowY: 'hidden',
    });
  });
});
