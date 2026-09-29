// The "Manage Rules" badge counts pending suggested rules and must say so to
// screen readers, not only show a bare number.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts ? `${key}${JSON.stringify(opts)}` : key),
  }),
}));
vi.mock('@/lib/api', () => ({}));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: vi.fn() }),
}));
vi.mock('@/stores/toastStore', () => ({
  useToastStore: (selector: (s: { addToast: () => void }) => unknown) => selector({ addToast: vi.fn() }),
}));

import { AttachmentToolbar } from './AttachmentToolbar';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

function render(suggestionCount: number) {
  act(() => {
    root.render(
      <AttachmentToolbar
        accountId="acc-1"
        totalCount={0}
        selectedTag={null}
        availableTags={[]}
        checkedCount={0}
        allChecked={false}
        onSetSelectedTag={vi.fn()}
        onToggleCheckAll={vi.fn()}
        onClearChecked={vi.fn()}
        checkedIds={new Set()}
        onOpenRules={vi.fn()}
        suggestionCount={suggestionCount}
      />,
    );
  });
}

beforeEach(() => {
  container = document.createElement('div');
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
});

describe('AttachmentToolbar suggestion badge', () => {
  it('shows the count and names what it counts for screen readers', () => {
    render(3);

    expect(container.querySelector('[aria-hidden="true"]')?.textContent).toBe('3');
    expect(container.querySelector('.sr-only')?.textContent).toBe('attachments:suggestions.badgeTitle{"count":3}');
  });

  it('is absent when there is nothing to review', () => {
    render(0);

    expect(container.textContent).not.toContain('attachments:suggestions.badgeTitle');
  });
});
