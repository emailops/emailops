export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const gb = bytes / 1e9;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / 1e6;
  return `${mb.toFixed(0)} MB`;
}

export function formatProgress(downloaded: number, total: number): string {
  if (total === 0) return '…';
  const pct = Math.round((downloaded / total) * 100);
  return `${pct}% · ${formatBytes(downloaded)} / ${formatBytes(total)}`;
}

/** Default prompt budget for remote (OpenRouter) models, in tokens. */
export const DEFAULT_CONTEXT_BUDGET = 32768;
/** Smallest budget the backend accepts for `chat.remote_n_ctx_budget`. */
export const MIN_CONTEXT_BUDGET = 4096;

/** The budget to show for a stored `chat.remote_n_ctx_budget` value. */
export function contextBudgetFromPref(raw: string | null): number {
  const n = raw != null && raw.trim() !== '' ? Number.parseInt(raw, 10) : Number.NaN;
  return Number.isFinite(n) && n >= MIN_CONTEXT_BUDGET ? n : DEFAULT_CONTEXT_BUDGET;
}

/** The value to store for a budget typed in Settings. */
export function contextBudgetToPref(tokens: number): string {
  if (!Number.isFinite(tokens)) return String(DEFAULT_CONTEXT_BUDGET);
  return String(Math.max(MIN_CONTEXT_BUDGET, Math.round(tokens)));
}
