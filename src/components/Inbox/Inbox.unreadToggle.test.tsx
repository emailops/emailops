// The list header's All / Unread toggle: it drives the store's session flag in
// every list, and an empty unread list says so.

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
vi.mock('./VirtualEmailList', () => ({
  VirtualEmailList: ({ emptyStateMessage }: { emptyStateMessage: string }) => (
    <p data-testid="empty">{emptyStateMessage}</p>
  ),
}));
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

import { useEmailStore } from '@/stores/emailStore';
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
  useEmailStore.setState({ searchQuery: null, unreadOnly: false });
});

async function mount(searchQuery: string | null = null) {
  useEmailStore.setState({ searchQuery });
  await act(async () => {
    root.render(
      <Inbox
        emails={[]}
        isLoading={false}
        isSyncing={false}
        syncProgress={null}
        isLoadingMore={false}
        hasMore={false}
        selectedEmailId={null}
        onSelectEmail={() => {}}
        onLoadMore={() => {}}
        accountId="acct-1"
        selectedCategories={new Set<EmailCategory>()}
        onSelectCategories={() => {}}
        availableCategories={[]}
      />,
    );
  });
}

const toggle = () => container.querySelector('[role="radiogroup"][aria-label="inbox:readFilter.aria"]');
const option = (label: string) =>
  [...container.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find((b) => b.textContent === label);

describe('the list Unread toggle', () => {
  it('starts on All', async () => {
    await mount();
    expect(option('inbox:readFilter.all')?.getAttribute('aria-checked')).toBe('true');
    expect(option('inbox:readFilter.unread')?.getAttribute('aria-checked')).toBe('false');
  });

  it('turns the unread filter on and off', async () => {
    await mount();

    await act(async () => option('inbox:readFilter.unread')?.click());
    expect(useEmailStore.getState().unreadOnly).toBe(true);
    expect(option('inbox:readFilter.unread')?.getAttribute('aria-checked')).toBe('true');

    await act(async () => option('inbox:readFilter.all')?.click());
    expect(useEmailStore.getState().unreadOnly).toBe(false);
  });

  it('stays available on a sidebar filter', async () => {
    await mount('from:bob@acme.com');
    expect(toggle()).not.toBeNull();
  });

  it('stays while a free-text search is active, because it applies there too', async () => {
    await mount('invoice');
    expect(toggle()).not.toBeNull();
  });

  it('says there is nothing unread when the unread list is empty', async () => {
    useEmailStore.setState({ unreadOnly: true });
    await mount();
    expect(container.querySelector('[data-testid="empty"]')?.textContent).toBe('inbox:readFilter.empty');
  });
});
