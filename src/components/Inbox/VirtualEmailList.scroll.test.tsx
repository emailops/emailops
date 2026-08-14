// Regression cover for the inbox "blank band above the rows after Back" bug.
//
// In full-width layout, App.tsx keeps the inbox mounted and hides it with
// Tailwind `hidden` (display: none) while an email is open — the comment there
// says this preserves scroll position. It does the opposite: the browser resets
// scrollTop to 0 on display:none and fires NO scroll event. Since
// @tanstack/virtual-core learns its offset only from scroll events
// (`observeOffset` registers a listener and never reads scrollTop on attach),
// on Back the virtualizer still believed the pre-hide offset while the container
// sat at 0 — so it rendered the row window for a position the container wasn't
// at, leaving a blank band above the rows until the first real scroll.
//
// Restoring the saved scrollTop covers that, but only as far as the scroll event
// the write happens to fire — and it fires none when the container already holds
// that value, or when the browser clamps the write against content that has not
// caught up yet. Two further things are needed, and are what these tests pin
// down: the virtualizer has to be forced to re-read the container when the two
// disagree, and a row that measures zero because it is hidden must not be
// recorded as a zero-height row.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Email } from '@/types';
import { VirtualEmailList } from './VirtualEmailList';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

// jsdom ships no ResizeObserver. The virtualizer constructs its own alongside
// the component's, so the stub must hand callbacks a realistic `entries` array —
// virtual-core reads entries[0] and would throw on a bare `cb()`.
const observers = new Set<StubResizeObserver>();

/** Row height the browser reports. 0 models a `display: none` ancestor: every
 *  box in the subtree collapses, rows included — which is what poisons the
 *  virtualizer's size cache in the real bug. */
const ROW_HEIGHT = 48;
let reportedRowHeight = ROW_HEIGHT;

class StubResizeObserver {
  private readonly targets = new Set<Element>();

  constructor(private readonly cb: (entries: unknown[], observer: unknown) => void) {
    observers.add(this);
  }

  observe(target: Element) {
    this.targets.add(target);
  }

  unobserve(target: Element) {
    this.targets.delete(target);
  }

  disconnect() {
    this.targets.clear();
    observers.delete(this);
  }

  trigger() {
    const entries = Array.from(this.targets, (target) => {
      const isRow = (target as HTMLElement).hasAttribute('data-index');
      const height = isRow ? reportedRowHeight : ((target as HTMLElement).clientHeight ?? 0);
      const width = (target as HTMLElement).clientWidth ?? 0;
      const box = [{ inlineSize: width, blockSize: height }];
      return {
        target,
        contentRect: { width, height, top: 0, left: 0, bottom: height, right: width, x: 0, y: 0 },
        borderBoxSize: box,
        contentBoxSize: box,
        devicePixelContentBoxSize: box,
      };
    });
    this.cb(entries, this);
  }
}

function fireResize() {
  act(() => {
    for (const observer of Array.from(observers)) observer.trigger();
  });
}

const VIEWPORT_HEIGHT = 800;

/** jsdom hard-codes clientHeight to 0; override it to model visibility. */
function setVisible(el: HTMLElement, visible: boolean) {
  Object.defineProperty(el, 'clientHeight', { value: visible ? VIEWPORT_HEIGHT : 0, configurable: true });
  reportedRowHeight = visible ? ROW_HEIGHT : 0;
}

/** Show the list and let the rows settle on their measured height. The first
 *  observation only registers them (the virtualizer has no scroll element to
 *  hang its observer on until after the first commit), the second measures. */
function showAndSettle(el: HTMLElement) {
  setVisible(el, true);
  fireResize();
  fireResize();
}

/** Model what the browser does to scrollTop when an ancestor is display:none. */
function browserHides(el: HTMLElement) {
  setVisible(el, false);
  el.scrollTop = 0; // silent — no scroll event, exactly as the browser does it
}

function renderedRowTops(): number[] {
  return Array.from(container.querySelectorAll<HTMLElement>('[data-index]'), (row) => {
    const match = /translateY\((-?[\d.]+)px\)/.exec(row.style.transform);
    return match ? Number(match[1]) : Number.NaN;
  }).sort((a, b) => a - b);
}

/**
 * The user-visible invariant: the rows the virtualizer chose to render must
 * cover the part of the list the container is actually showing. A blank band is
 * exactly the case where the topmost rendered row starts *below* the viewport's
 * top edge.
 */
function expectNoBlankBand(el: HTMLElement) {
  const tops = renderedRowTops();
  expect(tops.length, 'some rows must be rendered').toBeGreaterThan(0);
  expect(tops[0], 'topmost rendered row must start at or above the viewport top').toBeLessThanOrEqual(el.scrollTop);
}

/** Complementary to expectNoBlankBand, for a list long enough to fill the viewport. */
function expectRowsReachTheBottom(el: HTMLElement) {
  const tops = renderedRowTops();
  expect(
    tops[tops.length - 1] + ROW_HEIGHT,
    'bottom-most rendered row must reach the viewport bottom',
  ).toBeGreaterThanOrEqual(el.scrollTop + VIEWPORT_HEIGHT);
}

