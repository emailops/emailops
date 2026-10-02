// The Output bar publishes its height as a CSS variable so the toast stack
// can sit above it, whether the panel is collapsed or expanded.

import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { usePublishedHeight } from './usePublishedHeight';

type Callback = (entries: { borderBoxSize?: { blockSize: number }[] }[]) => void;
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

function Panel() {
  const ref = useRef<HTMLDivElement>(null);
  usePublishedHeight(ref, '--test-panel-height');
  return <div ref={ref} />;
}

let container: HTMLDivElement;
let root: Root;
const property = () => document.documentElement.style.getPropertyValue('--test-panel-height');

beforeEach(() => {
  observers = [];
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  container.remove();
  vi.unstubAllGlobals();
});

describe('usePublishedHeight', () => {
  it('publishes the element height and follows it as it changes', () => {
    act(() => root.render(<Panel />));
    act(() => observers[0]([{ borderBoxSize: [{ blockSize: 31 }] }]));
    expect(property()).toBe('31px');
    // Expanding the panel.
    act(() => observers[0]([{ borderBoxSize: [{ blockSize: 207.4 }] }]));
    expect(property()).toBe('207px');
  });

  it('removes the variable on unmount', () => {
    act(() => root.render(<Panel />));
    act(() => observers[0]([{ borderBoxSize: [{ blockSize: 31 }] }]));
    act(() => root.unmount());
    expect(property()).toBe('');
    expect(observers).toHaveLength(0);
  });
});
