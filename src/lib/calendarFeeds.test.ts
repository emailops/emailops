// Feed calendars are read-only, and a Proton feed can still "create" an event
// by handing the day over to Proton Calendar's own create deep link.
import { describe, expect, it } from 'vitest';
import type { Calendar } from '@/types';
import { isProtonFeed, isReadOnlyCalendar, planCreateSlot, protonCreateEventUrl } from './calendarFeeds';

function calendar(partial: Partial<Calendar>): Calendar {
  return {
    id: 'acc:cal',
    accountId: 'acc',
    providerCalendarId: 'cal',
    name: 'Cal',
    color: '',
    isPrimary: false,
    accessRole: 'owner',
    isVisible: true,
    sortOrder: 0,
    feedHost: null,
    createdAt: 0,
    updatedAt: 0,
    ...partial,
  };
}

const OWNED = calendar({ providerCalendarId: 'primary', isPrimary: true });
const PROTON = calendar({ providerCalendarId: 'feed-1', accessRole: 'reader', feedHost: 'calendar.proton.me' });
const OTHER_FEED = calendar({ providerCalendarId: 'feed-2', accessRole: 'reader', feedHost: 'cal.example.com' });

describe('isReadOnlyCalendar', () => {
  it('is true for reader and freeBusyReader roles only', () => {
    expect(isReadOnlyCalendar(PROTON)).toBe(true);
    expect(isReadOnlyCalendar(calendar({ accessRole: 'freeBusyReader' }))).toBe(true);
    expect(isReadOnlyCalendar(OWNED)).toBe(false);
    expect(isReadOnlyCalendar(calendar({ accessRole: 'writer' }))).toBe(false);
  });
});

describe('isProtonFeed', () => {
  it('matches the Proton Calendar host case-insensitively and nothing else', () => {
    expect(isProtonFeed(PROTON)).toBe(true);
    expect(isProtonFeed(calendar({ feedHost: 'Calendar.Proton.ME' }))).toBe(true);
    expect(isProtonFeed(OTHER_FEED)).toBe(false);
    expect(isProtonFeed(OWNED)).toBe(false);
  });
});

describe('protonCreateEventUrl', () => {
  it('opens the week of the local day with the create action and an empty title', () => {
    // Local date parts, month and day unpadded (Proton's own route shape).
    expect(protonCreateEventUrl(new Date(2026, 2, 5, 9, 30))).toBe(
      'https://calendar.proton.me/week/2026/3/5?action=create&title=',
    );
    expect(protonCreateEventUrl(new Date(2026, 11, 25))).toBe(
      'https://calendar.proton.me/week/2026/12/25?action=create&title=',
    );
  });
});

describe('planCreateSlot', () => {
  const slot = new Date(2026, 6, 28, 10, 0).getTime() / 1000;

  it('opens the in-app dialog when the account owns a writable calendar', () => {
    expect(planCreateSlot([PROTON, OWNED], slot)).toEqual({ kind: 'dialog' });
  });

  it('hands over to Proton when the only visible calendars are Proton feeds', () => {
    expect(planCreateSlot([PROTON], slot)).toEqual({
      kind: 'proton',
      url: 'https://calendar.proton.me/week/2026/7/28?action=create&title=',
    });
  });

  it('ignores hidden Proton feeds and does nothing for other read-only feeds', () => {
    expect(planCreateSlot([{ ...PROTON, isVisible: false }], slot)).toEqual({ kind: 'none' });
    expect(planCreateSlot([OTHER_FEED], slot)).toEqual({ kind: 'none' });
    expect(planCreateSlot([], slot)).toEqual({ kind: 'none' });
  });
});