let container: HTMLDivElement;
let root: Root;
let scrollContainerRef: { current: HTMLDivElement | null };

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', StubResizeObserver);
  observers.clear();
  reportedRowHeight = ROW_HEIGHT;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  scrollContainerRef = { current: null };
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function email(id: string): Email {
  return {
    id,
    accountId: 'acct-1',
    threadId: `thread-${id}`,
    messageId: `<${id}@example.test>`,
    subject: `Subject ${id}`,
    sender: 'Sender Name',
    senderEmail: 'sender@example.test',
    recipients: ['me@example.test'],
    cc: [],
    body: 'body',
    snippet: 'snippet',
    timestamp: 1_770_000_000,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
  };
}

function renderWith(emails: Email[]) {
  act(() => {
    root.render(
      <VirtualEmailList
        emails={emails}
        selectedEmailId={null}
        focusEmailId={null}
        scrollContainerRef={scrollContainerRef}
        isLoadingMore={false}
        hasMore={false}
        isSyncing={false}
        onSelectEmail={() => {}}
        onLoadMore={() => {}}
      />,
    );
  });
}

const manyEmails = Array.from({ length: 40 }, (_, i) => email(`e${i}`));

describe('VirtualEmailList scroll position across a display:none hide', () => {
  it('restores the scroll position when the list becomes visible again', () => {
    renderWith(manyEmails);
    const el = scrollContainerRef.current;
    expect(el).not.toBeNull();
    if (!el) return;

    // Visible and scrolled down — the scroll listener records the position.
    setVisible(el, true);
    el.scrollTop = 640;
    act(() => {
      el.dispatchEvent(new Event('scroll'));
    });

    // Email opened: the wrapper goes display:none, the browser silently zeroes
    // scrollTop, and the ResizeObserver reports the collapsed box.
    browserHides(el);
    fireResize();
    expect(el.scrollTop, 'the browser has zeroed it at this point').toBe(0);

    // Back: the wrapper is visible again.
    setVisible(el, true);
    fireResize();

    expect(el.scrollTop, 'the saved position must be written back, which also resyncs the virtualizer').toBe(640);
  });

  it('does not invent a scroll position when the list was never scrolled', () => {
    renderWith(manyEmails);
    const el = scrollContainerRef.current;
    if (!el) return;

    setVisible(el, true);
    fireResize();

    browserHides(el);
    fireResize();

    setVisible(el, true);
    fireResize();

    expect(el.scrollTop).toBe(0);
  });

  it('honours a scroll back to the top before hiding', () => {
    renderWith(manyEmails);
    const el = scrollContainerRef.current;
    if (!el) return;

    setVisible(el, true);
    el.scrollTop = 640;
    act(() => el.dispatchEvent(new Event('scroll')));
    // User scrolls back up before opening the email.
    el.scrollTop = 0;
    act(() => el.dispatchEvent(new Event('scroll')));

    browserHides(el);
    fireResize();
    setVisible(el, true);
    fireResize();

    expect(el.scrollTop, 'must not resurrect the earlier 640 offset').toBe(0);
  });

  // The restored scrollTop is only half the job. @tanstack/virtual-core learns
  // the offset from scroll events alone, so unless something makes it re-read
  // the container it keeps rendering the window for a position the container is
  // not at — the blank band the user sees. Assert on the rendered rows, not on
  // scrollTop: the previous fix left this failing while its own test was green.
  it('renders the rows the container is actually showing after a hide/show cycle', () => {
    renderWith(manyEmails);
    const el = scrollContainerRef.current;
    if (!el) return;

    showAndSettle(el);
    el.scrollTop = 640;
    act(() => {
      el.dispatchEvent(new Event('scroll'));
    });
    expectNoBlankBand(el);
    expectRowsReachTheBottom(el);

    browserHides(el);
    fireResize();

    showAndSettle(el);

    expectNoBlankBand(el);
    expectRowsReachTheBottom(el);
  });

  // While the wrapper is display:none every row reports a zero-height box.
  // virtual-core has no guard for that: it writes 0 into its size cache for the
  // whole live window, which collapses getTotalSize() and makes it adjust its
  // own offset with no DOM counterpart. A hidden element measuring 0 says
  // nothing about the row, only about the container.
  it('does not let a hidden measurement shrink the list', () => {
    renderWith(manyEmails);
    const el = scrollContainerRef.current;
    if (!el) return;

    showAndSettle(el);
    const spacer = () => container.querySelector<HTMLElement>('.overflow-y-auto > div');
    const before = spacer()?.style.height;
    expect(before, 'the rows have been measured by now').not.toBe('0px');

    browserHides(el);
    fireResize();

    expect(spacer()?.style.height, 'the list height must survive being hidden').toBe(before);
  });

  // Supporting invariant: the scroll container is a single stable DOM node, so
  // the effect attached on mount keeps observing the right element even as the
  // list flips between its empty and populated branches.
  it('keeps one stable scroll container across empty <-> populated', () => {
    renderWith([]);
    const beforeNode = scrollContainerRef.current;
    expect(beforeNode).not.toBeNull();
    expect(container.querySelectorAll('.overflow-y-auto').length).toBe(1);

    renderWith(manyEmails);

    expect(scrollContainerRef.current).toBe(beforeNode);
    expect(container.querySelectorAll('.overflow-y-auto').length).toBe(1);
  });
});
