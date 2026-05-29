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
