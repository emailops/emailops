// The column splitter: drag with the mouse, arrows/Home/End from the
// keyboard, double-click to restore the default width. A drag moves the
// column live and saves its width once, on release.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { KEY_STEP, LIST_COLUMN } from '@/lib/columnResize';
import { ColumnResizeHandle } from './ColumnResizeHandle';

let container: HTMLDivElement;
let root: Root;
let onCommit: ReturnType<typeof vi.fn<(width: number) => void>>;

// As in App, the handle sits right after the column it resizes.
function render(width = 400) {
  act(() => {
    root.render(
      <>
        <div data-testid="column" style={{ width }} />
        <ColumnResizeHandle spec={LIST_COLUMN} width={width} onCommit={onCommit} label="Resize list" title="hint" />
      </>,
    );
  });
  const handle = container.querySelector<HTMLElement>('[role="separator"]');
  if (!handle) throw new Error('handle not rendered');
  return handle;
}

function column(): HTMLElement {
  const el = container.querySelector<HTMLElement>('[data-testid="column"]');
  if (!el) throw new Error('column not rendered');
  return el;
}

const captureLayer = () => document.querySelector('[data-testid="column-resize-drag-capture"]');

// jsdom has no PointerEvent; a MouseEvent with the pointer type name reaches
// React's onPointerDown and the window listeners the same way.
function pointer(target: EventTarget, type: string, clientX: number) {
  act(() => {
    target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX, button: 0 }));
  });
}

beforeEach(() => {
  onCommit = vi.fn();
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

  it('moves the column with the pointer and saves the width once, on release', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 100);
    expect(document.body.style.userSelect).toBe('none');
    for (let x = 110; x <= 300; x += 10) pointer(window, 'pointermove', x);
    expect(column().style.width).toBe('600px');
    expect(handle.getAttribute('aria-valuenow')).toBe('600');
    expect(onCommit).not.toHaveBeenCalled();

    pointer(window, 'pointerup', 300);
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith(600);
    expect(document.body.style.userSelect).toBe('');
  });

  it('stops following the pointer after the release', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 100);
    pointer(window, 'pointermove', 160);
    pointer(window, 'pointerup', 160);
    pointer(window, 'pointermove', 300);
    expect(column().style.width).toBe('460px');
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('never goes past the limits while dragging', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 500);
    pointer(window, 'pointermove', -2000);
    pointer(window, 'pointerup', -2000);
    expect(onCommit).toHaveBeenLastCalledWith(LIST_COLUMN.min);
  });

  // Over an email iframe the window gets no pointer events, and a drag that
  // crossed it froze; the layer keeps the pointer in this document.
  it('covers the window with a capture layer while the drag moves', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 100);
    pointer(window, 'pointermove', 150);
    expect(captureLayer()).not.toBeNull();
    pointer(window, 'pointerup', 150);
    expect(captureLayer()).toBeNull();
  });

  // A layer that appeared on press would take the release, and the clicks of
  // a double-click would never reach the handle.
  it('treats a press without movement as a click: no capture layer, nothing saved', () => {
    const handle = render(400);
    pointer(handle, 'pointerdown', 100);
    pointer(window, 'pointermove', 101);
    expect(captureLayer()).toBeNull();
    pointer(window, 'pointerup', 101);
    expect(onCommit).not.toHaveBeenCalled();
  });

  it('resizes from the keyboard, saving each step', () => {
    const handle = render(400);
    act(() => {
      handle.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    });
    expect(onCommit).toHaveBeenLastCalledWith(400 + KEY_STEP);
    act(() => {
      handle.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    });
    expect(onCommit).toHaveBeenLastCalledWith(LIST_COLUMN.min);
  });

  it('double-click restores the default width', () => {
    const handle = render(700);
    act(() => {
      handle.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    });
    expect(onCommit).toHaveBeenLastCalledWith(LIST_COLUMN.defaultWidth);
  });
});
