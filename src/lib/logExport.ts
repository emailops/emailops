// Turning the output panel into text someone can paste.
//
// Pure and DOM-free so the shape is table-testable: the view only decides when
// to call it and where to put the result.

import type { LogEntry } from '@/stores/logStore';

/** `HH:MM:SS` in the device's own timezone.
 *
 *  Local, so a pasted line matches what the panel showed on screen — someone
 *  reporting "it stalled at 21:35" must find 21:35 in the paste. Fixed-width
 *  rather than locale-formatted, because the reader is often in another locale
 *  and `03:04:05` is unambiguous where `3:04:05 a. m.` is not. */
function clockTime(timestamp: number): string {
  const d = new Date(timestamp);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/**
 * One line per entry: `HH:MM:SS  LEVEL  source  message`.
 *
 * Newlines inside a message are folded to spaces. Backend errors arrive with
 * embedded newlines, and one entry spilling across lines makes the paste
 * unreadable exactly when it matters — the failure case is the one being
 * reported.
 */
export function formatLogsForCopy(entries: LogEntry[]): string {
  return entries
    .map((e) => `${clockTime(e.timestamp)}  ${e.level.padEnd(7)}  ${e.source.padEnd(11)}  ${foldLines(e.message)}`)
    .join('\n');
}

function foldLines(message: string): string {
  return message.replace(/\s*\n\s*/g, ' ').trim();
}
