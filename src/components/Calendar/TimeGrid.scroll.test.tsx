// Regression test: the initial scroll-to-7:00 must be applied AFTER the first
// paint (via requestAnimationFrame), not synchronously during the mount
// commit. Setting scrollTop while WKWebView is still compositing the freshly
// mounted scroll container can leave hit-testing misaligned with the painted
// content — events LOOK right but clicks land ~7 hours off, so "clicking does
// nothing" until the view is remounted. Deferring the scroll past first paint
// keeps pixels and hit-testing in agreement.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TimeGrid } from './TimeGrid';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '00:00', date: () => '', dateTime: () => '' }),
}));

let container: HTMLDivElement;
let root: Root;
let rafQueue: FrameRequestCallback[];

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  rafQueue = [];
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    rafQueue.push(cb);
    return rafQueue.length;
  });
  vi.stubGlobal('cancelAnimationFrame', () => {});
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function flushAnimationFrames() {
  // Run queued frames, including frames queued by the frames themselves.
  for (let i = 0; i < 5 && rafQueue.length > 0; i++) {
    const batch = [...rafQueue];
    rafQueue.length = 0;
    for (const cb of batch) cb(0);
  }
}

describe('TimeGrid initial scroll', () => {
  const day = new Date(2026, 6, 27); // local midnight anchor

  function mount() {
    act(() => {
      root.render(<TimeGrid days={[day]} events={[]} onSelectEvent={() => {}} onCreateSlot={() => {}} />);
    });
  }

  function scrollContainer(): HTMLElement {
    const el = container.querySelector('.overflow-y-auto');
    if (!(el instanceof HTMLElement)) throw new Error('scroll container not rendered');
    return el;
  }

  it('does NOT scroll synchronously during the mount commit', () => {
    mount();
    expect(scrollContainer().scrollTop).toBe(0);
  });

  it('scrolls to 07:00 once the post-paint animation frames run', () => {
    mount();
    flushAnimationFrames();
    expect(scrollContainer().scrollTop).toBe(7 * 48);
  });
});
