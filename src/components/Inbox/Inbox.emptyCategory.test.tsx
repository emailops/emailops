// An empty category tab shows an empty list. Right after a Gmail account was
// added its first sync is still running, and an empty Updates tab used to
// fall back to the unfiltered list: the user saw Primary mail under a
// highlighted Updates tab.

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
  VirtualEmailList: ({
    emails,
    emptyStateMessage,
    emptyStateAction,
  }: {
    emails: { id: string }[];
    emptyStateMessage: string;
    emptyStateAction?: { label: string; onClick: () => void };
  }) => (
    <div>
      <ul>
        {emails.map((e) => (
          <li key={e.id} data-testid="row">
            {e.id}
          </li>
        ))}
      </ul>
      {emails.length === 0 && <p data-testid="empty">{emptyStateMessage}</p>}
      {emails.length === 0 && emptyStateAction && <span data-testid="empty-action">{emptyStateAction.label}</span>}
    </div>
  ),
}));
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

import { useEmailStore } from '@/stores/emailStore';
import type { Email, EmailCategory } from '@/types';
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
  useEmailStore.getState().reset();
});

const email = (id: string, category: EmailCategory) => ({ id, accountId: 'acct-1', threadId: id, category }) as Email;

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
        selectedEmailId={null}
        onSelectEmail={() => {}}
        onLoadMore={() => {}}
        accountId="acct-1"
        selectedCategories={new Set<EmailCategory>(['updates'])}
        onSelectCategories={() => {}}
        availableCategories={['primary', 'social', 'updates']}
        disableAutoSelect
        {...props}
      />,
    );
  });
}

const rows = () => [...container.querySelectorAll('[data-testid="row"]')].map((r) => r.textContent);
const emptyMessage = () => container.querySelector('[data-testid="empty"]')?.textContent;
const emptyAction = () => container.querySelector('[data-testid="empty-action"]')?.textContent;
const selectedTabs = () =>
  [...container.querySelectorAll('[role="tab"]')]
    .filter((t) => t.getAttribute('aria-selected') === 'true')
    .map((t) => t.textContent);

const primaryOnly = [email('p1', 'primary'), email('p2', 'primary')];

describe('an empty category tab', () => {
  it('shows no rows from another category while a sync is running', async () => {
    await mount({ emails: primaryOnly, isSyncing: true });

    expect(rows()).toEqual([]);
    expect(emptyMessage()).toBe('inbox:emptyState.syncing');
    expect(selectedTabs()).toEqual(['inbox:categoryFilter.updates']);
  });

  it('says nothing matches and offers every category once the sync is over', async () => {
    await mount({ emails: primaryOnly });

    expect(rows()).toEqual([]);
    expect(emptyMessage()).toBe('inbox:emptyState.noFilterResults');
    expect(emptyAction()).toBe('inbox:emptyState.showAllCategories');
  });
});

describe('a category tab with mail', () => {
  it('shows only the rows of the highlighted category while a sync is running', async () => {
    await mount({
      emails: [email('p1', 'primary'), email('u1', 'updates'), email('s1', 'social')],
      isSyncing: true,
    });

    expect(rows()).toEqual(['u1']);
  });
});

describe('the auto-expand to All after the first sync', () => {
  it('selects every category when the default one has no mail', async () => {
    const onSelectCategories = vi.fn();
    await mount({
      emails: [email('u1', 'updates')],
      selectedCategories: new Set<EmailCategory>(['primary']),
      onSelectCategories,
    });

    expect(onSelectCategories).toHaveBeenCalledWith(new Set(['primary', 'social', 'updates']));
  });

  it('waits for the sync to end', async () => {
    const onSelectCategories = vi.fn();
    await mount({
      emails: [email('u1', 'updates')],
      selectedCategories: new Set<EmailCategory>(['primary']),
      onSelectCategories,
      isSyncing: true,
    });

    expect(onSelectCategories).not.toHaveBeenCalled();
  });
});
