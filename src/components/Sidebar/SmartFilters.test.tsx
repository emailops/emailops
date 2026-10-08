// Smart filters: a click replaces the search with the filter; a right-click
// offers adding it to the current search instead.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import type { ActiveFilter, SmartFilter } from '@/types';
import { SmartFilters } from './SmartFilters';

let container: HTMLDivElement;
let root: Root;
const onToggleFilter = vi.fn();
const onAppendFilter = vi.fn();

const urgent: SmartFilter = { type: 'priority', value: 'urgent', count: 3 };

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  onToggleFilter.mockReset();
  onAppendFilter.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root.render(
      <SmartFilters
        filters={[urgent]}
        activeFilter={null}
        isLoading={false}
        onToggleFilter={onToggleFilter}
        onAppendFilter={onAppendFilter}
        onClearFilter={() => {}}
        onPinFilter={() => {}}
        onUnpinFilter={() => {}}
        onRemoveFilter={() => {}}
        onRefresh={() => {}}
        isPinned={(_: ActiveFilter) => false}
      />,
    );
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function filterRow(): HTMLButtonElement {
  const row = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes('urgent'));
  if (!row) throw new Error('filter row not rendered');
  return row;
}

function menuItem(): HTMLButtonElement | undefined {
  return Array.from(document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')).find((b) =>
    b.textContent?.includes('Add to search'),
  );
}

describe('SmartFilters', () => {
  it('a click hands the filter over to replace the search', () => {
    act(() => filterRow().click());
    expect(onToggleFilter).toHaveBeenCalledWith(urgent);
    expect(onAppendFilter).not.toHaveBeenCalled();
  });

  it('a right-click offers adding the filter to the current search', () => {
    expect(menuItem()).toBeUndefined();

    act(() => {
      filterRow().dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10 }));
    });
    const item = menuItem();
    expect(item).toBeDefined();

    act(() => item?.click());
    expect(onAppendFilter).toHaveBeenCalledWith({ type: 'priority', value: 'urgent' });
    expect(onToggleFilter).not.toHaveBeenCalled();
    expect(menuItem()).toBeUndefined();
  });

  it('Escape closes the menu without changing the search', () => {
    act(() => {
      filterRow().dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
    });
    act(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(menuItem()).toBeUndefined();
    expect(onAppendFilter).not.toHaveBeenCalled();
  });
});
