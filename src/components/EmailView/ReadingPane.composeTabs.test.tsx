// What the user typed lives in the compose tab's own component, so the tab has
// to stay mounted while another one is shown, and each tab needs its instance.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./EmailView', () => ({ EmailView: () => <div data-testid="thread-view" /> }));
vi.mock('./EmailTabBar', () => ({ EmailTabBar: () => null }));
vi.mock('@/components/Attachments/PdfViewer', () => ({ PdfViewer: () => null }));
vi.mock('./ComposeTabView', () => ({
  ComposeTabView: ({ tab }: { tab: { id: string } }) => <textarea data-testid={`compose-${tab.id}`} />,
}));

import { initI18n } from '@/i18n';
import type { EmailTab } from '@/stores/emailStore';
import { ReadingPane } from './ReadingPane';

const tabA = { type: 'compose', id: 'compose-a' } as unknown as EmailTab;
const tabB = { type: 'compose', id: 'compose-b' } as unknown as EmailTab;

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(activeTab: EmailTab | null) {
  act(() =>
    root.render(
      <ReadingPane
        threadEmails={[]}
        isLoading={false}
        selectedEmail={null}
        tabs={[tabA, tabB]}
        activeTabId={activeTab?.id ?? null}
        activeTab={activeTab}
        accounts={[]}
        activeAccountId="acct-1"
        onSelectMainTab={() => {}}
        onSelectTab={() => {}}
        onCloseTab={() => {}}
        onCloseMain={() => {}}
      />,
    ),
  );
}

const editor = (tab: EmailTab) => container.querySelector<HTMLTextAreaElement>(`[data-testid="compose-${tab.id}"]`);

describe('ReadingPane compose tabs', () => {
  it('keeps what was typed when another tab is shown and back', () => {
    render(tabA);
    const typedInto = editor(tabA);
    if (typedInto) typedInto.value = 'half a sentence';

    render(null);
    render(tabA);

    expect(editor(tabA)?.value).toBe('half a sentence');
  });

  it('gives each compose tab its own editor', () => {
    render(tabA);
    const first = editor(tabA);
    if (first) first.value = 'to Ana';

    render(tabB);

    expect(editor(tabB)?.value).toBe('');
    expect(editor(tabA)?.value).toBe('to Ana');
  });

  it('shows the thread, not an editor, while no compose tab is active', () => {
    render(null);

    expect(container.querySelector('[data-testid="thread-view"]')).not.toBeNull();
  });
});
