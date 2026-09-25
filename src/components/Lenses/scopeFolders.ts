// Folder selection for a Lens scope. Custom IMAP folders are stored as the
// mailbox value `folder:<serverPath>`, so a folder chip is just another entry
// in `LensScope.mailboxes` next to the built-in ones.

import type { Folder } from '@/lib/api';
import { folderLabel } from '@/lib/folderDisplay';

const FOLDER_PREFIX = 'folder:';

export interface FolderChip {
  value: string;
  label: string;
  /** Selected in the scope but no longer among the account's folders. */
  missing: boolean;
}

export function folderChips(folders: Folder[], selected: string[]): FolderChip[] {
  const chips: FolderChip[] = folders.map((f) => ({
    value: `${FOLDER_PREFIX}${f.serverPath}`,
    label: folderLabel(f.displayName, f.delimiter),
    missing: false,
  }));
  const known = new Set(chips.map((c) => c.value));
  for (const value of selected) {
    if (value.startsWith(FOLDER_PREFIX) && !known.has(value)) {
      chips.push({ value, label: value.slice(FOLDER_PREFIX.length), missing: true });
    }
  }
  return chips;
}

export function withoutFolderMailboxes(mailboxes: string[]): string[] {
  return mailboxes.filter((m) => !m.startsWith(FOLDER_PREFIX));
}

/** Chips to render: selected ones first (always, whatever the filter says, so
 *  a long list never buries the current scope), then the unselected ones whose
 *  label contains `query`, case-insensitively. */
export function visibleFolderChips(chips: FolderChip[], selected: string[], query: string): FolderChip[] {
  const needle = query.trim().toLowerCase();
  const picked = chips.filter((c) => selected.includes(c.value));
  const rest = chips.filter((c) => !selected.includes(c.value) && c.label.toLowerCase().includes(needle));
  return [...picked, ...rest];
}
