/**
 * Pure planning for the calendar's account scope: one account, or "All
 * accounts" (`ALL_ACCOUNTS_ID`), which shows every calendar-enabled account's
 * events together, each in its account's colour (docs/DECISIONS.md).
 */
import { ALL_ACCOUNTS_ID } from '@/stores/accountStore';
import type { Account, Calendar, CalendarEvent } from '@/types';
import { type CreateSlotPlan, planCreateSlot } from './calendarFeeds';
import { accountColorHex } from './colors';

/** The account ids whose calendars the selection shows. */
export function calendarSelectionIds(selectedId: string | null, calendarAccounts: readonly Account[]): string[] {
  if (selectedId === null) return [];
  if (selectedId === ALL_ACCOUNTS_ID) return calendarAccounts.map((a) => a.id);
  return [selectedId];
}

/** The calendar's account selector: "All accounts" first once there are two
 *  accounts to combine, then each account by address. */
export function calendarAccountOptions(
  calendarAccounts: readonly Account[],
  allLabel: string,
): Array<{ value: string; label: string }> {
  const options = calendarAccounts.map((a) => ({ value: a.id, label: a.email }));
  return calendarAccounts.length >= 2 ? [{ value: ALL_ACCOUNTS_ID, label: allLabel }, ...options] : options;
}

/** The legend in "All accounts": each account's calendars under it, in the
 *  accounts' (sidebar) order; accounts with no calendar are left out. */
export function calendarLegendGroups(
  calendars: readonly Calendar[],
  accounts: readonly Account[],
): Array<{ account: Account; calendars: Calendar[] }> {
  return accounts
    .map((account) => ({ account, calendars: calendars.filter((c) => c.accountId === account.id) }))
    .filter((group) => group.calendars.length > 0);
}

/** An account a new event can go to, and how it is created there. */
export interface CreateAccountChoice {
  account: Account;
  plan: Exclude<CreateSlotPlan, { kind: 'none' }>;
}

/** Which accounts a double-clicked slot can create an event in, and how: the
 *  in-app dialog, or Proton Calendar's own form (Proton has no write API).
 *  Accounts with only other read-only feeds cannot create anything. */
export function createAccountChoices(
  accounts: readonly Account[],
  calendars: readonly Calendar[],
  slotStartSec: number,
): CreateAccountChoice[] {
  return accounts.flatMap((account) => {
    const plan = planCreateSlot(
      calendars.filter((c) => c.accountId === account.id),
      slotStartSec,
    );
    return plan.kind === 'none' ? [] : [{ account, plan }];
  });
}

/** An event's colour: its account's in "All accounts" (several accounts on
 *  one grid), its calendar's when one account is shown. */
export function eventColorFor(
  isAllAccounts: boolean,
  accounts: readonly Account[],
  calendarColor: (event: CalendarEvent) => string,
): (event: CalendarEvent) => string {
  if (!isAllAccounts) return calendarColor;
  const hexById = new Map(accounts.map((a) => [a.id, accountColorHex(a.color)]));
  return (event) => hexById.get(event.accountId) ?? calendarColor(event);
}
