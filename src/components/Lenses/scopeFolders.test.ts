import { describe, expect, it } from 'vitest';

import type { Folder } from '@/lib/api';

import { type FolderChip, folderChips, visibleFolderChips, withoutFolderMailboxes } from './scopeFolders';

function folder(serverPath: string, displayName = serverPath, delimiter: string | null = '.'): Folder {
  return { id: `id-${serverPath}`, accountId: 'acct1', serverPath, displayName, role: '', delimiter };
}

describe('folderChips', () => {
  it('offers one chip per account folder, addressed as folder:<serverPath>', () => {
    const chips = folderChips([folder('INBOX.Quotes'), folder('Projects')], []);
    expect(chips).toEqual([
      { value: 'folder:INBOX.Quotes', label: 'Quotes', missing: false },
      { value: 'folder:Projects', label: 'Projects', missing: false },
    ]);
  });

  it('labels with the decoded display name, not the wire path', () => {
    const chips = folderChips([folder('INBOX.Presupuestos &AOk-', 'INBOX.Presupuestos é')], []);
    expect(chips[0]).toEqual({ value: 'folder:INBOX.Presupuestos &AOk-', label: 'Presupuestos é', missing: false });
  });

  it('keeps a selected folder the account no longer has, so it can be deselected', () => {
    // Renamed or deleted on the server: without a chip the scope would keep
    // matching nothing for it and the user could not see why.
    const chips = folderChips([folder('Projects')], ['inbox', 'folder:Old/Clients']);
    expect(chips).toEqual([
      { value: 'folder:Projects', label: 'Projects', missing: false },
      { value: 'folder:Old/Clients', label: 'Old/Clients', missing: true },
    ]);
  });

  it('ignores built-in mailboxes in the selection', () => {
    expect(folderChips([], ['inbox', 'sent'])).toEqual([]);
  });
});

describe('withoutFolderMailboxes', () => {
  it('drops folder selections and keeps the built-in mailboxes', () => {
    // Folders belong to one account, so switching account must not carry
    // them over to an account that has no such folder.
    expect(withoutFolderMailboxes(['inbox', 'folder:Projects', 'sent'])).toEqual(['inbox', 'sent']);
  });
});

describe('visibleFolderChips', () => {
  const chip = (label: string, missing = false): FolderChip => ({ value: `folder:${label}`, label, missing });
  const chips = [chip('Archive/2025'), chip('Clients'), chip('Quotes'), chip('Old', true)];

  it('lists selected folders first so they stay in view above a long list', () => {
    const got = visibleFolderChips(chips, ['folder:Quotes', 'folder:Old'], '');
    expect(got.map((c) => c.label)).toEqual(['Quotes', 'Old', 'Archive/2025', 'Clients']);
  });

  it('filters unselected folders by a case-insensitive match on the label', () => {
    const got = visibleFolderChips(chips, [], 'CLI');
    expect(got.map((c) => c.label)).toEqual(['Clients']);
  });

  it('never hides a selected folder behind the filter', () => {
    const got = visibleFolderChips(chips, ['folder:Quotes'], 'arch');
    expect(got.map((c) => c.label)).toEqual(['Quotes', 'Archive/2025']);
  });
});
