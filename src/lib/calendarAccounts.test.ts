import { describe, expect, it } from 'vitest';
import { ALL_ACCOUNTS_ID } from '@/stores/accountStore';
import type { Account, Calendar, CalendarEvent } from '@/types';
import {
  calendarAccountOptions,
  calendarLegendGroups,
  calendarSelectionIds,
  createAccountChoices,
  eventColorFor,
} from './calendarAccounts';
import { accountColorHex } from './colors';

function account(id: string, extra: Partial<Account> = {}): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@mail.example`,
    name: '',
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
    color: 'blue',
    ...extra,
  };
}

function calendar(accountId: string, providerCalendarId: string, extra: Partial<Calendar> = {}): Calendar {
  return {
    id: `${accountId}:${providerCalendarId}`,
    accountId,
    providerCalendarId,
    name: providerCalendarId,
    color: '',
    isPrimary: false,
    accessRole: 'owner',
    isVisible: true,
    feedHost: null,
    ...extra,
  } as Calendar;
}

function event(accountId: string, calendarId: string): CalendarEvent {
  return { id: `${accountId}-${calendarId}`, accountId, calendarId } as CalendarEvent;
}

const a = account('a', { color: 'rose' });
const b = account('b', { color: 'cyan' });

describe('calendarSelectionIds', () => {
  it('loads every calendar account in "All accounts"', () => {
    expect(calendarSelectionIds(ALL_ACCOUNTS_ID, [a, b])).toEqual(['a', 'b']);
  });

  it('loads just the picked account otherwise', () => {
    expect(calendarSelectionIds('b', [a, b])).toEqual(['b']);
  });

  it('loads nothing before a selection exists', () => {
    expect(calendarSelectionIds(null, [a, b])).toEqual([]);
  });
});

describe('calendarAccountOptions', () => {
  it('puts "All accounts" first when two or more accounts have a calendar', () => {
    expect(calendarAccountOptions([a, b], 'All accounts')).toEqual([
      { value: ALL_ACCOUNTS_ID, label: 'All accounts' },
      { value: 'a', label: 'a@mail.example' },
      { value: 'b', label: 'b@mail.example' },
    ]);
  });

  it('offers no "All accounts" for a single account', () => {
    expect(calendarAccountOptions([a], 'All accounts')).toEqual([{ value: 'a', label: 'a@mail.example' }]);
  });
});

describe('calendarLegendGroups', () => {
  it("groups calendars under their account, in the accounts' order", () => {
    const groups = calendarLegendGroups([calendar('b', 'work'), calendar('a', 'home'), calendar('b', 'team')], [a, b]);
    expect(groups.map((g) => [g.account.id, g.calendars.map((c) => c.providerCalendarId)])).toEqual([
      ['a', ['home']],
      ['b', ['work', 'team']],
    ]);
  });

  it('leaves out accounts without calendars', () => {
    expect(calendarLegendGroups([calendar('b', 'work')], [a, b]).map((g) => g.account.id)).toEqual(['b']);
  });
});

describe('createAccountChoices', () => {
  const slot = Math.floor(new Date(2026, 9, 26, 10).getTime() / 1000);

  it('offers the accounts with a writable calendar', () => {
    const choices = createAccountChoices([a, b], [calendar('a', 'primary'), calendar('b', 'primary')], slot);
    expect(choices.map((c) => [c.account.id, c.plan.kind])).toEqual([
      ['a', 'dialog'],
      ['b', 'dialog'],
    ]);
  });

  it('offers a Proton account as a hand-over to Proton Calendar', () => {
    const proton = account('p', { provider: 'imap' });
    const choices = createAccountChoices(
      [a, proton],
      [calendar('a', 'primary'), calendar('p', 'feed-1', { accessRole: 'reader', feedHost: 'calendar.proton.me' })],
      slot,
    );
    expect(choices.map((c) => [c.account.id, c.plan.kind])).toEqual([
      ['a', 'dialog'],
      ['p', 'proton'],
    ]);
  });

  it('leaves out an account whose calendars are all read-only, non-Proton feeds', () => {
    const feedOnly = account('f', { provider: 'imap' });
    const choices = createAccountChoices(
      [a, feedOnly],
      [calendar('a', 'primary'), calendar('f', 'feed-1', { accessRole: 'reader', feedHost: 'cal.example' })],
      slot,
    );
    expect(choices.map((c) => c.account.id)).toEqual(['a']);
  });
});

describe('eventColorFor', () => {
  const calendarColor = () => '#039be5';

  it('paints events with their account\'s colour in "All accounts"', () => {
    const colorFor = eventColorFor(true, [a, b], calendarColor);
    expect(colorFor(event('b', 'primary'))).toBe(accountColorHex('cyan'));
  });

  it("keeps each calendar's own colour for a single account", () => {
    const colorFor = eventColorFor(false, [a, b], calendarColor);
    expect(colorFor(event('b', 'primary'))).toBe('#039be5');
  });
});
