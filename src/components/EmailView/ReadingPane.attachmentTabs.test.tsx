// An attachment tab keeps its viewer alive while another tab is shown, so a
// PDF read halfway down is still there on return. The built-in PDF viewer
// lives in an opaque-origin iframe whose scroll position cannot be read or
// restored, so the iframe itself has to survive the tab switch.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./EmailView', () => ({ EmailView: () => <div data-testid="thread-view" /> }));
vi.mock('./ComposeTabView', () => ({ ComposeTabView: () => null }));
vi.mock('./EmailTabBar', () => ({ EmailTabBar: () => null }));

import { initI18n } from '@/i18n';
import type { AttachmentViewTab, EmailTab, EmailThreadTab } from '@/stores/emailStore';
import { ReadingPane } from './ReadingPane';

const pdfTab: AttachmentViewTab = {
  type: 'attachment',
  id: 'att-1',
  accountId: 'acct-1',
  emailId: 'email-1',
  filename: 'report.pdf',
  mimeType: 'application/pdf',
  dataUrl: 'data:application/pdf;base64,JVBERi0=',
  isLoading: false,
};

const threadTab = {
  type: 'thread',
  id: 'thread-1',
  threadEmails: [],
  isLoading: false,
} as unknown as EmailThreadTab;

const tabs: EmailTab[] = [pdfTab, threadTab];

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
        tabs={tabs}
        activeTabId={activeTab?.id ?? null}
        activeTab={activeTab}
        accounts={[]}
        activeAccountId={null}
        onSelectMainTab={() => {}}
        onSelectTab={() => {}}
        onCloseTab={() => {}}
        onCloseMain={() => {}}
      />,
    ),
  );
}

function pdfFrame(): HTMLIFrameElement | null {
  return container.querySelector('iframe[title="report.pdf"]');
}

describe('ReadingPane attachment tabs', () => {
  it('keeps the same PDF viewer across a switch to another tab and back', () => {
    render(pdfTab);
    const frame = pdfFrame();
    expect(frame).not.toBeNull();

    render(threadTab);
    render(pdfTab);

    expect(pdfFrame()).toBe(frame);
  });

  it('hides an inactive attachment tab without collapsing it', () => {
    render(threadTab);

    const layer = pdfFrame()?.closest('[data-attachment-tab]') as HTMLElement | null;
    expect(layer).not.toBeNull();
    expect(layer?.className).toContain('invisible');
    expect(layer?.className).not.toContain('hidden');
    expect(container.querySelector('[data-testid="thread-view"]')).not.toBeNull();
  });

  it('shows the active attachment tab', () => {
    render(pdfTab);

    const layer = pdfFrame()?.closest('[data-attachment-tab]') as HTMLElement | null;
    expect(layer?.className).not.toContain('invisible');
    expect(container.querySelector('[data-testid="thread-view"]')).toBeNull();
  });
});
