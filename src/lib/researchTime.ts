// Time figures for research mode: the estimate before a run and the time left
// while it reads. Pure so they are testable without a clock.

/** "<1 min", "35 min", "2 h 10 min" — a glanceable duration. */
export function formatDuration(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return '<1 min';
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** Seconds left at the pace so far, or null before any email was read. */
export function remainingSeconds(
  progress: { emailsRead: number; emailsTotal: number },
  startedAtMs: number,
  nowMs: number,
): number | null {
  if (progress.emailsRead <= 0) return null;
  const perEmail = (nowMs - startedAtMs) / 1000 / progress.emailsRead;
  return Math.max(0, Math.round(perEmail * (progress.emailsTotal - progress.emailsRead)));
}
