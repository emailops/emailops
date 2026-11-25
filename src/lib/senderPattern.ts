/** One entry of a comma-separated sender pattern: its bounds in the value, trimmed. */
export interface SenderEntry {
  start: number;
  end: number;
  /** What to search senders with: the entry without its `*` wildcards. */
  text: string;
}

/** The entry of the comma-separated sender pattern `value` that holds `cursor`. */
export function senderEntryAt(value: string, cursor: number): SenderEntry {
  const segmentStart = value.lastIndexOf(',', cursor - 1) + 1;
  const nextComma = value.indexOf(',', cursor);
  const segmentEnd = nextComma === -1 ? value.length : nextComma;
  const segment = value.slice(segmentStart, segmentEnd);
  const start = segmentStart + (segment.length - segment.trimStart().length);
  const end = segmentEnd - (segment.length - segment.trimEnd().length);
  return { start, end, text: value.slice(start, end).replace(/\*/g, '') };
}

/** `value` with `entry` replaced by `address`; the other entries stay as typed. */
export function replaceSenderEntry(value: string, entry: SenderEntry, address: string): string {
  return value.slice(0, entry.start) + address + value.slice(entry.end);
}
