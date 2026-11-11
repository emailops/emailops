import { describe, expect, it } from 'vitest';
import {
  CHAT_PANEL_DEFAULT_WIDTH,
  CHAT_PANEL_MAX_WIDTH,
  CHAT_PANEL_MIN_WIDTH,
  clampChatPanelWidth,
  parseChatPanelWidth,
} from '@/lib/chatPanelLayout';

describe('clampChatPanelWidth', () => {
  it('passes through a width inside the allowed range', () => {
    expect(clampChatPanelWidth(420)).toBe(420);
  });

  it('clamps below the minimum', () => {
    // Dragging the handle past the right edge must not collapse the panel to
    // an unusable sliver — the user would have no target left to drag back.
    expect(clampChatPanelWidth(10)).toBe(CHAT_PANEL_MIN_WIDTH);
    expect(clampChatPanelWidth(-500)).toBe(CHAT_PANEL_MIN_WIDTH);
  });

  it('clamps above the maximum', () => {
    // Likewise the mail content must never be squeezed out entirely.
    expect(clampChatPanelWidth(5000)).toBe(CHAT_PANEL_MAX_WIDTH);
  });

  it('rounds fractional drag positions to whole pixels', () => {
    expect(clampChatPanelWidth(420.6)).toBe(421);
  });
});

describe('parseChatPanelWidth', () => {
  it('reads a stored numeric width', () => {
    expect(parseChatPanelWidth('440')).toBe(440);
  });

  it('clamps a stored width that is out of range', () => {
    // A pref written by an older/newer build, or hand-edited, must not be able
    // to render the panel unusable.
    expect(parseChatPanelWidth('99999')).toBe(CHAT_PANEL_MAX_WIDTH);
    expect(parseChatPanelWidth('1')).toBe(CHAT_PANEL_MIN_WIDTH);
  });

  it('rejects junk so the default applies', () => {
    expect(parseChatPanelWidth('wide')).toBeNull();
    expect(parseChatPanelWidth('')).toBeNull();
    expect(parseChatPanelWidth('NaN')).toBeNull();
  });

  it('has a default inside its own bounds', () => {
    expect(CHAT_PANEL_DEFAULT_WIDTH).toBeGreaterThanOrEqual(CHAT_PANEL_MIN_WIDTH);
    expect(CHAT_PANEL_DEFAULT_WIDTH).toBeLessThanOrEqual(CHAT_PANEL_MAX_WIDTH);
  });
});
