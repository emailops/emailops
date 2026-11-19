// Opening a draft and closing it unchanged saved it: the tab's autosave ran on
// mount, and the editor handed back its re-serialised HTML (`<div>` lines as
// paragraphs, `<b>` as `<strong>`) as if the user had typed it. Every save
// marks the draft as edited here, so a Gmail or Outlook draft was rewritten
// upstream by merely opening it, and an IMAP draft stopped following the
// server. Runs the real editor, which is what re-serialises.

import type { Editor } from '@tiptap/react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  currentPlatform: () => 'macos',
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async (req: { id?: string }) => ({ id: req.id ?? 'd-new' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'req'),
}));

import * as api from '@/lib/api';
import type { ComposeTab } from '@/stores/emailStore';
import type { Account } from '@/types';
import { ComposeTabView } from './ComposeTabView';

const account = {
  id: 'a1',
  provider: 'imap',
  email: 'me@example.test',
  name: 'Me',
  createdAt: 0,
  sortOrder: 0,
  enabled: true,
  syncFromTimestamp: null,
} as Account;

/** A draft saved by another mail client, opened from Drafts. */
const draftTab: ComposeTab = {
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  toAddresses: ['dest@example.test'],
  subject: 'Plan',
  bodyHtml: '<div>Line one</div><div><b>Bold</b> line two</div>',
  draftId: 'd1',
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

async function open(tab: ComposeTab = draftTab) {
  await act(async () => {
    root.render(<ComposeTabView tab={tab} accounts={[account]} onClose={() => {}} />);
  });
}

async function waitPastAutosave() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
}

/** Tiptap hangs the editor off its contenteditable root. */
function editor(): Editor {
  const dom = container.querySelector('.ProseMirror') as (HTMLElement & { editor?: Editor }) | null;
  if (!dom?.editor) throw new Error('editor not found');
  return dom.editor;
}

describe('ComposeTabView opened on an existing draft', () => {
  it('does not save a draft that was only opened', async () => {
    await open();
    await waitPastAutosave();
    expect(api.saveDraft).not.toHaveBeenCalled();
  });

  it('saves an edit to the body once, into the same draft', async () => {
    await open();
    await act(async () => {
      editor().commands.insertContent('!');
    });
    await waitPastAutosave();

    expect(api.saveDraft).toHaveBeenCalledTimes(1);
    const saved = vi.mocked(api.saveDraft).mock.calls[0][0];
    expect(saved.id).toBe('d1');
    expect(saved.bodyHtml).toContain('!');
  });

  it('saves an edit to the subject once', async () => {
    await open();
    const subject = container.querySelector<HTMLInputElement>('input[placeholder="compose:subjectPlaceholderLong"]');
    if (!subject) throw new Error('subject input not rendered');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    await act(async () => {
      setter?.call(subject, 'Plan B');
      subject.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await waitPastAutosave();

    expect(api.saveDraft).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.saveDraft).mock.calls[0][0]).toMatchObject({ id: 'd1', subject: 'Plan B' });
  });
});

describe('ComposeTabView on a message with no draft yet', () => {
  it('saves what it opened with, as before', async () => {
    // A tab without a draft row (a message taken back into a tab) keeps it.
    const { draftId: _none, ...unsaved } = draftTab;
    await open(unsaved);
    await waitPastAutosave();

    expect(api.saveDraft).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.saveDraft).mock.calls[0][0].id).toBeUndefined();
  });
});
