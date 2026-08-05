// The sidebar drawer stays mounted while closed so it keeps its expanded
// account groups and filter list. That also kept its scroll offset, so
// re-opening the drawer on a phone showed it parked mid-list. These tests pin
// the reset-on-token-change behaviour that fixes it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useScrollReset } from './useScrollReset';

let container: HTMLDivElement;
let root: Root;
/** Every value written to the scroller's `scrollTop`, in order. */
let writes: number[];

function Harness({ token }: { token: unknown }) {
  const ref = useScrollReset<HTMLDivElement>(token);
  return <div ref={ref} data-testid="scroller" />;
}

/**
 * jsdom performs no layout, so a real `scrollTop` round-trip always reads 0 and
 * would make the assertions vacuous. Intercept the property instead and record
 * what the hook writes.
 */
function instrumentScroller(): void {
  const node = container.querySelector('[data-testid="scroller"]') as HTMLDivElement;
  let value = 0;
  Object.defineProperty(node, 'scrollTop', {
    configurable: true,
    get: () => value,
    set: (next: number) => {
      value = next;
      writes.push(next);
    },
  });
}

function render(token: unknown): void {
  act(() => {
    root.render(<Harness token={token} />);
  });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  writes = [];
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('useScrollReset', () => {
  it('scrolls back to the top when the token changes', () => {
    render(1);
    instrumentScroller();
    writes = [];

    render(2);

    expect(writes).toEqual([0]);
  });

  it('leaves the scroll position alone while the token is unchanged', () => {
    // Re-renders happen constantly (a sync tick, an unread count). Resetting on
    // every one of them would yank the list from under a user mid-scroll.
    render(1);
    instrumentScroller();
    writes = [];

    render(1);
    render(1);

    expect(writes).toEqual([]);
  });

  it('resets across a run of distinct tokens, not only the first change', () => {
    render(1);
    instrumentScroller();
    writes = [];

    render(2);
    render(3);

    expect(writes).toEqual([0, 0]);
  });
});
