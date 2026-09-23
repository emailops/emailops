// A Lens scope could only name the five built-in mailboxes, so mail filed
// into a custom IMAP folder was unreachable by any Lens.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/lib/api', () => ({
  getFolders: vi.fn(),
}));

import * as api from '@/lib/api';
import { LensFolderChips } from './LensFolderChips';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function render(accountId: string, selected: string[], onToggle = vi.fn()) {
  await act(async () => {
    root.render(<LensFolderChips accountId={accountId} selected={selected} onToggle={onToggle} />);
  });
  return onToggle;
}

describe('LensFolderChips', () => {
  it("offers the account's folders and toggles them as folder:<serverPath>", async () => {
    vi.mocked(api.getFolders).mockResolvedValue([
      {
        id: 'f1',
        accountId: 'acc-1',
        serverPath: 'INBOX.Quotes',
        displayName: 'INBOX.Quotes',
        role: '',
        delimiter: '.',
      },
    ]);
    const onToggle = await render('acc-1', []);

    expect(api.getFolders).toHaveBeenCalledWith('acc-1');
    const chip = [...container.querySelectorAll('button')].find((b) => b.textContent === 'Quotes');
    expect(chip).toBeDefined();
    act(() => chip?.click());
    expect(onToggle).toHaveBeenCalledWith('folder:INBOX.Quotes');
  });

  it('asks for an account instead of listing folders across all accounts', async () => {
    await render('', []);
    expect(api.getFolders).not.toHaveBeenCalled();
    expect(container.textContent).toContain('lenses:scope.foldersPickAccount');
  });

  it('renders nothing for an account without custom folders', async () => {
    vi.mocked(api.getFolders).mockResolvedValue([]);
    await render('acc-gmail', []);
    expect(container.textContent).toBe('');
  });

  it('surfaces a folder-load failure instead of silently showing no folders', async () => {
    vi.mocked(api.getFolders).mockRejectedValue(new Error('offline'));
    await render('acc-1', []);
    expect(container.textContent).toContain('lenses:scope.foldersLoadError');
  });
});
