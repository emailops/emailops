// The active compose tab is the draft on screen for the chat panel. Inactive
// tabs stay mounted behind it, so they must not claim the slot.

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
  deleteDraft: vi.fn(async () => {}),
}));

import { useDraftContextStore } from '@/stores/draftContextStore';
import type { ComposeTab } from '@/stores/emailStore';
import type { Account } from '@/types';
import { ComposeTabView } from './ComposeTabView';

const account = { id: 'a1', provider: 'imap', email: 'me@example.com', name: 'Me', enabled: true } as Account;
const newTab: ComposeTab = {
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  toAddresses: ['ana@example.com'],
  subject: 'Q3 report',
  bodyHtml: '<p>The figures are final.</p>',
};

let container: HTMLDivElement;
let root: Root;
const store = () => useDraftContextStore.getState();

beforeEach(() => {
  useDraftContextStore.setState({ active: null, subject: '', hasUserText: false });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(tab: ComposeTab, isActive: boolean) {
  await act(async () => {
    root.render(<ComposeTabView tab={tab} accounts={[account]} onClose={() => {}} isActive={isActive} />);
  });
}

describe('ComposeTabView as the draft on screen', () => {
  it('does not offer a tab behind another one', async () => {
    await render(newTab, false);
    expect(store().active).toBeNull();
  });

  it('offers the active tab as a new email', async () => {
    await render(newTab, true);
    expect(store().active?.kind).toBe('new');
    expect(store().subject).toBe('Q3 report');
    expect(store().active?.snapshot()).toEqual({
      kind: 'new',
      subject: 'Q3 report',
      to: ['ana@example.com'],
      cc: [],
      body: 'The figures are final.',
      replyToEmailId: null,
    });
  });

  it('offers a tab once it becomes active', async () => {
    await render(newTab, false);
    await render(newTab, true);
    expect(store().active).not.toBeNull();
  });

  it('offers a reply reopened in a tab as a reply to its email', async () => {
    await render({ ...newTab, replyToEmailId: 'e1' }, true);
    expect(store().active?.kind).toBe('reply');
    expect(store().active?.snapshot()?.replyToEmailId).toBe('e1');
  });
});
