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
// The fix restores the saved scrollTop when the container becomes visible, which
// both returns the user to where they were and fires the scroll event that
// resyncs the virtualizer.

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
      const height = (target as HTMLElement).clientHeight ?? 0;
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

/** jsdom hard-codes clientHeight to 0; override it to model visibility. */
function setVisible(el: HTMLElement, visible: boolean) {
  Object.defineProperty(el, 'clientHeight', { value: visible ? 800 : 0, configurable: true });
}

/** Model what the browser does to scrollTop when an ancestor is display:none. */
function browserHides(el: HTMLElement) {
  setVisible(el, false);
  el.scrollTop = 0; // silent — no scroll event, exactly as the browser does it
}

let container: HTMLDivElement;
let root: Root;
let scrollContainerRef: { current: HTMLDivElement | null };

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', StubResizeObserver);
  observers.clear();
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
