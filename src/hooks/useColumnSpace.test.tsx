// The resizable columns are fitted to the space the sidebar, the list and the
// reading area share; it shrinks when the right dock opens or the window
// narrows, and the hook has to follow it.

import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useColumnSpace } from './useColumnSpace';

type Callback = () => void;
let observers: Callback[] = [];

class FakeResizeObserver {
  constructor(private readonly cb: Callback) {
    observers.push(cb);
  }
  observe() {}
  disconnect() {
    observers = observers.filter((o) => o !== this.cb);
  }
}

// jsdom lays nothing out: each test places the row and the main area.
function place(el: Element, left: number, right: number) {
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue({ left, right } as DOMRect);
}

function Layout() {
  const mainRef = useRef<HTMLElement>(null);
  const space = useColumnSpace(mainRef);
  return (
    <div data-testid="row">
      <aside />
      <main ref={mainRef} />
      <output>{space === null ? 'unmeasured' : space}</output>
    </div>
  );
}

let container: HTMLDivElement;
let root: Root;
const shown = () => container.querySelector('output')?.textContent;
const row = () => container.querySelector('[data-testid="row"]') as HTMLElement;
const main = () => container.querySelector('main') as HTMLElement;

beforeEach(() => {
  observers = [];
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('useColumnSpace', () => {
  it('is unknown until the main area is measured', () => {
    act(() => root.render(<Layout />));
    expect(shown()).toBe('unmeasured');
  });

  it('measures from the row start to the end of the main area, and follows it', () => {
    act(() => root.render(<Layout />));
    place(row(), 0, 1200);
    place(main(), 256, 1200);
    act(() => observers[0]());
    expect(shown()).toBe('1200');
    // The right dock opens beside the main area.
    place(main(), 256, 820);
    act(() => observers[0]());
    expect(shown()).toBe('820');
  });

  it('stops observing on unmount', () => {
    act(() => root.render(<Layout />));
    act(() => root.unmount());
    expect(observers).toHaveLength(0);
  });
});
