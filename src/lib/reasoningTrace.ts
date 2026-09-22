// Formatting helpers for the reasoning panel. The turn's step order and its
// KV-cache facts are built once in the backend (`services::chat::trace_steps`)
// and arrive on `ChatTrace.steps`, shared with the CLI and the eval report.

/** Format a millisecond duration as "1.2s" (>= 1s) or "850ms". */
export function formatLatency(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

/** Turn-level throughput. Returns 0 when timing or token data is missing so
 *  callers can branch on a falsy value instead of guarding NaN/Infinity. */
export function tokensPerSecond(tokens: number | null | undefined, ms: number | null | undefined): number {
  if (!tokens || !ms || ms <= 0) {
    return 0;
  }
  return tokens / (ms / 1000);
}
