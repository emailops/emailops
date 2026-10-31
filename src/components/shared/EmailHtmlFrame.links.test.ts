// Every link in an email must go through the parent's confirmation, so the
// bridge has to stop the frame navigating on its own for each kind of link.
// We exercise the real BRIDGE_SCRIPT under jsdom.

import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { BRIDGE_SCRIPT } from './EmailHtmlFrame';

function clickFirst(selector: string): MouseEvent {
  const event = new MouseEvent('click', { bubbles: true, cancelable: true });
  document.querySelector(selector)?.dispatchEvent(event);
  return event;
}

function linkMessages(calls: unknown[][]): (string | undefined)[] {
  return calls
    .map((call) => call[0] as { type?: string; href?: string })
    .filter((msg) => msg?.type === 'link')
    .map((msg) => msg.href);
}

describe('EmailHtmlFrame bridge links', () => {
  beforeAll(() => {
    // The bridge's listeners live on the document, so it only runs once.
    new Function(BRIDGE_SCRIPT)();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = '';
  });

  it.each([
    ['an anchor', '<a href="https://example.com/a">x</a>', 'a'],
    ['an image-map area', '<map name="m"><area href="https://example.com/a"></map>', 'area'],
  ])('hands %s to the parent instead of navigating', (_name, html, selector) => {
    document.body.innerHTML = html;
    const spy = vi.spyOn(window, 'postMessage');

    const event = clickFirst(selector);

    expect(event.defaultPrevented).toBe(true);
    expect(linkMessages(spy.mock.calls)).toEqual(['https://example.com/a']);
  });

  it('hands an SVG xlink:href link to the parent instead of navigating', () => {
    document.body.innerHTML = '<svg><a><text>x</text></a></svg>';
    document.querySelector('a')?.setAttributeNS('http://www.w3.org/1999/xlink', 'xlink:href', 'https://example.com/a');
    const spy = vi.spyOn(window, 'postMessage');

    const event = clickFirst('text');

    expect(event.defaultPrevented).toBe(true);
    expect(linkMessages(spy.mock.calls)).toEqual(['https://example.com/a']);
  });
});
