// The list publishes the rows it shows so the root key handler (j/k, x, *a)
// walks exactly what the user sees, and passes the keyboard cursor down.

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
}));
const listProps = vi.fn();
vi.mock('./VirtualEmailList', () => ({
  VirtualEmailList: (props: Record<string, unknown>) => {
    listProps(props);
    return null;
  },
}));
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email, EmailCategory } from '@/types';
import { Inbox } from './Inbox';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  listProps.mockClear();
  useShortcutStore.setState({ listEmails: [], cursorId: null });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  container.remove();
});

const emails = [
  { id: 'e1', threadId: 't1', category: 'primary', isRead: false } as Email,
  { id: 'e2', threadId: 't2', category: 'promotions', isRead: false } as Email,
];

async function render(fullWidth: boolean, onSelect = vi.fn()) {
  await act(async () => {
    root.render(
      <Inbox
        emails={emails}
        isLoading={false}
        isSyncing={false}
        syncProgress={null}
        isLoadingMore={false}
        hasMore={false}
        totalCount={2}
        selectedEmailId={null}
        onSelectEmail={onSelect}
        onLoadMore={() => {}}
        selectedCategories={new Set<EmailCategory>(['primary'])}
        onSelectCategories={() => {}}
        disableAutoSelect
        fullWidth={fullWidth}
      />,
    );
  });
  return onSelect;
}

const lastListProps = () => listProps.mock.calls[listProps.mock.calls.length - 1][0];

describe('Inbox and the keyboard', () => {
  it('publishes the visible rows (after the category filter) and clears them on unmount', async () => {
    await render(true);
    expect(useShortcutStore.getState().listEmails.map((e) => e.id)).toEqual(['e1']);
    act(() => root.unmount());
    expect(useShortcutStore.getState().listEmails).toEqual([]);
  });

  it('shows the cursor in the full-width list only', async () => {
    useShortcutStore.setState({ cursorId: 'e1' });
    await render(true);
    expect(lastListProps().cursorEmailId).toBe('e1');
    expect(lastListProps().showCursor).toBe(true);
    await render(false);
    expect(lastListProps().showCursor).toBe(false);
    act(() => root.unmount());
  });

  it('opening a row with the mouse moves the cursor there', async () => {
    const onSelect = await render(true);
    const select = lastListProps().onSelectEmail as (e: Email) => void;
    act(() => select(emails[0]));
    expect(useShortcutStore.getState().cursorId).toBe('e1');
    expect(onSelect).toHaveBeenCalledWith(emails[0], undefined);
    act(() => root.unmount());
  });
});
