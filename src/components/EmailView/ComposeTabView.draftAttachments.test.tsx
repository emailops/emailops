// A draft's file attachments are managed by the composer that opened with them.
// Removing the last one has to reach the backend: an omitted list means "leave
// them", so the file stayed on the draft, and an attachment that could not be
// downloaded could never be taken off to send the draft without it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => <div data-testid="body">{value}</div>,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
}));

import * as api from '@/lib/api';
import type { ComposeTab } from '@/stores/emailStore';
import type { Account } from '@/types';
import { ComposeTabView } from './ComposeTabView';

const account = { id: 'a1', provider: 'gmail', email: 'me@example.com', name: 'Me' } as Account;

const attachment = {
  id: 'att-1',
  draftId: 'd1',
  filePath: 'attachments/a1/drafts/5f1e.pdf',
  filename: 'report.pdf',
  mimeType: 'application/pdf',
};

const tab: ComposeTab = {
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  draftId: 'd1',
  toAddresses: ['bob@example.com'],
  subject: 'Report',
  bodyHtml: '<p>Attached</p>',
  attachments: [attachment],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.clearAllMocks();
});

async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
}

function lastSave() {
  const saves = vi.mocked(api.saveDraft).mock.calls;
  return saves[saves.length - 1][0];
}

describe('ComposeTabView with a draft file attachment', () => {
  it('keeps it across saves', async () => {
    await act(async () => {
      root.render(<ComposeTabView tab={tab} accounts={[account]} onClose={() => {}} />);
    });
    await settle();
    expect(lastSave().attachments).toEqual([
      { filePath: attachment.filePath, filename: attachment.filename, mimeType: attachment.mimeType },
    ]);
  });

  it('saves an empty list when the last one is removed', async () => {
    await act(async () => {
      root.render(<ComposeTabView tab={tab} accounts={[account]} onClose={() => {}} />);
    });
    const remove = container.querySelector<HTMLButtonElement>(`[title="${attachment.filePath}"] button`);
    if (!remove) throw new Error('attachment chip not rendered');
    await act(async () => {
      remove.click();
    });
    await settle();
    expect(lastSave().attachments).toEqual([]);
  });

  it('still leaves the attachments out when the tab opened with none', async () => {
    await act(async () => {
      root.render(<ComposeTabView tab={{ ...tab, attachments: undefined }} accounts={[account]} onClose={() => {}} />);
    });
    await settle();
    expect(lastSave().attachments).toBeUndefined();
  });
});
