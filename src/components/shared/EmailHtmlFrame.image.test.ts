// Tapping an image in a message opens it full-screen. The body is a
// null-origin sandboxed iframe, so the tap is only observable inside the frame
// — the bridge script reports it to the parent, which owns the viewer. These
// tests exercise the real BRIDGE_SCRIPT under jsdom and assert on what it
// posts, the same way the pinch-zoom and search tests do.

import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { BRIDGE_SCRIPT } from './EmailHtmlFrame';

function runBridge() {
  new Function(BRIDGE_SCRIPT)();
}

interface FrameMessage {
  __emailFrame?: boolean;
  type?: string;
  src?: string;
  alt?: string;
  href?: string;
}

function posted(spy: { mock: { calls: unknown[][] } }, type: string): FrameMessage[] {
  return spy.mock.calls
    .map((call) => call[0] as FrameMessage)
    .filter((msg) => msg?.__emailFrame === true && msg.type === type);
}

/** jsdom never decodes an image, so `naturalWidth` stays 0 and the bridge falls
 *  back to the presentational size — which the width/height attributes set. */
function click(selector: string, body: string) {
  document.body.innerHTML = body;
  const target = document.querySelector(selector);
  if (!target) throw new Error(`no element matched ${selector}`);
  const spy = vi.spyOn(window, 'postMessage');
  target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
  return spy;
}

const PIXEL = 'data:image/gif;base64,R0lGODlhAQABAAAAACw=';

describe('EmailHtmlFrame bridge image taps', () => {
  // The bridge installs document-level listeners it never removes; run it once.
  beforeAll(() => {
    runBridge();
  });

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('reports the source and alt text of a tapped image', () => {
    const spy = click('img', `<img src="${PIXEL}" alt="Q3 chart" width="600" height="400">`);
    expect(posted(spy, 'image')).toEqual([{ __emailFrame: true, type: 'image', src: PIXEL, alt: 'Q3 chart' }]);
  });

  it('reports an image with no alt text as an empty caption', () => {
    const spy = click('img', `<img src="${PIXEL}" width="600" height="400">`);
    expect(posted(spy, 'image')[0]?.alt).toBe('');
  });

  it('reports a tap that lands on an element wrapping the image', () => {
    const spy = click('figure', `<figure><img src="${PIXEL}" width="600" height="400"></figure>`);
    // The tap target is the <figure>; only a tap that resolves to an image counts.
    expect(posted(spy, 'image')).toHaveLength(0);
  });

  // Newsletters wrap their hero image in the offer link. Tapping it means "go
  // to the offer" in every other mail client, so the link keeps winning and the
  // viewer stays out of the way.
  it('lets a surrounding link win over the viewer', () => {
    const spy = click('img', `<a href="https://example.com/offer"><img src="${PIXEL}" width="600" height="400"></a>`);
    expect(posted(spy, 'link')).toHaveLength(1);
    expect(posted(spy, 'image')).toHaveLength(0);
  });

  it('opens the viewer for an image inside an anchor with no destination', () => {
    const spy = click('img', `<a><img src="${PIXEL}" width="600" height="400"></a>`);
    expect(posted(spy, 'image')).toHaveLength(1);
  });

  // Tracking pixels and the 1px images newsletters use as rules and spacers are
  // scattered through real mail; opening a full-screen viewer on one is never
  // what the tap meant.
  it('ignores tracking pixels and spacer images', () => {
    expect(posted(click('img', `<img src="${PIXEL}" width="1" height="1">`), 'image')).toHaveLength(0);
    expect(posted(click('img', `<img src="${PIXEL}" width="600" height="1">`), 'image')).toHaveLength(0);
    expect(posted(click('img', `<img src="${PIXEL}" width="20" height="20">`), 'image')).toHaveLength(0);
  });

  it('ignores an image that never loaded and has no size at all', () => {
    expect(posted(click('img', `<img src="${PIXEL}">`), 'image')).toHaveLength(0);
  });

  it('ignores an image with no source', () => {
    expect(posted(click('img', '<img alt="broken" width="600" height="400">'), 'image')).toHaveLength(0);
  });

  it('says nothing when the tap is on ordinary text', () => {
    const spy = click('p', '<p>Just a paragraph.</p>');
    expect(posted(spy, 'image')).toHaveLength(0);
    expect(posted(spy, 'link')).toHaveLength(0);
  });
});
