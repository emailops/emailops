// The full-width list's column header: Sender, Subject, Date and Size sort the
// whole list, and the column picker shows or hides Size and Has attachment.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { DEFAULT_LIST_COLUMNS, DEFAULT_LIST_SORT, type ListColumns, type ListSort } from '@/lib/listColumns';
import { ListColumnHeader } from './ListColumnHeader';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(
  opts: {
    columns?: ListColumns;
    sort?: ListSort;
    sortable?: boolean;
    inSentView?: boolean;
    onSort?: (key: string) => void;
    onColumnsChange?: (columns: ListColumns) => void;
  } = {},
) {
  act(() => {
    root.render(
      <ListColumnHeader
        columns={opts.columns ?? DEFAULT_LIST_COLUMNS}
        onColumnsChange={opts.onColumnsChange ?? (() => {})}
        sort={opts.sort ?? DEFAULT_LIST_SORT}
        onSort={opts.onSort ?? (() => {})}
        sortable={opts.sortable ?? true}
        inSentView={opts.inSentView ?? false}
      />,
    );
  });
}

const sortButton = (key: string) =>
  container.querySelector<HTMLButtonElement>(`button[data-testid="list-sort-${key}"]`);
const click = (el: Element | null) =>
  act(() => {
    el?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });

describe('ListColumnHeader', () => {
  it('offers Sender, Subject and Date, and Size only when its column is shown', () => {
    render();
    expect(sortButton('sender')?.textContent).toContain('Sender');
    expect(sortButton('subject')?.textContent).toContain('Subject');
    expect(sortButton('date')?.textContent).toContain('Date');
    expect(sortButton('size')).toBeNull();

    render({ columns: { size: true, attachment: true } });
    expect(sortButton('size')?.textContent).toContain('Size');
    expect(container.querySelector('[data-testid="list-column-attachment"]')?.getAttribute('aria-label')).toBe(
      'Has attachment',
    );
  });

  it('puts Has attachment before Size, in the header and in the column picker', () => {
    render({ columns: { size: true, attachment: true } });
    const attachment = container.querySelector('[data-testid="list-column-attachment"]');
    const size = sortButton('size');
    expect(
      attachment && size && attachment.compareDocumentPosition(size) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    click(container.querySelector('button[aria-label="Columns"]'));
    const toggles = [...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].map(
      (input) => input.closest('label')?.textContent,
    );
    expect(toggles).toEqual(['Has attachment', 'Size']);
  });

  it('sorts by the clicked column', () => {
    const onSort = vi.fn();
    render({ columns: { size: true, attachment: false }, onSort });

    click(sortButton('size'));

    expect(onSort).toHaveBeenCalledWith('size');
  });

  it('says which column the list is sorted by, and which way', () => {
    render({ sort: { key: 'subject', dir: 'asc' } });
    expect(sortButton('subject')?.getAttribute('aria-label')).toBe('Subject, sorted ascending');
    expect(sortButton('date')?.getAttribute('aria-label')).toBe('Sort by Date');

    render({ sort: DEFAULT_LIST_SORT });
    expect(sortButton('date')?.getAttribute('aria-label')).toBe('Date, sorted descending');
  });

  it('only labels the columns while the list cannot be sorted', () => {
    render({ sortable: false });
    expect(container.querySelectorAll('button[data-testid^="list-sort-"]')).toHaveLength(0);
    expect(container.textContent).toContain('Subject');
  });

  it('names the recipients column in the Sent view, which does not sort by sender', () => {
    render({ inSentView: true });
    expect(sortButton('sender')).toBeNull();
    expect(container.textContent).toContain('To');
  });

  it('shows or hides an optional column from the column picker', () => {
    const onColumnsChange = vi.fn();
    render({ columns: { size: false, attachment: true }, onColumnsChange });

    click(container.querySelector('button[aria-label="Columns"]'));
    const sizeToggle = [...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(
      (input) => input.closest('label')?.textContent === 'Size',
    );
    expect(sizeToggle?.checked).toBe(false);
    click(sizeToggle ?? null);

    expect(onColumnsChange).toHaveBeenCalledWith({ size: true, attachment: true });
  });
});
