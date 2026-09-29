// Split layout auto-selects the top email once, when a list first loads. It
// used to re-select whenever the selection became null — closing the reading
// pane or deleting/moving the open email immediately opened (and marked read)
// the next one.

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
});

const emails = [
  { id: 'e1', threadId: 't1', category: 'primary', isRead: false } as Email,
  { id: 'e2', threadId: 't2', category: 'primary', isRead: false } as Email,
];

async function render(
  props: { isLoading?: boolean; selectedEmailId?: string | null; list?: Email[] },
  onSelect: (email: Email, opts?: { auto?: boolean }) => void,
) {
  await act(async () => {
    root.render(
      <Inbox
        emails={props.list ?? emails}
        isLoading={props.isLoading ?? false}
        isSyncing={false}
        syncProgress={null}
        isLoadingMore={false}
        hasMore={false}
        totalCount={2}
        selectedEmailId={props.selectedEmailId ?? null}
        onSelectEmail={onSelect}
        onLoadMore={() => {}}
        selectedCategories={new Set<EmailCategory>()}
        onSelectCategories={() => {}}
      />,
    );
  });
}

describe('Inbox auto-select', () => {
  it('selects the top email when the list first loads, flagged as automatic', async () => {
    const onSelect = vi.fn();
    await render({}, onSelect);
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(emails[0], { auto: true });
  });

  it('does not re-select when the selection is cleared', async () => {
    const onSelect = vi.fn();
    await render({}, onSelect);
    await render({ selectedEmailId: 'e1' }, onSelect);
    await render({ selectedEmailId: null, list: [emails[1]] }, onSelect);
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('selects again once a new list has loaded', async () => {
    const onSelect = vi.fn();
    await render({}, onSelect);
    await render({ selectedEmailId: 'e1' }, onSelect);
    await render({ isLoading: true, list: [] }, onSelect);
    await render({ list: [emails[1]] }, onSelect);
    expect(onSelect).toHaveBeenCalledTimes(2);
    expect(onSelect).toHaveBeenLastCalledWith(emails[1], { auto: true });
  });
});
