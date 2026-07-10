// Deterministic color hashing shared by avatars and the unified-inbox
// account indicator. Same seed → same color, across the whole app.

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

/** Smaller high-contrast palette for the per-account indicator in the unified
 *  inbox — distinct hues so a handful of accounts stay tell-apart-able. */
export const ACCOUNT_PALETTE = [
  'bg-blue-500',
  'bg-emerald-500',
  'bg-amber-500',
  'bg-purple-500',
  'bg-rose-500',
  'bg-cyan-500',
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

/** Color for an account's indicator in the unified ("All accounts") views. */
export function accountColorClass(accountId: string): string {
  return hashColorClass(accountId, ACCOUNT_PALETTE);
}
