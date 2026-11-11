// Contract: the calendar commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe, expect, it } from 'vitest';
import { itMatchesRustArguments, rustStructFields, tsInterfaceFields } from './contract';

describe('api.ts ↔ Rust: calendar commands', () => {
  itMatchesRustArguments(['calendar.rs']);
});

describe('calendar responses have the shape the frontend types declare', () => {
  it('get_calendar_invite answers CalendarInvite', () => {
    expect(rustStructFields('services/calendar/invite.rs', 'CalendarInvite')).toEqual(
      tsInterfaceFields('CalendarInvite'),
    );
  });

  it('get_calendars answers Calendar', () => {
    expect(rustStructFields('models/mod.rs', 'Calendar')).toEqual(tsInterfaceFields('Calendar'));
  });

  it('list_calendar_feeds answers CalendarFeed', () => {
    expect(rustStructFields('models/mod.rs', 'CalendarFeed')).toEqual(tsInterfaceFields('CalendarFeed'));
  });
});
