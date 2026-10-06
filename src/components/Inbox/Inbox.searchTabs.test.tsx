// A search spans every category, so while one is active the tab strip must
// say so: "All" reads as selected and the category tabs are disabled. Leaving
// the user's tab highlighted suggested the search was scoped to it, and
// clicking another tab silently changed the selection without changing the
// results.

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
  useEmailStore.setState({ searchQuery: null });
});

async function mount(searchQuery: string | null, onSelectCategories = vi.fn()) {
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
        totalCount={0}
        selectedEmailId={null}
        onSelectEmail={() => {}}
        onLoadMore={() => {}}
        accountId="acct-1"
        selectedCategories={new Set<EmailCategory>(['primary'])}
        onSelectCategories={onSelectCategories}
        availableCategories={['primary', 'promotions', 'updates']}
      />,
    );
  });
  return onSelectCategories;
}

const tabs = () => [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
const selectedTabs = () => tabs().filter((t) => t.getAttribute('aria-selected') === 'true');

describe('Inbox category tabs during a search', () => {
  it('keeps the chosen tab selected when no search is active', async () => {
    await mount(null);
    expect(selectedTabs().map((t) => t.textContent)).toEqual(['Primary']);
  });

  it('shows "All" as selected while a search is active', async () => {
    await mount('invoice');
    expect(selectedTabs().map((t) => t.textContent)).toEqual(['inbox:allCategories']);
  });

  it('disables the tabs and explains why while a search is active', async () => {
    await mount('invoice');
    expect(tabs().every((t) => t.disabled)).toBe(true);
    expect(container.querySelector('[role="tablist"]')?.getAttribute('title')).toBe('inbox:searchAllCategories');
  });

  it('does not change the category selection when a tab is clicked during a search', async () => {
    const onSelectCategories = await mount('invoice');
    await act(async () => tabs()[2].click());
    expect(onSelectCategories).not.toHaveBeenCalled();
  });
});
