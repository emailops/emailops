/** Sidebar/menu label for a custom folder: hide the near-universal `INBOX.` /
 *  `INBOX/` prefix Dovecot-style servers put on every folder; keep the full
 *  path elsewhere (tooltips, identity). */
export function folderLabel(displayName: string, delimiter: string | null): string {
  const delim = delimiter ?? '.';
  const prefix = `INBOX${delim}`;
  if (displayName.toUpperCase().startsWith(prefix.toUpperCase()) && displayName.length > prefix.length) {
    return displayName.slice(prefix.length);
  }
  return displayName;
}
