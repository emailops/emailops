// The full-width list carries the column header: a click on a header sorts
// the list, and the optional columns are a saved preference. The split layout
// has no header, so it lists newest first.

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
  getPref: vi.fn(async () => '{"size":true,"attachment":false}'),
  setPref: vi.fn(async () => {}),
}));
vi.mock('./VirtualEmailList', () => ({
  VirtualEmailList: ({
    columns,
    scrollContainerRef,
  }: {
    columns?: unknown;
    scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  }) => (
    <div ref={scrollContainerRef} data-testid="scroller">
      <p data-testid="columns">{JSON.stringify(columns)}</p>
    </div>
  ),
}));
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

import { DEFAULT_LIST_SORT } from '@/lib/listColumns';
import { useEmailStore } from '@/stores/emailStore';
import type { Email, EmailCategory } from '@/types';
import { Inbox } from './Inbox';

const row = { id: 'e1', accountId: 'acct-1', threadId: 't1', category: 'primary', timestamp: 1 } as Email;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useEmailStore.setState({ listSort: DEFAULT_LIST_SORT, listScope: 'inbox', searchQuery: null });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(fullWidth: boolean) {
  await act(async () => {
    root.render(
      <Inbox
        emails={[row]}
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
        fullWidth={fullWidth}
      />,
    );
  });
}

describe('the full-width list column header', () => {
  it('sorts the list by the clicked column', async () => {
    await mount(true);

    await act(async () => {
      container
        .querySelector('[data-testid="list-sort-subject"]')
        ?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(useEmailStore.getState().listSort).toEqual({ key: 'subject', dir: 'asc' });
  });

  it('shows the top of the list in its new order', async () => {
    await mount(true);
    const scroller = container.querySelector<HTMLDivElement>('[data-testid="scroller"]');
    if (!scroller) throw new Error('list not rendered');
    scroller.scrollTop = 900;

    await act(async () => {
      container
        .querySelector('[data-testid="list-sort-subject"]')
        ?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(scroller.scrollTop).toBe(0);
  });

  it('hands the saved optional columns to the rows', async () => {
    await mount(true);
    expect(container.querySelector('[data-testid="columns"]')?.textContent).toBe('{"size":true,"attachment":false}');
  });

  it('is not shown in the split layout, which lists newest first', async () => {
    useEmailStore.setState({ listSort: { key: 'size', dir: 'desc' } });

    await mount(false);

    expect(container.querySelector('[data-testid^="list-sort-"]')).toBeNull();
    expect(useEmailStore.getState().listSort).toEqual(DEFAULT_LIST_SORT);
  });
});
