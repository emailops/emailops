/** Stable color hash so each avatar isn't the same color. */
export function avatarColors(seed: string): { bg: string; fg: string } {
  let h = 0;
  for (let i = 0; i < seed.length; i += 1) {
    h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  }
  const palette = [
    { bg: 'bg-blue-100', fg: 'text-blue-700' },
    { bg: 'bg-emerald-100', fg: 'text-emerald-700' },
    { bg: 'bg-amber-100', fg: 'text-amber-700' },
    { bg: 'bg-rose-100', fg: 'text-rose-700' },
    { bg: 'bg-violet-100', fg: 'text-violet-700' },
    { bg: 'bg-cyan-100', fg: 'text-cyan-700' },
    { bg: 'bg-fuchsia-100', fg: 'text-fuchsia-700' },
    { bg: 'bg-lime-100', fg: 'text-lime-700' },
  ];
  return palette[h % palette.length];
}
