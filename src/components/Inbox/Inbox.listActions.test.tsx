// What the list offers when it has nothing to show or stopped loading: every
// dead end comes with the action that gets out of it, in the user's language.

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
  getEmails: vi.fn(async () => []),
}));
vi.mock('./VirtualEmailList', () => ({
  VirtualEmailList: ({
    emptyStateMessage,
    emptyStateAction,
  }: {
    emptyStateMessage: string;
    emptyStateAction?: { label: string; onClick: () => void };
  }) => (
    <div>
      <p data-testid="empty">{emptyStateMessage}</p>
      {emptyStateAction && (
        <button type="button" data-testid="empty-action" onClick={emptyStateAction.onClick}>
          {emptyStateAction.label}
        </button>
      )}
    </div>
  ),
}));
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

import * as api from '@/lib/api';
import { useEmailStore } from '@/stores/emailStore';
import type { Email, EmailCategory } from '@/types';
import { Inbox } from './Inbox';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useEmailStore.getState().reset();
  useEmailStore.setState({ unreadOnly: false });
});

async function mount(props: Partial<Parameters<typeof Inbox>[0]> = {}) {
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
        accountId="acct-1"
        selectedCategories={new Set<EmailCategory>()}
        onSelectCategories={() => {}}
        availableCategories={[]}
        {...props}
      />,
    );
  });
}

const emptyMessage = () => container.querySelector('[data-testid="empty"]')?.textContent;
const emptyAction = () => container.querySelector<HTMLButtonElement>('[data-testid="empty-action"]');

describe('the Load more button after a failed page', () => {
  it('asks the backend again', async () => {
    const first = { id: 'first', accountId: 'acct-1', threadId: 't1' } as Email;
    useEmailStore.setState({ emails: [first], paging: { status: 'idle', cursor: null, offset: 1 }, totalCount: 500 });
    vi.mocked(api.getEmails).mockRejectedValueOnce(new Error('offline'));
    const loadMore = () => void useEmailStore.getState().loadMoreEmails('acct-1', null, [], 'inbox');
    await act(async () => loadMore());
    await mount({ emails: [first], hasMore: true, onLoadMore: loadMore });
    vi.mocked(api.getEmails).mockClear();

    const button = [...container.querySelectorAll('button')].find((b) => b.textContent === 'inbox:loadMore');
    await act(async () => button?.click());

    expect(api.getEmails).toHaveBeenCalledTimes(1);
  });
});

describe('the empty list', () => {
  it('offers to leave the Unread filter', async () => {
    useEmailStore.setState({ unreadOnly: true });
    await mount();
    expect(emptyMessage()).toBe('inbox:readFilter.empty');

    await act(async () => emptyAction()?.click());

    expect(useEmailStore.getState().unreadOnly).toBe(false);
  });

  it('offers every category when the selected one has no mail', async () => {
    const onSelectCategories = vi.fn();
    await mount({
      availableCategories: ['primary', 'social'],
      selectedCategories: new Set<EmailCategory>(['social']),
      onSelectCategories,
    });
    expect(emptyMessage()).toBe('inbox:emptyState.noFilterResults');

    await act(async () => emptyAction()?.click());

    expect(onSelectCategories).toHaveBeenCalledWith(new Set(['primary', 'social']));
  });

  it('offers a sync when the account has no mail yet', async () => {
    const onSync = vi.fn();
    await mount({ onSync });
    expect(emptyMessage()).toBe('inbox:emptyState.noEmails');

    await act(async () => emptyAction()?.click());

    expect(onSync).toHaveBeenCalledTimes(1);
  });

  it('offers nothing while a sync is filling it, nor next to the search notice', async () => {
    const onSync = vi.fn();
    await mount({ onSync, isSyncing: true });
    expect(emptyMessage()).toBe('inbox:emptyState.syncing');
    expect(emptyAction()).toBeNull();

    useEmailStore.setState({ searchQuery: 'invoice' });
    await mount({ onSync });
    expect(emptyMessage()).toBe('inbox:emptyState.noSearchResults');
    expect(emptyAction()).toBeNull();
  });
});

describe('the category tabs', () => {
  it('are labelled through i18n', async () => {
    await mount({ availableCategories: ['primary', 'promotions'] });
    const tabs = [...container.querySelectorAll('[role="tab"]')].map((t) => t.textContent);
    expect(tabs).toEqual(['inbox:allCategories', 'inbox:categoryFilter.primary', 'inbox:categoryFilter.promotions']);
  });
});
