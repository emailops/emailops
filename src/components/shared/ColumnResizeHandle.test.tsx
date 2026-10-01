// The column splitter: drag with the mouse, arrows/Home/End from the
// keyboard, double-click to restore the default width.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { KEY_STEP, LIST_COLUMN } from '@/lib/columnResize';
import { ColumnResizeHandle } from './ColumnResizeHandle';

let container: HTMLDivElement;
let root: Root;
let onResize: ReturnType<typeof vi.fn<(width: number) => void>>;

function render(width = 400) {
  act(() => {
    root.render(
      <ColumnResizeHandle spec={LIST_COLUMN} width={width} onResize={onResize} label="Resize list" title="hint" />,
    );
  });
  const handle = container.querySelector<HTMLElement>('[role="separator"]');
  if (!handle) throw new Error('handle not rendered');
  return handle;
}

function pointer(el: HTMLElement, type: string, clientX: number) {
  const e = new MouseEvent(type, { bubbles: true, clientX, button: 0 }) as MouseEvent & { pointerId: number };
  Object.defineProperty(e, 'pointerId', { value: 1 });
  act(() => {
    el.dispatchEvent(e);
  });
}

beforeEach(() => {
  onResize = vi.fn();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.style.userSelect = '';
});

describe('ColumnResizeHandle', () => {
  it('is a focusable vertical separator exposing its width', () => {
    const handle = render(400);
    expect(handle.getAttribute('aria-orientation')).toBe('vertical');
    expect(handle.getAttribute('aria-label')).toBe('Resize list');
    expect(handle.getAttribute('aria-valuenow')).toBe('400');
    expect(handle.getAttribute('aria-valuemin')).toBe(String(LIST_COLUMN.min));
    expect(handle.tabIndex).toBe(0);
  });

  it('follows the pointer while dragging, then stops', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 100);
    expect(document.body.style.userSelect).toBe('none');
    pointer(handle, 'pointermove', 160);
    expect(onResize).toHaveBeenLastCalledWith(460);
    pointer(handle, 'pointerup', 160);
    expect(document.body.style.userSelect).toBe('');
    onResize.mockClear();
    pointer(handle, 'pointermove', 300);
    expect(onResize).not.toHaveBeenCalled();
  });

  it('never goes past the limits while dragging', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 500);
    pointer(handle, 'pointermove', -2000);
    expect(onResize).toHaveBeenLastCalledWith(LIST_COLUMN.min);
  });

  it('resizes from the keyboard', () => {
    const handle = render(400);
    act(() => {
      handle.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    });
    expect(onResize).toHaveBeenLastCalledWith(400 + KEY_STEP);
    act(() => {
      handle.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    });
    expect(onResize).toHaveBeenLastCalledWith(LIST_COLUMN.min);
  });

  it('double-click restores the default width', () => {
    const handle = render(700);
    act(() => {
      handle.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    });
    expect(onResize).toHaveBeenLastCalledWith(LIST_COLUMN.defaultWidth);
  });
});
