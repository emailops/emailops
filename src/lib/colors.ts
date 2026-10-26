// Colour helpers: deterministic hashing for avatars and sender names (same
// seed → same colour across the app) and each account's stored colour.

import type { Account, AccountColor } from '@/types';

/** Palette used by sender avatars (moved here from EmailRow). */
export const AVATAR_PALETTE = [
  'bg-blue-500',
  'bg-emerald-500',
  'bg-purple-500',
  'bg-pink-500',
  'bg-amber-500',
  'bg-cyan-500',
  'bg-indigo-500',
  'bg-rose-500',
  'bg-teal-500',
  'bg-orange-500',
];

/** Account colours in palette order — the `accounts.color` keys (V040). The
 *  backend assigns new accounts the least-used one; the user can change it in
 *  the account's settings. */
export const ACCOUNT_COLORS: readonly AccountColor[] = [
  'blue',
  'emerald',
  'amber',
  'purple',
  'rose',
  'cyan',
  'indigo',
  'orange',
];

/** Dot / bar class per account colour (literal so Tailwind keeps them). */
const ACCOUNT_COLOR_CLASS: Record<AccountColor, string> = {
  blue: 'bg-blue-500',
  emerald: 'bg-emerald-500',
  amber: 'bg-amber-500',
  purple: 'bg-purple-500',
  rose: 'bg-rose-500',
  cyan: 'bg-cyan-500',
  indigo: 'bg-indigo-500',
  orange: 'bg-orange-500',
};

/** The same colours as hex (Tailwind's 500 step), for the calendar chips,
 *  which compute their text contrast from the fill. */
const ACCOUNT_COLOR_HEX: Record<AccountColor, string> = {
  blue: '#3b82f6',
  emerald: '#10b981',
  amber: '#f59e0b',
  purple: '#a855f7',
  rose: '#f43f5e',
  cyan: '#06b6d4',
  indigo: '#6366f1',
  orange: '#f97316',
};

/** Text colours for sender names. Deliberately darker than `AVATAR_PALETTE`
 *  (600/700 rather than 500) because these are set on small type against a
 *  white card and have to clear contrast, not just be distinguishable. */
export const SENDER_TEXT_PALETTE = [
  'text-blue-700',
  'text-emerald-700',
  'text-purple-700',
  'text-pink-700',
  'text-amber-700',
  'text-cyan-700',
  'text-indigo-700',
  'text-rose-700',
  'text-teal-700',
  'text-orange-700',
];

/** Deterministic color from a seed string so the same seed always renders
 *  with the same color across the app. */
export function hashColorClass(seed: string, palette: string[]): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) {
    hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  }
  return palette[hash % palette.length];
}

/** Class for an account's colour dot or bar. An account the list does not
 *  hold (removed meanwhile) gets a neutral grey. */
export function accountColorClass(accounts: ReadonlyArray<Pick<Account, 'id' | 'color'>>, accountId: string): string {
  const color = accounts.find((a) => a.id === accountId)?.color;
  return color ? ACCOUNT_COLOR_CLASS[color] : 'bg-gray-400';
}

/** Class for an account colour itself (the colour picker's swatches). */
export function accountColorSwatchClass(color: AccountColor): string {
  return ACCOUNT_COLOR_CLASS[color];
}

export function accountColorHex(color: AccountColor): string {
  return ACCOUNT_COLOR_HEX[color];
}

/** Colour for a sender's displayed name. Seeded on the lowercased address so
 *  one person keeps one colour regardless of how the provider cased the From
 *  header. */
export function senderTextColorClass(senderEmail: string): string {
  return hashColorClass(senderEmail.trim().toLowerCase(), SENDER_TEXT_PALETTE);
}
