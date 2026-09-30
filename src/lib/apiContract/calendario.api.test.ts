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
});
