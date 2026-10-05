// Pasting a block copied from Excel (tab-separated) into a cell fills the
// sheet from that cell; plain text stays an ordinary paste. Formula cells show
// their result, column widths are dragged into the document, and column
// filters hide rows from this view only.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }) }));

import { ensureGrid, readGrid, setCell } from '@/lib/sheetModel';
import { useToastStore } from '@/stores/toastStore';
import { SheetEditor } from './SheetEditor';

function paste(target: Element, text: string): boolean {
  const event = new Event('paste', { bubbles: true, cancelable: true }) as Event & {
    clipboardData: { getData: (type: string) => string };
  };
  event.clipboardData = { getData: (type) => (type === 'text/plain' ? text : '') };
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

describe('SheetEditor', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  async function mount(doc: Y.Doc, editable = true) {
    await act(async () => {
      root.render(<SheetEditor doc={doc} editable={editable} />);
    });
  }

  it('fills the cells from the one pasted into', async () => {
    const doc = new Y.Doc();
    await mount(doc);
    let handled = false;
    act(() => {
      handled = paste(container.querySelector('[data-cell="1:1"]') as Element, 'Item\tPrice\r\nDesk\t69,00 €\r\n');
    });
    expect(handled).toBe(true);
    const { values } = readGrid(doc);
    expect(values[1].slice(1, 3)).toEqual(['Item', 'Price']);
    expect(values[2].slice(1, 3)).toEqual(['Desk', '69,00 €']);
  });

  it('leaves plain text to the browser', async () => {
    const doc = new Y.Doc();
    await mount(doc);
    let handled = true;
    act(() => {
      handled = paste(container.querySelector('[data-cell="0:0"]') as Element, 'just text');
    });
    expect(handled).toBe(false);
  });

  function sheetWith(rows: string[][]): Y.Doc {
    const doc = new Y.Doc();
    ensureGrid(doc, 4, 2);
    const { rowIds, colIds } = readGrid(doc);
    rows.forEach((row, r) => {
      row.forEach((value, c) => {
        setCell(doc, rowIds[r], colIds[c], value);
      });
    });
    return doc;
  }

  const cell = (ref: string) => container.querySelector(`[data-cell="${ref}"]`) as HTMLInputElement;

  it('shows the result of a formula and the formula while focused', async () => {
    const doc = sheetWith([
      ['Item', 'Price'],
      ['Desk', '69'],
      ['Lamp', '31'],
      ['Total', '=SUM(B2:B3)'],
    ]);
    await mount(doc);
    expect(cell('3:1').value).toBe('100');
    act(() => cell('3:1').focus());
    expect(cell('3:1').value).toBe('=SUM(B2:B3)');
  });

  it('writes the dragged width of a column into the document', async () => {
    const doc = sheetWith([['Item', 'Price']]);
    await mount(doc);
    const handle = container.querySelector('[data-testid="sheet-resize-0"]') as Element;
    act(() => {
      handle.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 100 }));
    });
    act(() => {
      window.dispatchEvent(new MouseEvent('pointermove', { clientX: 160 }));
    });
    act(() => {
      window.dispatchEvent(new MouseEvent('pointerup', {}));
    });
    expect(readGrid(doc).widths[0]).toBe(188);
  });

  it('hides the rows whose value is unchecked in a column filter, keeping the header', async () => {
    const doc = sheetWith([
      ['Item', 'Room'],
      ['Desk', 'Office'],
      ['Lamp', 'Hall'],
      ['Chair', 'Office'],
    ]);
    await mount(doc);
    act(() => (container.querySelector('[data-testid="sheet-filter-1"]') as HTMLButtonElement).click());
    act(() => (container.querySelector('[data-testid="sheet-filter-value-Office"]') as HTMLInputElement).click());
    expect(cell('0:0')).not.toBeNull();
    expect(cell('1:0')).toBeNull();
    expect(cell('2:0').value).toBe('Lamp');
    expect(cell('3:0')).toBeNull();
    expect(readGrid(doc).values[1][0]).toBe('Desk');
  });

  it('shows a row added while a filter is on, though its cells do not match', async () => {
    const doc = sheetWith([
      ['Item', 'Room'],
      ['Desk', 'Office'],
      ['Lamp', 'Hall'],
      ['Chair', 'Office'],
    ]);
    await mount(doc);
    act(() => (container.querySelector('[data-testid="sheet-filter-1"]') as HTMLButtonElement).click());
    act(() => (container.querySelector('[data-testid="sheet-filter-value-Hall"]') as HTMLInputElement).click());
    act(() => (container.querySelector('[data-testid="sheet-add-row"]') as HTMLButtonElement).click());
    expect(readGrid(doc).rowIds).toHaveLength(5);
    expect(cell('4:0')).not.toBeNull();
  });

  it('inserts a row above another, and a sum over the rows takes it in', async () => {
    const doc = sheetWith([['1'], ['2'], ['=SUM(A1:A2)']]);
    await mount(doc);
    act(() => (container.querySelector('[data-testid="sheet-insert-row-1"]') as HTMLButtonElement).click());
    expect(
      readGrid(doc)
        .values.map((row) => row[0])
        .slice(0, 4),
    ).toEqual(['1', '', '2', '=SUM(A1:A3)']);
  });

  it('undoes and redoes this person’s edits from the toolbar and the keyboard', async () => {
    const doc = sheetWith([['Item']]);
    await mount(doc);
    const { rowIds, colIds } = readGrid(doc);
    act(() => setCell(doc, rowIds[0], colIds[1], 'Price'));
    act(() => (container.querySelector('[data-testid="sheet-undo"]') as HTMLButtonElement).click());
    expect(readGrid(doc).values[0][1]).toBe('');
    act(() => (container.querySelector('[data-testid="sheet-redo"]') as HTMLButtonElement).click());
    expect(readGrid(doc).values[0][1]).toBe('Price');
    act(() => {
      cell('0:1').dispatchEvent(new KeyboardEvent('keydown', { key: 'z', metaKey: true, bubbles: true }));
    });
    expect(readGrid(doc).values[0][1]).toBe('');
    act(() => {
      cell('0:1').dispatchEvent(
        new KeyboardEvent('keydown', { key: 'z', metaKey: true, shiftKey: true, bubbles: true }),
      );
    });
    expect(readGrid(doc).values[0][1]).toBe('Price');
  });

  it('never undoes changes that came from other people', async () => {
    const doc = sheetWith([['Item']]);
    await mount(doc);
    const peer = new Y.Doc();
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc));
    const { rowIds, colIds } = readGrid(peer);
    setCell(peer, rowIds[0], colIds[1], 'Price');
    act(() => Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer), 'shared-doc-remote'));
    act(() => (container.querySelector('[data-testid="sheet-undo"]') as HTMLButtonElement).click());
    expect(readGrid(doc).values[0][1]).toBe('Price');
  });

  it('flags a cell two people changed at once and brings the dropped value back on request', async () => {
    const base = sheetWith([
      ['Item', 'Cost'],
      ['Desk', '100'],
    ]);
    const doc = new Y.Doc({ gc: false });
    doc.clientID = 1;
    Y.applyUpdate(doc, Y.encodeStateAsUpdate(base));
    const other = new Y.Doc({ gc: false });
    other.clientID = 2;
    Y.applyUpdate(other, Y.encodeStateAsUpdate(base));
    const before = Y.encodeStateVector(other);
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[1], colIds[1], '120');
    setCell(other, rowIds[1], colIds[1], '130');
    await mount(doc);
    const toasts: string[] = [];
    useToastStore.setState({ toasts: [] });

    act(() => Y.applyUpdate(doc, Y.encodeStateAsUpdate(other, before), 'shared-doc-remote'));
    for (const t of useToastStore.getState().toasts) toasts.push(t.message);

    const kept = readGrid(doc).values[1][1];
    const lost = kept === '120' ? '130' : '120';
    expect(container.querySelector('[data-testid="sheet-conflicts"]')).not.toBeNull();
    expect(cell('1:1').getAttribute('data-conflict')).toBe('true');
    expect(toasts).toHaveLength(1);
    act(() => (container.querySelector('[data-testid="sheet-conflict-restore"]') as HTMLButtonElement).click());
    expect(readGrid(doc).values[1][1]).toBe(lost);
    expect(container.querySelector('[data-testid="sheet-conflicts"]')).toBeNull();
  });
});
