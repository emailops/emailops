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

/** Color for an account's indicator in the unified ("All accounts") views. */
export function accountColorClass(accountId: string): string {
  return hashColorClass(accountId, ACCOUNT_PALETTE);
}

/** Colour for a sender's displayed name. Seeded on the lowercased address so
 *  one person keeps one colour regardless of how the provider cased the From
 *  header. */
export function senderTextColorClass(senderEmail: string): string {
  return hashColorClass(senderEmail.trim().toLowerCase(), SENDER_TEXT_PALETTE);
}
