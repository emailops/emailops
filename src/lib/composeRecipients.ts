/** Normalize a recipient string: strip a `Name <addr>` wrapper, trim, lowercase. */
export function extractEmail(raw: string): string {
  const match = raw.match(/<([^>]+)>/);
  return (match ? match[1] : raw).trim().toLowerCase();
}

/**
 * Merge a committed recipient list with a pending, not-yet-tokenized input.
 *
 * The recipient field keeps confirmed addresses as tokens (`toRecipients`) and
 * the in-progress text separately (`toInput`). A valid email left in the input
 * box — the user typed it but didn't press Enter/Tab or pick a suggestion —
 * must still count, otherwise Send stays disabled and the typed address is
 * silently dropped when the message is sent.
 */
export function mergePendingRecipient(committed: string[], pendingInput: string): string[] {
  const pending = extractEmail(pendingInput);
  if (!pending.includes('@')) return committed;
  if (committed.includes(pending)) return committed;
  return [...committed, pending];
}
