import { describe, expect, it } from 'vitest';
import { parseRightDockContent, railButtons, rightDockMode } from '@/lib/rightDockLayout';

const base = {
  aiEnabled: true,
  calendarAvailable: true,
  panelOpen: true,
  content: 'chat' as const,
  fullChatView: false,
  fullCalendarView: false,
};

describe('rightDockMode', () => {
  it('docks the requested content when it is open and available', () => {
    expect(rightDockMode(base)).toEqual({ kind: 'panel', content: 'chat' });
    expect(rightDockMode({ ...base, content: 'calendar' })).toEqual({ kind: 'panel', content: 'calendar' });
  });

  it('leaves a rail when the panel is collapsed', () => {
    expect(rightDockMode({ ...base, panelOpen: false })).toEqual({ kind: 'rail' });
  });

  it('shows nothing when neither chat nor calendar can be docked', () => {
    // AI off removes chat; no account with a calendar removes the calendar.
    expect(rightDockMode({ ...base, aiEnabled: false, calendarAvailable: false })).toEqual({ kind: 'none' });
    // The full-page views take their own content off the dock.
    expect(rightDockMode({ ...base, fullChatView: true, calendarAvailable: false })).toEqual({ kind: 'none' });
  });

  it('keeps the calendar available with AI switched off', () => {
    expect(rightDockMode({ ...base, aiEnabled: false, content: 'calendar' })).toEqual({
      kind: 'panel',
      content: 'calendar',
    });
    expect(rightDockMode({ ...base, aiEnabled: false, panelOpen: false })).toEqual({ kind: 'rail' });
  });

  it('falls back to the other content when the requested one is unavailable', () => {
    // Full-page calendar open while the dock remembers "calendar": show chat.
    expect(rightDockMode({ ...base, content: 'calendar', fullCalendarView: true })).toEqual({
      kind: 'panel',
      content: 'chat',
    });
    // Full-page chat while the dock remembers "chat": show the calendar.
    expect(rightDockMode({ ...base, fullChatView: true })).toEqual({ kind: 'panel', content: 'calendar' });
  });
});

describe('railButtons', () => {
  it('lists chat first, then calendar, only when each can be docked', () => {
    expect(railButtons(base)).toEqual(['chat', 'calendar']);
    expect(railButtons({ ...base, aiEnabled: false })).toEqual(['calendar']);
    expect(railButtons({ ...base, calendarAvailable: false })).toEqual(['chat']);
    expect(railButtons({ ...base, fullChatView: true })).toEqual(['calendar']);
  });
});

describe('parseRightDockContent', () => {
  it('accepts the two known values and rejects the rest', () => {
    expect(parseRightDockContent('chat')).toBe('chat');
    expect(parseRightDockContent('calendar')).toBe('calendar');
    expect(parseRightDockContent('mail')).toBeNull();
    expect(parseRightDockContent('')).toBeNull();
  });
});
