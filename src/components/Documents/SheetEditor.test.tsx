// Pasting a block copied from Excel (tab-separated) into a cell fills the
// sheet from that cell; plain text stays an ordinary paste.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

import { readGrid } from '@/lib/sheetModel';
import { SheetEditor } from './SheetEditor';

function paste(target: Element, text: string): boolean {
  const event = new Event('paste', { bubbles: true, cancelable: true }) as Event & {
    clipboardData: { getData: (type: string) => string };
  };
  event.clipboardData = { getData: (type) => (type === 'text/plain' ? text : '') };
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

describe('SheetEditor paste', () => {
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
});
