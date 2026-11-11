/** What the right dock can host. One at a time: chat or the calendar agenda. */
export type RightDockContent = 'chat' | 'calendar';

export type RightDockMode = { kind: 'panel'; content: RightDockContent } | { kind: 'rail' } | { kind: 'none' };

export interface RightDockState {
  /** Master AI switch — chat is an AI surface. */
  aiEnabled: boolean;
  /** Some account can show a calendar (API or feeds, or an IMAP account that
   *  could subscribe to one). */
  calendarAvailable: boolean;
  /** Pref `chat_panel_open` (kept under its historical name). */
  panelOpen: boolean;
  /** Pref `right_dock_content`: what the user last had docked. */
  content: RightDockContent;
  /** The full-page chat is on screen — the same conversation must not show twice. */
  fullChatView: boolean;
  /** The full calendar view is on screen — no agenda beside it. */
  fullCalendarView: boolean;
}

/** Contents that can be docked right now, in rail order (chat first). */
export function railButtons(s: RightDockState): RightDockContent[] {
  const out: RightDockContent[] = [];
  if (s.aiEnabled && !s.fullChatView) out.push('chat');
  if (s.calendarAvailable && !s.fullCalendarView) out.push('calendar');
  return out;
}

/**
 * The docked panel when it is open (the remembered content, or the other one
 * when that content is currently a full-page view); a rail to reopen it when
 * collapsed; nothing when neither chat nor calendar can be docked.
 */
export function rightDockMode(s: RightDockState): RightDockMode {
  const available = railButtons(s);
  if (available.length === 0) return { kind: 'none' };
  if (!s.panelOpen) return { kind: 'rail' };
  return { kind: 'panel', content: available.includes(s.content) ? s.content : available[0] };
}

/** Parse the persisted `right_dock_content` pref; null lets the default apply. */
export function parseRightDockContent(raw: string): RightDockContent | null {
  return raw === 'chat' || raw === 'calendar' ? raw : null;
}
