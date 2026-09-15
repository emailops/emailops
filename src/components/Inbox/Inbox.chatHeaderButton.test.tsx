// The inbox header carries one chat button, not two: while the docked panel is
// closed it reads "Open chat panel" and re-docks it; while the panel is open it
// is the usual "New chat". A second "Open chat panel" icon next to "New chat"
// was indistinguishable from it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/lib/api', () => ({
  getEmailTagsBatch: vi.fn(async () => ({})),
  getJunkConfig: vi.fn(async () => null),
  getJunkVerdicts: vi.fn(async () => []),
  setJunkConfig: vi.fn(async () => {}),
  setJunkFeedback: vi.fn(async () => {}),
}));
vi.mock('./VirtualEmailList', () => ({ VirtualEmailList: () => null }));
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

import type { EmailCategory } from '@/types';
import { Inbox } from './Inbox';

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
});

async function mount(props: { onNewChat?: () => void; isChatPanelOpen?: boolean }) {
  await act(async () => {
    root.render(
      <Inbox
        emails={[]}
        isLoading={false}
        isSyncing={false}
        syncProgress={null}
        isLoadingMore={false}
        hasMore={false}
        totalCount={0}
        selectedEmailId={null}
        onSelectEmail={() => {}}
        onLoadMore={() => {}}
        selectedCategories={new Set<EmailCategory>()}
        onSelectCategories={() => {}}
        {...props}
      />,
    );
  });
}

const chatButtons = () =>
  [...container.querySelectorAll('button')].filter((b) =>
    ['chat:panel.open', 'chat:panel.newChat'].includes(b.getAttribute('aria-label') ?? ''),
  );

describe('Inbox header chat button', () => {
  it('re-docks the panel while it is closed, under the "Open chat panel" label', async () => {
    const onNewChat = vi.fn();
    await mount({ onNewChat, isChatPanelOpen: false });
    const buttons = chatButtons();
    expect(buttons.map((b) => b.getAttribute('aria-label'))).toEqual(['chat:panel.open']);
    await act(async () => buttons[0].click());
    expect(onNewChat).toHaveBeenCalledTimes(1);
  });

  it('is the single "New chat" button while the panel is open', async () => {
    await mount({ onNewChat: () => {}, isChatPanelOpen: true });
    expect(chatButtons().map((b) => b.getAttribute('aria-label'))).toEqual(['chat:panel.newChat']);
  });

  it('renders no chat button when AI is off', async () => {
    await mount({});
    expect(chatButtons()).toHaveLength(0);
  });
});
