/** Narrow enough to sit beside the mail content, wide enough to read a reply. */
export const CHAT_PANEL_MIN_WIDTH = 280;
export const CHAT_PANEL_MAX_WIDTH = 720;
export const CHAT_PANEL_DEFAULT_WIDTH = 380;

/** Keep a dragged width inside the range where both panes stay usable. */
export function clampChatPanelWidth(width: number): number {
  return Math.round(Math.min(CHAT_PANEL_MAX_WIDTH, Math.max(CHAT_PANEL_MIN_WIDTH, width)));
}

/**
 * Parse the persisted `chat_panel_width` pref. Returns null for anything
 * unparseable so `usePersistedPref` falls back to the default; a stored value
 * that is merely out of range is clamped rather than discarded.
 */
export function parseChatPanelWidth(raw: string): number | null {
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || raw.trim() === '') return null;
  return clampChatPanelWidth(parsed);
}

/** What sits against the window's right edge. */
export type ChatDockMode = 'panel' | 'rail' | 'none';

/**
 * The docked chat panel when it is open; a rail to reopen it when collapsed,
 * so every view can bring the chat back; nothing while the full-page chat is
 * on screen (the same conversation would show twice) or AI is switched off.
 */
export function chatDockMode(s: { aiEnabled: boolean; panelOpen: boolean; fullChatView: boolean }): ChatDockMode {
  if (!s.aiEnabled || s.fullChatView) return 'none';
  return s.panelOpen ? 'panel' : 'rail';
}
