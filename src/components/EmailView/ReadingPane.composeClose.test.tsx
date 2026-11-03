// The autosaved draft keeps the text of a message, not the files attached to it,
// which live in the compose tab until the message is sent. Closing the tab, from
// its own × or from the tab bar, therefore asks first while it holds files.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('./EmailView', () => ({ EmailView: () => <div data-testid="thread-view" /> }));
vi.mock('@/components/Attachments/PdfViewer', () => ({ PdfViewer: () => null }));
vi.mock('@/components/shared/RichTextEditor', () => ({ RichTextEditor: () => <div data-testid="body" /> }));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
}));

import type { EmailAttachment } from '@/lib/api';
import type { ComposeTab, EmailTab } from '@/stores/emailStore';
import type { Account } from '@/types';
import { ReadingPane } from './ReadingPane';

const account = { id: 'a1', email: 'me@example.com', name: 'Me', enabled: true } as Account;
const file: EmailAttachment = { filename: 'contract.pdf', mimeType: 'application/pdf', data: 'AAAA' };
const composeTab = (over: Partial<ComposeTab> = {}): ComposeTab => ({
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  toAddresses: ['bob@example.com'],
  subject: 'Feedback',
  bodyHtml: '<p>Hi</p>',
  ...over,
});

let container: HTMLDivElement;
let root: Root;
const onCloseTab = vi.fn();
const onSelectTab = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(tab: EmailTab, activeTabId: string | null = tab.id) {
  await act(async () => {
    root.render(
      <ReadingPane
        threadEmails={[]}
        isLoading={false}
        selectedEmail={null}
        tabs={[tab]}
        activeTabId={activeTabId}
        activeTab={activeTabId ? tab : null}
        accounts={[account]}
        activeAccountId="a1"
        onSelectMainTab={() => {}}
        onSelectTab={onSelectTab}
        onCloseTab={onCloseTab}
        onCloseMain={() => {}}
      />,
    );
  });
}

const tabBarClose = () => container.querySelector<HTMLElement>('[aria-label="inbox:emailView.closeTab"]');
const ownClose = () => container.querySelector<HTMLElement>('[data-testid="compose-tab-close"]');
const confirmation = () => document.querySelector<HTMLElement>('[data-testid="modal-overlay"]');
const button = (label: string, scope: ParentNode) => {
  const found = [...scope.querySelectorAll('button')].find((b) => b.textContent === label);
  if (!found) throw new Error(`${label} not rendered`);
  return found;
};

describe('closing a compose tab', () => {
  it.each([
    ['the tab bar', tabBarClose],
    ['its own ×', ownClose],
  ])('asks from %s while it holds files', async (_where, close) => {
    await mount(composeTab({ fileAttachments: [file] }));

    await act(async () => close()?.click());

    expect(onCloseTab).not.toHaveBeenCalled();
    expect(confirmation()?.textContent).toContain('compose:closeWithFiles.title');
  });

  it('closes at once without files: the draft has everything', async () => {
    await mount(composeTab());

    await act(async () => tabBarClose()?.click());

    expect(onCloseTab).toHaveBeenCalledWith('t1');
    expect(confirmation()).toBeNull();
  });

  it('keeps writing when the user says so', async () => {
    await mount(composeTab({ fileAttachments: [file] }));
    await act(async () => tabBarClose()?.click());

    await act(async () => button('compose:closeWithFiles.keep', confirmation() as HTMLElement).click());

    expect(confirmation()).toBeNull();
    expect(onCloseTab).not.toHaveBeenCalled();
    expect(container.textContent).toContain('contract.pdf');
  });

  it('closes when the user confirms', async () => {
    await mount(composeTab({ fileAttachments: [file] }));
    await act(async () => tabBarClose()?.click());

    await act(async () => button('common:actions.close', confirmation() as HTMLElement).click());

    expect(onCloseTab).toHaveBeenCalledWith('t1');
  });

  it('brings a tab behind another one forward: its question would not show otherwise', async () => {
    await mount(composeTab({ fileAttachments: [file] }), null);

    await act(async () => tabBarClose()?.click());

    expect(onSelectTab).toHaveBeenCalledWith('t1');
    expect(onCloseTab).not.toHaveBeenCalled();
  });
});
