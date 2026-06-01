// Guards against the v0.5.0 auto-height ratchet: the iframe body grew without
// bound (measured 23,622px for content that was really ~20,654px). The cause was
// measuring `Math.max(documentElement.scrollHeight, body.scrollHeight)`. The
// root element's scrollHeight floors to the viewport height, and the viewport
// equals the height the parent just set on the iframe — so each measurement
// could only ratchet upward. The fix measures the body's content height, which
// shrink-wraps its content and never floors to the viewport.
//
// We exercise the real BRIDGE_SCRIPT under jsdom: stub the two scrollHeights to
// differ, run the script, and assert it posts the body height (not the larger
// documentElement height).

import { afterEach, describe, expect, it, vi } from 'vitest';
import { BRIDGE_SCRIPT, FRAME_BASE_CSS } from './EmailHtmlFrame';

function runBridge() {
  // BRIDGE_SCRIPT is a self-invoking IIFE; new Function runs it in global scope
  // so its `window`/`document`/`parent` references resolve to the jsdom globals.
  new Function(BRIDGE_SCRIPT)();
}

interface FrameMessage {
  __emailFrame?: boolean;
  type?: string;
  height?: number;
}

function heightMessages(calls: unknown[][]): number[] {
  return calls
    .map((call) => call[0] as FrameMessage)
    .filter((msg) => msg && msg.__emailFrame && msg.type === 'height' && typeof msg.height === 'number')
    .map((msg) => msg.height as number);
}

describe('EmailHtmlFrame bridge auto-height', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    Reflect.deleteProperty(document.documentElement, 'scrollHeight');
    Reflect.deleteProperty(document.body, 'scrollHeight');
  });

  it('posts the body content height, ignoring the viewport-floored documentElement height', () => {
    // documentElement.scrollHeight floors to the viewport (the height we set);
    // body.scrollHeight is the true content height.
    Object.defineProperty(document.documentElement, 'scrollHeight', { configurable: true, value: 23622 });
    Object.defineProperty(document.body, 'scrollHeight', { configurable: true, value: 20654 });

    const spy = vi.spyOn(window, 'postMessage');
    runBridge();

    const heights = heightMessages(spy.mock.calls);
    expect(heights.length).toBeGreaterThan(0);
    for (const h of heights) {
      expect(h).toBe(20654);
    }
  });
});

describe('EmailHtmlFrame margin containment', () => {
  // Regression: a short email ending in a footer (e.g. `<p>body</p>...<hr><p>Sent
  // with EmailOps</p>`) rendered with its footer clipped. The body has no
  // padding/border, so the leading <p>'s top margin collapsed *through* the body
  // and escaped above it — shifting content down ~16px without being counted in
  // document.body.scrollHeight. Auto-height then under-measured and the iframe
  // clipped the trailing footer. A block formatting context on the body contains
  // those margins so scrollHeight reflects the true content height. jsdom can't
  // exercise layout, so we lock the CSS that prevents the collapse.
  it('the frame body establishes a block formatting context to contain child margins', () => {
    expect(FRAME_BASE_CSS).toMatch(/display:\s*flow-root/);
  });
});
